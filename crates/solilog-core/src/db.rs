//! SQLite persistence, and the domain operations built on it.
//!
//! The database lives at `~/Library/Application Support/Solilog/solilog.db`
//! (platform equivalent elsewhere) unless `SOLILOG_DB` points to another file.
//!
//! The database is the single source of truth. Several processes (the GUI,
//! any number of CLI invocations) may use it concurrently; WAL mode and a busy
//! timeout make that safe, and [`Db::data_version`] lets long-lived processes
//! notice changes made by others.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, Result};
use rusqlite::{Connection, OptionalExtension as _, Row, ToSql, TransactionBehavior, params};

use crate::activity::{ActivityKind, Actor};
use crate::attachment::attachments_dir_for;
use crate::checklist::normalize_item_text;
use crate::filter::IssueFilter;
use crate::issue::{
    Issue, IssueChange, IssueEdit, IssueId, NewIssue, Status, normalize_description,
    normalize_title,
};
use crate::label::normalize_label_name;
use crate::order::{Ordered, Placement};
use crate::revision::Field;

/// Schema migrations, applied in order. `PRAGMA user_version` tracks how many ran.
/// Never edit an existing entry; append a new one.
pub(crate) const MIGRATIONS: &[&str] = &[
    // 1: issues
    "CREATE TABLE issues (
        id         INTEGER PRIMARY KEY,
        title      TEXT    NOT NULL,
        created_at INTEGER NOT NULL DEFAULT (unixepoch())
    );",
    // 2: status
    "ALTER TABLE issues ADD COLUMN status TEXT NOT NULL DEFAULT 'todo'
        CHECK (status IN ('backlog', 'todo', 'in_progress', 'done', 'blocked', 'cancelled'));",
    // 3: labels. `name` is the full path (`acme/reverb`); every ancestor has its own row.
    "CREATE TABLE labels (
        id         INTEGER PRIMARY KEY,
        name       TEXT    NOT NULL UNIQUE COLLATE NOCASE,
        created_at INTEGER NOT NULL DEFAULT (unixepoch())
    );
    CREATE TABLE issue_labels (
        issue_id INTEGER NOT NULL REFERENCES issues (id) ON DELETE CASCADE,
        label_id INTEGER NOT NULL REFERENCES labels (id) ON DELETE CASCADE,
        PRIMARY KEY (issue_id, label_id)
    ) WITHOUT ROWID;
    CREATE INDEX issue_labels_by_label ON issue_labels (label_id);",
    // 4: projects, their default labels, and issue membership.
    "CREATE TABLE projects (
        id          INTEGER PRIMARY KEY,
        name        TEXT    NOT NULL UNIQUE COLLATE NOCASE,
        status      TEXT    NOT NULL DEFAULT 'active'
            CHECK (status IN ('planned', 'active', 'done', 'archived')),
        description TEXT    NOT NULL DEFAULT '',
        target_date TEXT,
        created_at  INTEGER NOT NULL DEFAULT (unixepoch())
    );
    CREATE TABLE project_labels (
        project_id INTEGER NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
        label_id   INTEGER NOT NULL REFERENCES labels (id) ON DELETE CASCADE,
        PRIMARY KEY (project_id, label_id)
    ) WITHOUT ROWID;
    CREATE TABLE issue_projects (
        issue_id   INTEGER NOT NULL REFERENCES issues (id) ON DELETE CASCADE,
        project_id INTEGER NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
        PRIMARY KEY (issue_id, project_id)
    ) WITHOUT ROWID;
    CREATE INDEX issue_projects_by_project ON issue_projects (project_id);",
    // 5: views. `filter` is a JSON-serialized `IssueFilter`.
    "CREATE TABLE views (
        id         INTEGER PRIMARY KEY,
        name       TEXT    NOT NULL UNIQUE COLLATE NOCASE,
        filter     TEXT    NOT NULL,
        created_at INTEGER NOT NULL DEFAULT (unixepoch())
    );",
    // 6: projects no longer have a target date.
    "ALTER TABLE projects DROP COLUMN target_date;",
    // 7: projects are listed in a user-defined order: `position` ascending.
    // Existing projects start out active, then planned, then closed, by name.
    "ALTER TABLE projects ADD COLUMN position INTEGER NOT NULL DEFAULT 0;
    UPDATE projects SET position = ranked.position
    FROM (
        SELECT id, row_number() OVER (ORDER BY
            CASE status WHEN 'active' THEN 0 WHEN 'planned' THEN 1 WHEN 'done' THEN 2 ELSE 3 END,
            name COLLATE NOCASE) AS position
        FROM projects
    ) AS ranked
    WHERE ranked.id = projects.id;",
    // 8: labels are listed in a user-defined order: as a tree, siblings by
    // `position` ascending. Existing labels start out by name.
    "ALTER TABLE labels ADD COLUMN position INTEGER NOT NULL DEFAULT 0;
    UPDATE labels SET position = ranked.position
    FROM (
        SELECT id, row_number() OVER (ORDER BY name COLLATE NOCASE) AS position FROM labels
    ) AS ranked
    WHERE ranked.id = labels.id;",
    // 9: archived issues are out of the way: hidden from listings and counts.
    "ALTER TABLE issues ADD COLUMN archived INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1));",
    // 10: labels can have a color (a `LabelColor` key).
    "ALTER TABLE labels ADD COLUMN color TEXT;",
    // 11: issues have a plain-text description (empty when there is none).
    "ALTER TABLE issues ADD COLUMN description TEXT NOT NULL DEFAULT '';",
    // 12: checklists: ordered items (`position` ascending within an issue).
    "CREATE TABLE checklist_items (
        id         INTEGER PRIMARY KEY,
        issue_id   INTEGER NOT NULL REFERENCES issues (id) ON DELETE CASCADE,
        text       TEXT    NOT NULL,
        done       INTEGER NOT NULL DEFAULT 0 CHECK (done IN (0, 1)),
        position   INTEGER NOT NULL,
        created_at INTEGER NOT NULL DEFAULT (unixepoch())
    );
    CREATE INDEX checklist_items_by_issue ON checklist_items (issue_id, position);",
    // 13: issues are listed in a user-defined order: `position` ascending
    // (within a status group in the GUI). Existing issues start out newest first.
    "ALTER TABLE issues ADD COLUMN position INTEGER NOT NULL DEFAULT 0;
    UPDATE issues SET position = ranked.position
    FROM (SELECT id, row_number() OVER (ORDER BY id DESC) AS position FROM issues) AS ranked
    WHERE ranked.id = issues.id;",
    // 14: attachments: files in `<attachments dir>/SOL-<issue>/<name>`.
    "CREATE TABLE attachments (
        id         INTEGER PRIMARY KEY,
        issue_id   INTEGER NOT NULL REFERENCES issues (id) ON DELETE CASCADE,
        name       TEXT    NOT NULL,
        size       INTEGER NOT NULL,
        created_at INTEGER NOT NULL DEFAULT (unixepoch()),
        UNIQUE (issue_id, name COLLATE NOCASE)
    );",
    // 15: settings stored with the data (see `backup.rs`), and an id that
    // tells this database's backups from others in the same directory.
    "CREATE TABLE settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    ) WITHOUT ROWID;
    INSERT INTO settings (key, value) VALUES ('database_id', lower(hex(randomblob(4))));",
    // 16: a label's sublabels can be collapsed (hidden) in the GUI sidebar.
    "ALTER TABLE labels ADD COLUMN collapsed INTEGER NOT NULL DEFAULT 0 CHECK (collapsed IN (0, 1));",
    // 17: revisions: earlier text of titles, descriptions and checklist
    // items, to restore it (see `revision.rs`). A checklist item's revisions
    // outlive it (no reference); `position` is where a deleted item was.
    // Checklist item ids are no longer reused (AUTOINCREMENT), so that a
    // restored item gets its id back and revisions never mix two items.
    "CREATE TABLE revisions (
        id                INTEGER PRIMARY KEY,
        issue_id          INTEGER NOT NULL REFERENCES issues (id) ON DELETE CASCADE,
        field             TEXT    NOT NULL
            CHECK (field IN ('title', 'description', 'checklist_item')),
        checklist_item_id INTEGER
            CHECK ((field = 'checklist_item') = (checklist_item_id IS NOT NULL)),
        text              TEXT    NOT NULL,
        reason            TEXT    NOT NULL CHECK (reason IN ('edit', 'delete', 'restore')),
        position          INTEGER,
        created_at        INTEGER NOT NULL DEFAULT (unixepoch())
    );
    CREATE INDEX revisions_by_field ON revisions (issue_id, field, checklist_item_id, created_at);
    CREATE TABLE checklist_items_new (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        issue_id   INTEGER NOT NULL REFERENCES issues (id) ON DELETE CASCADE,
        text       TEXT    NOT NULL,
        done       INTEGER NOT NULL DEFAULT 0 CHECK (done IN (0, 1)),
        position   INTEGER NOT NULL,
        created_at INTEGER NOT NULL DEFAULT (unixepoch())
    );
    INSERT INTO checklist_items_new (id, issue_id, text, done, position, created_at)
        SELECT id, issue_id, text, done, position, created_at FROM checklist_items;
    DROP TABLE checklist_items;
    ALTER TABLE checklist_items_new RENAME TO checklist_items;
    CREATE INDEX checklist_items_by_issue ON checklist_items (issue_id, position);",
    // 18: dependencies between issues (see `dependency.rs`).
    "CREATE TABLE issue_dependencies (
        issue_id      INTEGER NOT NULL REFERENCES issues (id) ON DELETE CASCADE,
        depends_on_id INTEGER NOT NULL REFERENCES issues (id) ON DELETE CASCADE,
        PRIMARY KEY (issue_id, depends_on_id),
        CHECK (issue_id <> depends_on_id)
    ) WITHOUT ROWID;
    CREATE INDEX issue_dependencies_by_target ON issue_dependencies (depends_on_id);",
    // 19: authorship on issues, checklist items and revisions, a lightweight
    // activity log, and per-issue review tracking (see `activity.rs`).
    "ALTER TABLE issues ADD COLUMN created_by TEXT NOT NULL DEFAULT 'user';
    ALTER TABLE issues ADD COLUMN title_by TEXT NOT NULL DEFAULT 'user';
    ALTER TABLE issues ADD COLUMN description_by TEXT NOT NULL DEFAULT 'user';
    ALTER TABLE issues ADD COLUMN updated_by TEXT NOT NULL DEFAULT 'user';
    ALTER TABLE issues ADD COLUMN updated_at INTEGER NOT NULL DEFAULT 0;
    UPDATE issues SET updated_at = created_at;
    ALTER TABLE issues ADD COLUMN reviewed_at INTEGER;
    ALTER TABLE issues ADD COLUMN reviewed_activity_id INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE checklist_items ADD COLUMN author TEXT NOT NULL DEFAULT 'user';
    ALTER TABLE revisions ADD COLUMN author TEXT NOT NULL DEFAULT 'user';
    CREATE TABLE activities (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        issue_id   INTEGER NOT NULL REFERENCES issues (id) ON DELETE CASCADE,
        actor      TEXT    NOT NULL,
        kind       TEXT    NOT NULL,
        detail     TEXT    NOT NULL DEFAULT '',
        created_at INTEGER NOT NULL DEFAULT (unixepoch())
    );
    CREATE INDEX activities_by_issue ON activities (issue_id, id);",
];

/// The schema version of a database with every migration applied.
pub(crate) const SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64;

/// The first schema version that can be backed up: it has the `settings`
/// table (migration 15), which holds the database id and backup directory.
const FIRST_BACKUP_SCHEMA: i64 = 15;

fn ensure_supported(version: i64, latest: i64) -> Result<()> {
    anyhow::ensure!(
        version <= latest,
        "database schema version {version} is newer than this build supports ({latest}); update solilog"
    );
    Ok(())
}

/// Label names per issue.
const ISSUE_LABELS_SQL: &str =
    "SELECT il.issue_id, l.name FROM issue_labels il JOIN labels l ON l.id = il.label_id
     WHERE ?1 IS NULL OR il.issue_id = ?1
     ORDER BY l.name COLLATE NOCASE";

/// Project names per issue.
const ISSUE_PROJECTS_SQL: &str =
    "SELECT ip.issue_id, p.name FROM issue_projects ip JOIN projects p ON p.id = ip.project_id
     WHERE ?1 IS NULL OR ip.issue_id = ?1
     ORDER BY p.name COLLATE NOCASE";

const ISSUE_COLUMNS: &str = "id, title, status, created_at, archived, description,
     created_by, title_by, description_by, updated_by, updated_at, reviewed_at";

pub struct Db {
    pub(crate) conn: Connection,
    /// Where attachment files go (see `attachment.rs`); `None` in memory.
    pub(crate) attachments_dir: Option<PathBuf>,
    /// Who is attributed to writes on this connection (`"user"` by default).
    pub(crate) actor: Actor,
}

impl Db {
    /// Opens the database at the default location, creating it if needed.
    pub fn open_default() -> Result<Self> {
        Self::open(&default_path()?)
    }

    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            ensure_dir(dir)?;
        }
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        Self::init(conn, Some(attachments_dir_for(path)))
    }

    /// A database without a file, for tests. It can't have attachments.
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?, None)
    }

    fn init(conn: Connection, attachments_dir: Option<PathBuf>) -> Result<Self> {
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let mut db = Self {
            conn,
            attachments_dir,
            actor: Actor::user(),
        };
        db.migrate()?;
        Ok(db)
    }

    /// Brings the schema up to date. A database with data in it is backed
    /// up first (see [`Db::backup_before_upgrade`]).
    pub(crate) fn migrate(&mut self) -> Result<()> {
        self.migrate_with(MIGRATIONS, true)
    }

    /// Runs the pending `migrations`, all in one transaction: if one fails,
    /// none is applied and `user_version` stays as it was.
    pub(crate) fn migrate_with(&mut self, migrations: &[&str], back_up: bool) -> Result<()> {
        let latest = migrations.len() as i64;
        let version = self.schema_version()?;
        ensure_supported(version, latest)?;
        if version == latest {
            // Nothing to write: other connections see no change.
            return Ok(());
        }
        // Version 0 is a new, empty database; before migration 15 there is
        // nowhere to keep backup settings (no release had such a database).
        if back_up && version >= FIRST_BACKUP_SCHEMA && self.path().is_some() {
            self.backup_before_upgrade()
                .context("backing up before upgrading the database (nothing was changed)")?;
        }
        // Foreign keys off, so that rebuilding a table (SQLite's way to
        // change a column: create, copy, drop, rename) doesn't cascade-delete
        // the rows that point to it. The pragma has no effect inside a
        // transaction, hence here; the references are checked before commit.
        self.conn.pragma_update(None, "foreign_keys", "OFF")?;
        let result = self.run_migrations(migrations, latest);
        self.conn.pragma_update(None, "foreign_keys", "ON")?;
        result
    }

    fn run_migrations(&mut self, migrations: &[&str], latest: i64) -> Result<()> {
        // Immediate: another process (the GUI and the CLI, both upgraded)
        // can't migrate at the same time; the version is read again under
        // the lock, in case it just did.
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
        ensure_supported(version, latest)?;
        for (i, sql) in migrations.iter().enumerate().skip(version as usize) {
            tx.execute_batch(sql)
                .with_context(|| format!("running migration {}", i + 1))?;
        }
        let broken: Vec<String> = tx
            .prepare("SELECT DISTINCT \"table\" || ' → ' || parent FROM pragma_foreign_key_check")?
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        anyhow::ensure!(
            broken.is_empty(),
            "the migrations left rows pointing nowhere ({}); nothing was changed",
            broken.join(", ")
        );
        tx.pragma_update(None, "user_version", latest)?;
        tx.commit()?;
        Ok(())
    }

    /// `PRAGMA user_version`: how many migrations ran.
    pub(crate) fn schema_version(&self) -> Result<i64> {
        Ok(self
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))?)
    }

    /// The database file, or `None` for an in-memory database.
    pub fn path(&self) -> Option<PathBuf> {
        self.conn
            .path()
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
    }

    /// Whether this connection has changed any data since it was opened.
    pub fn has_written(&self) -> bool {
        self.conn.total_changes() > 0
    }

    /// Changes whenever *another* connection commits to the database.
    /// Poll it to detect external modifications (e.g. from the CLI).
    pub fn data_version(&self) -> Result<i64> {
        Ok(self
            .conn
            .pragma_query_value(None, "data_version", |row| row.get(0))?)
    }

    /// Issues that are not archived, in the user-defined order (new issues
    /// and issues whose status changed come first; see [`Db::move_issue`]).
    /// Fails if the filter names a label or project that doesn't exist.
    pub fn list_issues(&self, filter: &IssueFilter) -> Result<Vec<Issue>> {
        self.list_issues_where(filter, false)
    }

    /// Archived issues, newest first; otherwise like [`Db::list_issues`].
    pub fn list_archived_issues(&self, filter: &IssueFilter) -> Result<Vec<Issue>> {
        self.list_issues_where(filter, true)
    }

    fn list_issues_where(&self, filter: &IssueFilter, archived: bool) -> Result<Vec<Issue>> {
        let filter = self.resolve_filter(filter)?;
        let order = if archived {
            "id DESC"
        } else {
            "position, id DESC"
        };
        let mut issues: Vec<Issue> = self
            .conn
            .prepare_cached(&format!(
                "SELECT {ISSUE_COLUMNS} FROM issues WHERE archived = ?1 ORDER BY {order}"
            ))?
            .query_map([archived], issue_from_row)?
            .collect::<rusqlite::Result<_>>()?;
        let mut labels = self.names_per_issue(ISSUE_LABELS_SQL, None)?;
        let mut projects = self.names_per_issue(ISSUE_PROJECTS_SQL, None)?;
        let mut checklists = self.checklists_per_issue(None)?;
        let mut attachments = self.attachments_per_issue(None)?;
        let mut unreviewed = self.unreviewed_actors_per_issue(None)?;
        for issue in &mut issues {
            issue.labels = labels.remove(&issue.id).unwrap_or_default();
            issue.projects = projects.remove(&issue.id).unwrap_or_default();
            issue.checklist = checklists.remove(&issue.id).unwrap_or_default();
            issue.attachments = attachments.remove(&issue.id).unwrap_or_default();
            issue.unreviewed_actors = unreviewed.remove(&issue.id).unwrap_or_default();
            issue.unreviewed = !issue.unreviewed_actors.is_empty();
        }
        self.fill_dependencies(&mut issues, None)?;
        issues.retain(|i| filter.matches(i));
        Ok(issues)
    }

    pub fn get_issue(&self, id: IssueId) -> Result<Issue> {
        let mut issue = self
            .conn
            .query_row(
                &format!("SELECT {ISSUE_COLUMNS} FROM issues WHERE id = ?1"),
                [id],
                issue_from_row,
            )
            .optional()?
            .with_context(|| format!("no issue {id}"))?;
        issue.labels = self
            .names_per_issue(ISSUE_LABELS_SQL, Some(id))?
            .remove(&id)
            .unwrap_or_default();
        issue.projects = self
            .names_per_issue(ISSUE_PROJECTS_SQL, Some(id))?
            .remove(&id)
            .unwrap_or_default();
        issue.checklist = self
            .checklists_per_issue(Some(id))?
            .remove(&id)
            .unwrap_or_default();
        issue.attachments = self
            .attachments_per_issue(Some(id))?
            .remove(&id)
            .unwrap_or_default();
        issue.unreviewed_actors = self
            .unreviewed_actors_per_issue(Some(id))?
            .remove(&id)
            .unwrap_or_default();
        issue.unreviewed = !issue.unreviewed_actors.is_empty();
        self.fill_dependencies(std::slice::from_mut(&mut issue), Some(id))?;
        Ok(issue)
    }

    /// Creates an issue (and any labels that don't exist yet), adds it to
    /// its projects with their default labels, gives it its checklist, and
    /// returns it.
    pub fn create_issue(&self, new: &NewIssue) -> Result<Issue> {
        let title = normalize_title(&new.title)?;
        let labels = new
            .labels
            .iter()
            .map(|l| normalize_label_name(l))
            .collect::<Result<Vec<_>>>()?;
        let checklist = new
            .checklist
            .iter()
            .map(|t| normalize_item_text(t))
            .collect::<Result<Vec<_>>>()?;
        let tx = self.conn.unchecked_transaction()?;
        let id: IssueId = self.conn.query_row(
            "INSERT INTO issues
                (title, status, description, position,
                 created_by, title_by, description_by, updated_by, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?5, ?5, unixepoch()) RETURNING id",
            params![
                title,
                new.status,
                normalize_description(&new.description),
                self.first_position(Ordered::Issues)?,
                self.actor
            ],
            |row| row.get(0),
        )?;
        for label in &labels {
            let label_id = self.ensure_label(label)?;
            self.conn.execute(
                "INSERT OR IGNORE INTO issue_labels (issue_id, label_id) VALUES (?1, ?2)",
                params![id, label_id],
            )?;
        }
        self.join_projects(id, &new.projects)?;
        self.insert_checklist_items(id, &checklist)?;
        self.record_activity(id, ActivityKind::Created, "")?;
        tx.commit()?;
        self.get_issue(id)
    }

    /// Changes the fields of one issue (title, description) and returns it.
    /// The text it replaces is kept as a revision (see [`Db::issue_revisions`]).
    pub fn edit_issue(&self, id: IssueId, edit: &IssueEdit) -> Result<Issue> {
        let tx = self.conn.unchecked_transaction()?;
        let issue = self.get_issue(id)?;
        if let Some(title) = &edit.title {
            let title = normalize_title(title)?;
            if issue.title != title {
                self.record_edit(id, Field::Title, &issue.title, title)?;
                self.set_issue_column(id, "title", &title)?;
                self.set_issue_column(id, "title_by", &self.actor)?;
                self.record_activity(id, ActivityKind::TitleEdited, title)?;
            }
        }
        if let Some(description) = &edit.description {
            let description = normalize_description(description);
            if issue.description != description {
                self.record_edit(id, Field::Description, &issue.description, &description)?;
                self.set_issue_column(id, "description", &description)?;
                self.set_issue_column(id, "description_by", &self.actor)?;
                let detail = if description.is_empty() {
                    "cleared"
                } else {
                    ""
                };
                self.record_activity(id, ActivityKind::DescriptionEdited, detail)?;
            }
        }
        tx.commit()?;
        self.get_issue(id)
    }

    /// Applies a change to every given issue, all or nothing: if one fails
    /// (unknown issue, label or project), none is changed. Returns the
    /// updated issues, in the order given.
    ///
    /// Issues whose status changes go first in the order (keeping the order
    /// of `ids`), so they show at the top of their new status group.
    pub fn update_issues(&self, ids: &[IssueId], change: &IssueChange) -> Result<Vec<Issue>> {
        let tx = self.conn.unchecked_transaction()?;
        // Last first, so that the first one ends up on top.
        for &id in ids.iter().rev() {
            self.apply_change(id, change)?;
        }
        tx.commit()?;
        ids.iter().map(|&id| self.get_issue(id)).collect()
    }

    /// [`Db::update_issues`] for one issue.
    pub fn update_issue(&self, id: IssueId, change: &IssueChange) -> Result<Issue> {
        let mut issues = self.update_issues(&[id], change)?;
        Ok(issues.remove(0))
    }

    /// Applies a change to one issue. Call inside a transaction.
    fn apply_change(&self, id: IssueId, change: &IssueChange) -> Result<()> {
        match change {
            IssueChange::SetStatus(status) => {
                let old = self.get_issue(id)?.status;
                if old != *status {
                    self.set_issue_column(id, "status", status)?;
                    let top = self.first_position(Ordered::Issues)?;
                    self.set_issue_column(id, "position", &top)?;
                    self.record_activity(
                        id,
                        ActivityKind::StatusChanged,
                        &format!("{} → {}", old.key(), status.key()),
                    )?;
                }
                Ok(())
            }
            IssueChange::SetArchived(archived) => {
                let was_archived = self.get_issue(id)?.archived;
                self.set_issue_column(id, "archived", archived)?;
                if was_archived != *archived {
                    let kind = if *archived {
                        ActivityKind::Archived
                    } else {
                        ActivityKind::Unarchived
                    };
                    self.record_activity(id, kind, "")?;
                }
                Ok(())
            }
            IssueChange::AddLabels(labels) => {
                self.get_issue(id)?;
                let added = self.attach_labels(id, labels)?;
                if !added.is_empty() {
                    self.record_activity(id, ActivityKind::LabelsAdded, &added.join(", "))?;
                }
                Ok(())
            }
            IssueChange::RemoveLabels(labels) => {
                self.get_issue(id)?;
                let removed = self.detach_labels(id, labels)?;
                if !removed.is_empty() {
                    self.record_activity(id, ActivityKind::LabelsRemoved, &removed.join(", "))?;
                }
                Ok(())
            }
            IssueChange::AddToProjects(projects) => {
                self.get_issue(id)?;
                let joined = self.join_projects(id, projects)?;
                if !joined.is_empty() {
                    self.record_activity(id, ActivityKind::ProjectsAdded, &joined.join(", "))?;
                }
                Ok(())
            }
            IssueChange::RemoveFromProjects(projects) => {
                self.get_issue(id)?;
                let left = self.leave_projects(id, projects)?;
                if !left.is_empty() {
                    self.record_activity(id, ActivityKind::ProjectsRemoved, &left.join(", "))?;
                }
                Ok(())
            }
            IssueChange::AddDependencies(on) => {
                let added = self.insert_dependencies(id, on)?;
                if !added.is_empty() {
                    let detail = added
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ");
                    self.record_activity(id, ActivityKind::DependenciesAdded, &detail)?;
                }
                Ok(())
            }
            IssueChange::RemoveDependencies(on) => {
                let removed = self.delete_dependencies(id, on)?;
                if !removed.is_empty() {
                    let detail = removed
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ");
                    self.record_activity(id, ActivityKind::DependenciesRemoved, &detail)?;
                }
                Ok(())
            }
        }
    }

    pub(crate) fn set_issue_column(
        &self,
        id: IssueId,
        column: &str,
        value: &dyn ToSql,
    ) -> Result<()> {
        let updated = self.conn.execute(
            &format!("UPDATE issues SET {column} = ?2 WHERE id = ?1"),
            params![id, value],
        )?;
        anyhow::ensure!(updated == 1, "no issue {id}");
        Ok(())
    }

    /// Returns the updated issue.
    pub fn set_status(&self, id: IssueId, status: Status) -> Result<Issue> {
        self.update_issue(id, &IssueChange::SetStatus(status))
    }

    /// Moves an issue before or after another issue with the same status in
    /// the user-defined order. Returns the moved issue.
    pub fn move_issue(&self, id: IssueId, placement: &Placement<IssueId>) -> Result<Issue> {
        let issue = self.get_issue(id)?;
        let anchor = self.get_issue(*placement.anchor())?;
        anyhow::ensure!(
            issue.status == anchor.status,
            "issues {id} and {} don't have the same status",
            anchor.id
        );
        let tx = self.conn.unchecked_transaction()?;
        let order: Vec<i64> = self
            .conn
            .prepare_cached("SELECT id FROM issues ORDER BY position, id DESC")?
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        self.place(Ordered::Issues, order, id.0, anchor.id.0, placement)?;
        tx.commit()?;
        self.get_issue(id)
    }

    /// Archives an issue (or unarchives it): archived issues are left out of
    /// listings, views and counts, but keep everything else. Returns the issue.
    pub fn set_archived(&self, id: IssueId, archived: bool) -> Result<Issue> {
        self.update_issue(id, &IssueChange::SetArchived(archived))
    }

    /// Deletes an issue for good. Returns it as it was.
    pub fn delete_issue(&self, id: IssueId) -> Result<Issue> {
        let mut issues = self.delete_issues(&[id])?;
        Ok(issues.remove(0))
    }

    /// Deletes issues for good, with their attachments, all or nothing.
    /// Returns them as they were.
    pub fn delete_issues(&self, ids: &[IssueId]) -> Result<Vec<Issue>> {
        let tx = self.conn.unchecked_transaction()?;
        let mut deleted = Vec::with_capacity(ids.len());
        for &id in ids {
            if deleted.iter().any(|i: &Issue| i.id == id) {
                continue;
            }
            deleted.push(self.get_issue(id)?);
            self.conn
                .execute("DELETE FROM issues WHERE id = ?1", [id])?;
        }
        tx.commit()?;
        // After the commit: files are only removed once the issues are gone.
        let ids: Vec<IssueId> = deleted.iter().map(|i| i.id).collect();
        self.remove_attachment_dirs(&ids)?;
        Ok(deleted)
    }

    /// Related names per issue (`ISSUE_*_SQL`), for one issue or all of them.
    fn names_per_issue(
        &self,
        sql: &str,
        only: Option<IssueId>,
    ) -> Result<HashMap<IssueId, Vec<String>>> {
        let mut map: HashMap<IssueId, Vec<String>> = HashMap::new();
        let mut stmt = self.conn.prepare_cached(sql)?;
        let rows = stmt.query_map([only], |row| Ok((row.get(0)?, row.get(1)?)))?;
        for row in rows {
            let (id, name) = row?;
            map.entry(id).or_default().push(name);
        }
        Ok(map)
    }

    /// Checks that the filter is consistent and every name in it exists;
    /// uses stored spellings.
    pub(crate) fn resolve_filter(&self, filter: &IssueFilter) -> Result<IssueFilter> {
        anyhow::ensure!(
            !filter.no_project || filter.projects.is_empty(),
            "a filter cannot ask for both specific projects and no project"
        );
        anyhow::ensure!(
            !(filter.ready && filter.waiting),
            "a filter cannot ask for both ready and waiting issues"
        );
        let labels = |names: &[String]| -> Result<Vec<String>> {
            names.iter().map(|n| self.label_name(n)).collect()
        };
        Ok(IssueFilter {
            labels: labels(&filter.labels)?,
            without_labels: labels(&filter.without_labels)?,
            projects: filter
                .projects
                .iter()
                .map(|p| self.project_name(p))
                .collect::<Result<_>>()?,
            ..filter.clone()
        })
    }
}

fn issue_from_row(row: &Row) -> rusqlite::Result<Issue> {
    Ok(Issue {
        id: row.get(0)?,
        title: row.get(1)?,
        status: row.get(2)?,
        created_at: row.get(3)?,
        archived: row.get(4)?,
        description: row.get(5)?,
        labels: Vec::new(),
        projects: Vec::new(),
        checklist: Vec::new(),
        attachments: Vec::new(),
        depends_on: Vec::new(),
        dependents: Vec::new(),
        waiting_on: Vec::new(),
        created_by: row.get(6)?,
        title_by: row.get(7)?,
        description_by: row.get(8)?,
        updated_by: row.get(9)?,
        updated_at: row.get(10)?,
        reviewed_at: row.get(11)?,
        unreviewed: false,
        unreviewed_actors: Vec::new(),
    })
}

/// `SOLILOG_DB` if set, else the platform data directory.
pub fn default_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("SOLILOG_DB") {
        return Ok(path.into());
    }
    let dir = dirs::data_dir().context("no data directory for this platform")?;
    Ok(dir.join("Solilog").join("solilog.db"))
}

/// Creates `dir` (and any missing parents) if it does not exist yet.
///
/// Inside a macOS Seatbelt sandbox that blocks `dir`, `create_dir_all` can
/// fail with a misleading `EEXIST` ("File exists"): `mkdir` sees the existing
/// directory, then `is_dir()` calls `stat`, which fails with `EPERM` and gets
/// swallowed as `false`. We inspect `metadata(dir)` on `AlreadyExists` so
/// permission and sandbox blocks report their real cause.
pub(crate) fn ensure_dir(dir: &Path) -> Result<()> {
    if dir.as_os_str().is_empty() {
        return Ok(());
    }
    let Err(err) = std::fs::create_dir_all(dir) else {
        return Ok(());
    };
    classify_create_dir_error(dir, err, || std::fs::metadata(dir).map(|m| m.is_dir()))
}

fn classify_create_dir_error(
    dir: &Path,
    err: std::io::Error,
    stat: impl FnOnce() -> std::io::Result<bool>,
) -> Result<()> {
    use std::io::ErrorKind;

    match err.kind() {
        ErrorKind::AlreadyExists => match stat() {
            Ok(true) => Ok(()),
            Ok(false) => {
                anyhow::bail!(
                    "creating {}: path exists and is not a directory",
                    dir.display()
                )
            }
            Err(stat_err) if stat_err.kind() == ErrorKind::PermissionDenied => Err(stat_err)
                .with_context(|| {
                    format!(
                        "cannot access {} (blocked by permissions or a sandbox)",
                        dir.display()
                    )
                }),
            Err(_) => Err(err).with_context(|| format!("creating {}", dir.display())),
        },
        ErrorKind::PermissionDenied => Err(err).with_context(|| {
            format!(
                "cannot access {} (blocked by permissions or a sandbox)",
                dir.display()
            )
        }),
        _ => Err(err).with_context(|| format!("creating {}", dir.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles(db: &Db, filter: &IssueFilter) -> Vec<String> {
        db.list_issues(filter)
            .unwrap()
            .into_iter()
            .map(|i| i.title)
            .collect()
    }

    #[test]
    fn issue_order() {
        let db = Db::open_in_memory().unwrap();
        let all = IssueFilter::default();
        let id = |title: &str| db.create_issue(&NewIssue::new(title)).unwrap().id;
        let (a, b, c) = (id("a"), id("b"), id("c"));
        // New issues go first.
        assert_eq!(titles(&db, &all), ["c", "b", "a"]);

        db.move_issue(c, &Placement::After(a)).unwrap();
        assert_eq!(titles(&db, &all), ["b", "a", "c"]);
        db.move_issue(a, &Placement::Before(b)).unwrap();
        assert_eq!(titles(&db, &all), ["a", "b", "c"]);

        // Only among issues with the same status.
        db.set_status(c, Status::InProgress).unwrap();
        assert!(db.move_issue(c, &Placement::Before(a)).is_err());
        assert!(db.move_issue(a, &Placement::Before(IssueId(999))).is_err());

        // A status change puts the issues first, in the order given.
        assert_eq!(titles(&db, &all), ["c", "a", "b"]);
        db.update_issues(&[b, a], &IssueChange::SetStatus(Status::Done))
            .unwrap();
        assert_eq!(titles(&db, &all), ["b", "a", "c"]);
        // Setting the status an issue already has doesn't move it.
        db.set_status(c, Status::InProgress).unwrap();
        assert_eq!(titles(&db, &all), ["b", "a", "c"]);
    }

    #[test]
    fn edit_title() {
        let db = Db::open_in_memory().unwrap();
        let issue = db
            .create_issue(&NewIssue::new("old").labels(["bug"]))
            .unwrap();
        let edit = |title: &str| IssueEdit {
            title: Some(title.into()),
            ..Default::default()
        };
        let edited = db.edit_issue(issue.id, &edit("  new  ")).unwrap();
        assert_eq!(edited.title, "new");
        assert_eq!(edited.labels, ["bug"]);
        assert_eq!(db.get_issue(issue.id).unwrap().title, "new");

        assert!(db.edit_issue(issue.id, &edit("  ")).is_err());
        assert_eq!(db.get_issue(issue.id).unwrap().title, "new");
        assert!(db.edit_issue(IssueId(999), &edit("x")).is_err());
        // Nothing to change: returns the issue as it is.
        let same = db.edit_issue(issue.id, &IssueEdit::default()).unwrap();
        assert_eq!(same, edited);
    }

    #[test]
    fn descriptions() {
        let db = Db::open_in_memory().unwrap();
        let plain = db.create_issue(&NewIssue::new("plain")).unwrap();
        assert_eq!(plain.description, "");
        let issue = db
            .create_issue(&NewIssue::new("x").description("\nSteps:\r\n  1. play  \n\n"))
            .unwrap();
        assert_eq!(issue.description, "Steps:\n  1. play");
        assert_eq!(
            db.get_issue(issue.id).unwrap().description,
            "Steps:\n  1. play"
        );

        let edit = |description: &str| IssueEdit {
            description: Some(description.into()),
            ..Default::default()
        };
        let edited = db.edit_issue(issue.id, &edit("new\n\ntext")).unwrap();
        assert_eq!(edited.description, "new\n\ntext");
        assert_eq!(edited.title, "x", "the title is left alone");
        let listed = db.list_issues(&IssueFilter::default()).unwrap();
        assert_eq!(listed[0].description, "new\n\ntext");
        assert_eq!(listed[1].description, "");
        // An empty (or blank) description removes it.
        assert_eq!(
            db.edit_issue(issue.id, &edit("  \n")).unwrap().description,
            ""
        );
    }

    #[test]
    fn create_and_list() {
        let db = Db::open_in_memory().unwrap();
        db.create_issue(&NewIssue::new("first")).unwrap();
        db.create_issue(&NewIssue::new("  second  ")).unwrap();
        assert_eq!(titles(&db, &IssueFilter::default()), ["second", "first"]);
        assert!(db.create_issue(&NewIssue::new("   ")).is_err());
    }

    #[test]
    fn get_and_set_status() {
        let db = Db::open_in_memory().unwrap();
        let issue = db.create_issue(&NewIssue::new("x")).unwrap();
        assert_eq!(db.get_issue(issue.id).unwrap(), issue);
        let updated = db.set_status(issue.id, Status::Blocked).unwrap();
        assert_eq!(updated.status, Status::Blocked);
        assert_eq!(db.get_issue(issue.id).unwrap().status, Status::Blocked);
        assert!(db.set_status(IssueId(999), Status::Done).is_err());
        assert!(db.get_issue(IssueId(999)).is_err());
    }

    #[test]
    fn filters() {
        let db = Db::open_in_memory().unwrap();
        db.create_issue(&NewIssue::new("a")).unwrap();
        db.create_issue(&NewIssue::new("b").status(Status::Done))
            .unwrap();
        db.create_issue(&NewIssue::new("c").status(Status::Blocked))
            .unwrap();
        let open = IssueFilter {
            open_only: true,
            ..Default::default()
        };
        assert_eq!(titles(&db, &open), ["c", "a"]);
        let done = IssueFilter {
            statuses: vec![Status::Done],
            ..Default::default()
        };
        assert_eq!(titles(&db, &done), ["b"]);
    }

    #[test]
    fn archive_and_delete() {
        let db = Db::open_in_memory().unwrap();
        db.create_project(&crate::NewProject::new("p")).unwrap();
        let a = db
            .create_issue(&NewIssue::new("a").labels(["x"]).projects(["p"]))
            .unwrap();
        db.create_issue(&NewIssue::new("b")).unwrap();
        assert!(!a.archived);
        assert!(db.set_archived(a.id, true).unwrap().archived);
        // Out of counts, but keeps its labels and projects.
        assert_eq!(db.get_project("p").unwrap().issue_count, 0);
        assert_eq!(db.get_label("x").unwrap().issue_count, 0);
        assert_eq!(db.get_issue(a.id).unwrap().projects, ["p"]);
        assert!(db.get_issue(a.id).unwrap().archived);
        assert_eq!(titles(&db, &IssueFilter::default()), ["b"]);
        let archived = db.list_archived_issues(&IssueFilter::default()).unwrap();
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].title, "a");
        assert!(!db.set_archived(a.id, false).unwrap().archived);
        assert_eq!(titles(&db, &IssueFilter::default()), ["b", "a"]);
        assert!(db.set_archived(IssueId(999), true).is_err());

        assert_eq!(db.delete_issue(a.id).unwrap().title, "a");
        assert!(db.get_issue(a.id).is_err());
        assert!(db.delete_issue(a.id).is_err());
        assert_eq!(titles(&db, &IssueFilter::default()), ["b"]);
    }

    #[test]
    fn update_many() {
        let db = Db::open_in_memory().unwrap();
        db.create_project(&crate::NewProject {
            labels: vec!["from-p".into()],
            ..crate::NewProject::new("p")
        })
        .unwrap();
        let a = db.create_issue(&NewIssue::new("a")).unwrap().id;
        let b = db.create_issue(&NewIssue::new("b")).unwrap().id;
        let c = db.create_issue(&NewIssue::new("c")).unwrap().id;

        let updated = db
            .update_issues(&[b, a], &IssueChange::SetStatus(Status::Done))
            .unwrap();
        assert_eq!(
            updated.iter().map(|i| (i.id, i.status)).collect::<Vec<_>>(),
            [(b, Status::Done), (a, Status::Done)]
        );
        assert_eq!(db.get_issue(c).unwrap().status, Status::Todo);

        db.update_issues(&[a, b], &IssueChange::AddLabels(vec!["x/y".into()]))
            .unwrap();
        db.update_issues(&[a, b], &IssueChange::AddToProjects(vec!["p".into()]))
            .unwrap();
        assert_eq!(db.get_issue(a).unwrap().labels, ["from-p", "x/y"]);
        db.update_issues(&[a], &IssueChange::RemoveLabels(vec!["x/y".into()]))
            .unwrap();
        db.update_issues(&[a, b], &IssueChange::RemoveFromProjects(vec!["p".into()]))
            .unwrap();
        assert_eq!(db.get_issue(a).unwrap().labels, ["from-p"]);
        assert!(db.get_issue(b).unwrap().projects.is_empty());

        db.update_issues(&[a, c], &IssueChange::SetArchived(true))
            .unwrap();
        assert_eq!(titles(&db, &IssueFilter::default()), ["b"]);
    }

    #[test]
    fn update_many_is_all_or_nothing() {
        let db = Db::open_in_memory().unwrap();
        let a = db.create_issue(&NewIssue::new("a")).unwrap().id;
        let change = IssueChange::SetStatus(Status::Done);
        assert!(db.update_issues(&[a, IssueId(999)], &change).is_err());
        assert_eq!(db.get_issue(a).unwrap().status, Status::Todo);
        let change = IssueChange::AddToProjects(vec!["nope".into()]);
        assert!(db.update_issues(&[a], &change).is_err());

        assert!(db.delete_issues(&[a, IssueId(999)]).is_err());
        assert!(db.get_issue(a).is_ok());
        let b = db.create_issue(&NewIssue::new("b")).unwrap().id;
        let deleted = db.delete_issues(&[a, b, a]).unwrap();
        assert_eq!(deleted.len(), 2);
        assert!(titles(&db, &IssueFilter::default()).is_empty());
    }

    #[test]
    fn migrations_are_idempotent() {
        let mut db = Db::open_in_memory().unwrap();
        db.create_issue(&NewIssue::new("kept")).unwrap();
        db.migrate().unwrap();
        assert_eq!(db.list_issues(&IssueFilter::default()).unwrap().len(), 1);
    }

    #[test]
    fn migrations_can_rebuild_tables() {
        let mut db = Db::open_in_memory().unwrap();
        let issue = db
            .create_issue(&NewIssue::new("kept").labels(["x"]).checklist(["step"]))
            .unwrap();
        // Changes a column the SQLite way: rebuilding `issues`, which other
        // tables point to with ON DELETE CASCADE.
        let rebuild = "CREATE TABLE issues_new (
                id                   INTEGER PRIMARY KEY,
                title                TEXT    NOT NULL,
                created_at           INTEGER NOT NULL DEFAULT (unixepoch()),
                status               TEXT    NOT NULL DEFAULT 'todo',
                archived             INTEGER NOT NULL DEFAULT 0,
                description          TEXT    NOT NULL DEFAULT '',
                position             INTEGER NOT NULL DEFAULT 0,
                created_by           TEXT    NOT NULL DEFAULT 'user',
                title_by             TEXT    NOT NULL DEFAULT 'user',
                description_by       TEXT    NOT NULL DEFAULT 'user',
                updated_by           TEXT    NOT NULL DEFAULT 'user',
                updated_at           INTEGER NOT NULL DEFAULT 0,
                reviewed_at          INTEGER,
                reviewed_activity_id INTEGER NOT NULL DEFAULT 0
            );
            INSERT INTO issues_new SELECT * FROM issues;
            DROP TABLE issues;
            ALTER TABLE issues_new RENAME TO issues;";
        let migrations: Vec<&str> = MIGRATIONS.iter().copied().chain([rebuild]).collect();
        db.migrate_with(&migrations, true).unwrap();
        assert_eq!(db.get_issue(issue.id).unwrap(), issue);
        // Back on afterwards.
        let fk: bool = db
            .conn
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .unwrap();
        assert!(fk);
    }

    #[test]
    fn migrations_cannot_leave_broken_references() {
        let mut db = Db::open_in_memory().unwrap();
        let issue = db
            .create_issue(&NewIssue::new("kept").labels(["x"]))
            .unwrap();
        let migrations: Vec<&str> = MIGRATIONS
            .iter()
            .copied()
            .chain(["DELETE FROM labels"])
            .collect();
        let err = db.migrate_with(&migrations, true).unwrap_err();
        assert!(
            format!("{err:#}").contains("issue_labels → labels"),
            "{err:#}"
        );
        assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION);
        assert_eq!(db.get_issue(issue.id).unwrap().labels, ["x"]);
    }

    #[test]
    fn path_is_the_database_file() {
        assert_eq!(Db::open_in_memory().unwrap().path(), None);
        let dir = std::env::temp_dir().join(format!("solilog-test-path-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("p.db");
        let db = Db::open(&path).unwrap();
        assert_eq!(
            db.path().unwrap().canonicalize().unwrap(),
            path.canonicalize().unwrap()
        );
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn data_version_tracks_other_connections() {
        let dir = std::env::temp_dir().join(format!("solilog-test-{}", std::process::id()));
        let path = dir.join("dv.db");
        let _ = std::fs::remove_dir_all(&dir);
        let a = Db::open(&path).unwrap();
        let b = Db::open(&path).unwrap();
        let before = a.data_version().unwrap();
        a.create_issue(&NewIssue::new("own write")).unwrap();
        assert_eq!(a.data_version().unwrap(), before);
        b.create_issue(&NewIssue::new("other write")).unwrap();
        assert_ne!(a.data_version().unwrap(), before);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn sandbox_blocked_dir_reports_permission_error() {
        use std::io::{Error, ErrorKind};

        let dir = Path::new("/Users/example/Library/Application Support/Solilog");

        // macOS Seatbelt: mkdir returns EEXIST, stat returns EPERM.
        let err = classify_create_dir_error(
            dir,
            Error::new(ErrorKind::AlreadyExists, "File exists"),
            || Err(Error::new(ErrorKind::PermissionDenied, "Operation not permitted")),
        )
        .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("cannot access"), "{msg}");
        assert!(msg.contains("sandbox"), "{msg}");
        assert!(msg.contains("Operation not permitted"), "{msg}");
        assert!(!msg.contains("File exists"), "{msg}");

        // Direct EPERM/EACCES when the directory doesn't exist yet.
        let err = classify_create_dir_error(
            dir,
            Error::new(ErrorKind::PermissionDenied, "Operation not permitted"),
            || unreachable!("stat should not be called on PermissionDenied"),
        )
        .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("cannot access"), "{msg}");
        assert!(msg.contains("sandbox"), "{msg}");

        // Genuine file collision: EEXIST and stat confirms it's not a directory.
        let err = classify_create_dir_error(
            dir,
            Error::new(ErrorKind::AlreadyExists, "File exists"),
            || Ok(false),
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("not a directory"), "{err:#}");

        // Directory appeared concurrently.
        classify_create_dir_error(
            dir,
            Error::new(ErrorKind::AlreadyExists, "File exists"),
            || Ok(true),
        )
        .unwrap();
    }

    #[test]
    fn open_fails_clearly_when_parent_is_a_file() {
        let dir = std::env::temp_dir().join(format!(
            "solilog-test-parent-file-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("not-a-dir");
        std::fs::write(&file, b"").unwrap();

        let err = Db::open(&file.join("solilog.db")).err().unwrap();
        let msg = format!("{err:#}");
        assert!(msg.contains("not a directory"), "{msg}");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn open_fails_clearly_when_parent_is_unreadable() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = std::env::temp_dir().join(format!(
            "solilog-test-unreadable-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o000)).unwrap();

        let result = Db::open(&dir.join("sub").join("solilog.db"));

        // Restore permissions before asserting so cleanup always succeeds.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        // Running as root ignores 0o000 permissions.
        if let Err(err) = result {
            let msg = format!("{err:#}");
            assert!(msg.contains("cannot access"), "{msg}");
            assert!(msg.contains("sandbox"), "{msg}");
        }
    }
}
