//! SQLite persistence, and the domain operations built on it.
//!
//! The database lives at `~/Library/Application Support/Solilog/solilog.db`
//! (platform equivalent elsewhere) unless `SOLILOG_DB` points to another file.
//!
//! The database is the single source of truth. Several processes (the GUI,
//! any number of CLI invocations) may use it concurrently; WAL mode and a busy
//! timeout make that safe, and [`Db::data_version`] lets long-lived processes
//! notice changes made by others.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, Result};
use rusqlite::{Connection, OptionalExtension as _, Row, params};

use crate::issue::{Issue, IssueId, Status};

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
];

const ISSUE_COLUMNS: &str = "id, title, status, created_at";

/// Which issues [`Db::list_issues`] returns. The default matches everything.
#[derive(Debug, Clone, Default)]
pub struct IssueFilter {
    /// Only these statuses (any status if empty).
    pub statuses: Vec<Status>,
    /// Exclude done and cancelled issues.
    pub open_only: bool,
}

impl IssueFilter {
    pub fn matches(&self, issue: &Issue) -> bool {
        (self.statuses.is_empty() || self.statuses.contains(&issue.status))
            && !(self.open_only && issue.status.is_closed())
    }
}

pub struct Db {
    conn: Connection,
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

    /// Newest first.
    pub fn list_issues(&self, filter: &IssueFilter) -> Result<Vec<Issue>> {
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT {ISSUE_COLUMNS} FROM issues ORDER BY id DESC"
        ))?;
        let issues = stmt
            .query_map([], issue_from_row)?
            .filter(|r| r.as_ref().map_or(true, |i| filter.matches(i)))
            .collect::<rusqlite::Result<_>>()?;
        Ok(issues)
    }

    pub fn get_issue(&self, id: IssueId) -> Result<Issue> {
        self.conn
            .query_row(
                &format!("SELECT {ISSUE_COLUMNS} FROM issues WHERE id = ?1"),
                [id],
                issue_from_row,
            )
            .optional()?
            .with_context(|| format!("no issue {id}"))
    }

    pub fn create_issue(&self, title: &str, status: Status) -> Result<Issue> {
        let title = title.trim();
        anyhow::ensure!(!title.is_empty(), "issue title cannot be empty");
        Ok(self.conn.query_row(
            &format!(
                "INSERT INTO issues (title, status) VALUES (?1, ?2) RETURNING {ISSUE_COLUMNS}"
            ),
            params![title, status],
            issue_from_row,
        )?)
    }

    /// Returns the updated issue.
    pub fn set_status(&self, id: IssueId, status: Status) -> Result<Issue> {
        self.conn
            .query_row(
                &format!("UPDATE issues SET status = ?2 WHERE id = ?1 RETURNING {ISSUE_COLUMNS}"),
                params![id, status],
                issue_from_row,
            )
            .optional()?
            .with_context(|| format!("no issue {id}"))
    }
}

fn issue_from_row(row: &Row) -> rusqlite::Result<Issue> {
    Ok(Issue {
        id: row.get(0)?,
        title: row.get(1)?,
        status: row.get(2)?,
        created_at: row.get(3)?,
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
        db.create_issue("first", Status::Todo).unwrap();
        db.create_issue("  second  ", Status::Todo).unwrap();
        assert_eq!(titles(&db, &IssueFilter::default()), ["second", "first"]);
        assert!(db.create_issue("   ", Status::Todo).is_err());
    }

    #[test]
    fn get_and_set_status() {
        let db = Db::open_in_memory().unwrap();
        let issue = db.create_issue("x", Status::Todo).unwrap();
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
        db.create_issue("a", Status::Todo).unwrap();
        db.create_issue("b", Status::Done).unwrap();
        db.create_issue("c", Status::Blocked).unwrap();
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
    fn migrations_are_idempotent() {
        let mut db = Db::open_in_memory().unwrap();
        db.create_issue("kept", Status::Todo).unwrap();
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
        a.create_issue("own write", Status::Todo).unwrap();
        assert_eq!(a.data_version().unwrap(), before);
        b.create_issue("other write", Status::Todo).unwrap();
        assert_ne!(a.data_version().unwrap(), before);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
