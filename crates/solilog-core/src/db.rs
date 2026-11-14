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
use rusqlite::{Connection, OptionalExtension as _, Row, params};

use crate::filter::IssueFilter;
use crate::issue::{Issue, IssueId, NewIssue, Status};
use crate::label::normalize_label_name;

/// Schema migrations, applied in order. `PRAGMA user_version` tracks how many ran.
/// Never edit an existing entry; append a new one.
const MIGRATIONS: &[&str] = &[
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
];

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

const ISSUE_COLUMNS: &str = "id, title, status, created_at, archived";

pub struct Db {
    pub(crate) conn: Connection,
}

impl Db {
    /// Opens the database at the default location, creating it if needed.
    pub fn open_default() -> Result<Self> {
        Self::open(&default_path()?)
    }

    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let mut db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&mut self) -> Result<()> {
        let version: i64 = self
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))?;
        anyhow::ensure!(
            version as usize <= MIGRATIONS.len(),
            "database schema version {version} is newer than this build supports ({}); update solilog",
            MIGRATIONS.len()
        );
        let tx = self.conn.transaction()?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            tx.execute_batch(sql)
                .with_context(|| format!("running migration {}", i + 1))?;
        }
        tx.pragma_update(None, "user_version", MIGRATIONS.len() as i64)?;
        tx.commit()?;
        Ok(())
    }

    /// Changes whenever *another* connection commits to the database.
    /// Poll it to detect external modifications (e.g. from the CLI).
    pub fn data_version(&self) -> Result<i64> {
        Ok(self
            .conn
            .pragma_query_value(None, "data_version", |row| row.get(0))?)
    }

    /// Issues that are not archived, newest first. Fails if the filter names
    /// a label or project that doesn't exist.
    pub fn list_issues(&self, filter: &IssueFilter) -> Result<Vec<Issue>> {
        self.list_issues_where(filter, false)
    }

    /// Archived issues, newest first; otherwise like [`Db::list_issues`].
    pub fn list_archived_issues(&self, filter: &IssueFilter) -> Result<Vec<Issue>> {
        self.list_issues_where(filter, true)
    }

    fn list_issues_where(&self, filter: &IssueFilter, archived: bool) -> Result<Vec<Issue>> {
        let filter = self.resolve_filter(filter)?;
        let mut issues: Vec<Issue> = self
            .conn
            .prepare_cached(&format!(
                "SELECT {ISSUE_COLUMNS} FROM issues WHERE archived = ?1 ORDER BY id DESC"
            ))?
            .query_map([archived], issue_from_row)?
            .collect::<rusqlite::Result<_>>()?;
        let mut labels = self.names_per_issue(ISSUE_LABELS_SQL, None)?;
        let mut projects = self.names_per_issue(ISSUE_PROJECTS_SQL, None)?;
        for issue in &mut issues {
            issue.labels = labels.remove(&issue.id).unwrap_or_default();
            issue.projects = projects.remove(&issue.id).unwrap_or_default();
        }
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
        Ok(issue)
    }

    /// Creates an issue (and any labels that don't exist yet), adds it to
    /// its projects with their default labels, and returns it.
    pub fn create_issue(&self, new: &NewIssue) -> Result<Issue> {
        let title = new.title.trim();
        anyhow::ensure!(!title.is_empty(), "issue title cannot be empty");
        let labels = new
            .labels
            .iter()
            .map(|l| normalize_label_name(l))
            .collect::<Result<Vec<_>>>()?;
        let tx = self.conn.unchecked_transaction()?;
        let id: IssueId = self.conn.query_row(
            "INSERT INTO issues (title, status) VALUES (?1, ?2) RETURNING id",
            params![title, new.status],
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
        tx.commit()?;
        self.get_issue(id)
    }

    /// Returns the updated issue.
    pub fn set_status(&self, id: IssueId, status: Status) -> Result<Issue> {
        let updated = self.conn.execute(
            "UPDATE issues SET status = ?2 WHERE id = ?1",
            params![id, status],
        )?;
        anyhow::ensure!(updated == 1, "no issue {id}");
        self.get_issue(id)
    }

    /// Archives an issue (or unarchives it): archived issues are left out of
    /// listings, views and counts, but keep everything else. Returns the issue.
    pub fn set_archived(&self, id: IssueId, archived: bool) -> Result<Issue> {
        let updated = self.conn.execute(
            "UPDATE issues SET archived = ?2 WHERE id = ?1",
            params![id, archived],
        )?;
        anyhow::ensure!(updated == 1, "no issue {id}");
        self.get_issue(id)
    }

    /// Deletes an issue for good. Returns it as it was.
    pub fn delete_issue(&self, id: IssueId) -> Result<Issue> {
        let issue = self.get_issue(id)?;
        self.conn
            .execute("DELETE FROM issues WHERE id = ?1", [id])?;
        Ok(issue)
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
        labels: Vec::new(),
        projects: Vec::new(),
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
    fn migrations_are_idempotent() {
        let mut db = Db::open_in_memory().unwrap();
        db.create_issue(&NewIssue::new("kept")).unwrap();
        db.migrate().unwrap();
        assert_eq!(db.list_issues(&IssueFilter::default()).unwrap().len(), 1);
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
}
