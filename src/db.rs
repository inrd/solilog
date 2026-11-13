//! SQLite persistence.
//!
//! The database lives at `~/Library/Application Support/Solilog/solilog.db`
//! (platform equivalent elsewhere) unless `SOLILOG_DB` points to another file.

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use rusqlite::{Connection, params};

/// Schema migrations, applied in order. `PRAGMA user_version` tracks how many ran.
/// Never edit an existing entry; append a new one.
const MIGRATIONS: &[&str] = &[
    // 1: issues
    "CREATE TABLE issues (
        id         INTEGER PRIMARY KEY,
        title      TEXT    NOT NULL,
        created_at INTEGER NOT NULL DEFAULT (unixepoch())
    );",
];

#[derive(Debug, Clone)]
pub struct Issue {
    pub id: i64,
    pub title: String,
}

pub struct Db {
    conn: Connection,
}

impl Db {
    /// Opens the database at the default location, creating it if needed.
    pub fn open_default() -> Result<Self> {
        Self::open(default_path()?)
    }

    pub fn open(path: PathBuf) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let conn =
            Connection::open(&path).with_context(|| format!("opening {}", path.display()))?;
        Self::init(conn)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
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
        let tx = self.conn.transaction()?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            tx.execute_batch(sql)
                .with_context(|| format!("running migration {}", i + 1))?;
        }
        tx.pragma_update(None, "user_version", MIGRATIONS.len() as i64)?;
        tx.commit()?;
        Ok(())
    }

    pub fn list_issues(&self) -> Result<Vec<Issue>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, title FROM issues ORDER BY id DESC")?;
        let issues = stmt
            .query_map([], |row| {
                Ok(Issue {
                    id: row.get(0)?,
                    title: row.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(issues)
    }

    pub fn create_issue(&self, title: &str) -> Result<Issue> {
        self.conn
            .execute("INSERT INTO issues (title) VALUES (?1)", params![title])?;
        Ok(Issue {
            id: self.conn.last_insert_rowid(),
            title: title.to_owned(),
        })
    }
}

fn default_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("SOLILOG_DB") {
        return Ok(path.into());
    }
    let dir = dirs::data_dir().context("no data directory for this platform")?;
    Ok(dir.join("Solilog").join("solilog.db"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_list() {
        let db = Db::open_in_memory().unwrap();
        db.create_issue("first").unwrap();
        db.create_issue("second").unwrap();
        let titles: Vec<_> = db
            .list_issues()
            .unwrap()
            .into_iter()
            .map(|i| i.title)
            .collect();
        assert_eq!(titles, ["second", "first"]);
    }

    #[test]
    fn migrations_are_idempotent() {
        let mut db = Db::open_in_memory().unwrap();
        db.create_issue("kept").unwrap();
        db.migrate().unwrap();
        assert_eq!(db.list_issues().unwrap().len(), 1);
    }
}
