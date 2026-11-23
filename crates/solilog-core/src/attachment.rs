//! Attachments: files kept with an issue.
//!
//! Files live in a directory next to the database (see
//! [`attachments_dir_for`]), one subdirectory per issue (`SOL-12/`), under
//! the name they were added with. Moving the database file together with
//! that directory keeps everything. The database lists the attachments;
//! files added to the directory by hand are not attachments.
//!
//! Names are unique within an issue (ignoring case): adding a file whose
//! name is taken adds a number (`notes (2).txt`). Deleting an issue deletes
//! its directory; archiving keeps it.

use std::collections::HashMap;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context as _, Result};
use rusqlite::types::{FromSql, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use rusqlite::{OptionalExtension as _, params};
use serde::{Deserialize, Serialize};

use crate::Db;
use crate::activity::ActivityKind;
use crate::issue::IssueId;

/// Identifies an attachment, across all issues. Displayed as a plain
/// number; parsed from `12` or `#12`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AttachmentId(pub i64);

impl fmt::Display for AttachmentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for AttachmentId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        s.strip_prefix('#')
            .unwrap_or(s)
            .parse::<i64>()
            .ok()
            .filter(|n| *n > 0)
            .map(AttachmentId)
            .ok_or_else(|| format!("invalid attachment id {s:?} (expected a number)"))
    }
}

impl ToSql for AttachmentId {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        self.0.to_sql()
    }
}

impl FromSql for AttachmentId {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        i64::column_result(value).map(AttachmentId)
    }
}

/// What an attachment is, judging by its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentKind {
    Image,
    File,
}

const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "heic", "heif", "svg", "avif",
];

impl AttachmentKind {
    pub fn of(name: &str) -> Self {
        let is_image = Path::new(name)
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| IMAGE_EXTENSIONS.iter().any(|i| i.eq_ignore_ascii_case(e)));
        if is_image {
            AttachmentKind::Image
        } else {
            AttachmentKind::File
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Attachment {
    pub id: AttachmentId,
    pub issue_id: IssueId,
    /// File name, unique within the issue (ignoring case).
    pub name: String,
    pub kind: AttachmentKind,
    /// Size in bytes, when it was added.
    pub size: u64,
    /// Unix timestamp (seconds).
    pub created_at: i64,
    /// Where the file is; absent for a database without a file (in memory).
    pub path: Option<PathBuf>,
}

/// `512 B`, `12 KB`, `3.4 MB`: sizes for people (1 KB = 1000 bytes).
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut size = bytes as f64 / 1000.;
    let mut unit = 0;
    while size >= 999.95 && unit + 1 < UNITS.len() {
        size /= 1000.;
        unit += 1;
    }
    if size < 10. {
        format!("{size:.1} {}", UNITS[unit])
    } else {
        format!("{size:.0} {}", UNITS[unit])
    }
}

/// The attachments directory of the database at `db_path`: a sibling
/// directory named after it (`solilog.db` → `solilog-attachments/`).
pub fn attachments_dir_for(db_path: &Path) -> PathBuf {
    let stem = db_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "solilog".into());
    db_path.with_file_name(format!("{stem}-attachments"))
}

/// Checks that a file name can be used as is: one path component, not
/// empty, no control characters. Trims surrounding whitespace.
pub fn normalize_attachment_name(name: &str) -> Result<String> {
    let name = name.trim();
    anyhow::ensure!(!name.is_empty(), "attachment name cannot be empty");
    anyhow::ensure!(
        name != "." && name != ".." && !name.contains(['/', '\\']),
        "invalid attachment name {name:?}"
    );
    anyhow::ensure!(
        !name.chars().any(char::is_control),
        "invalid attachment name {name:?}"
    );
    Ok(name.to_owned())
}

/// `notes.txt` → `notes (n).txt`.
fn numbered(name: &str, n: usize) -> String {
    let path = Path::new(name);
    match (
        path.file_stem().and_then(|s| s.to_str()),
        path.extension().and_then(|e| e.to_str()),
    ) {
        (Some(stem), Some(ext)) if !stem.is_empty() => format!("{stem} ({n}).{ext}"),
        _ => format!("{name} ({n})"),
    }
}

/// Where data for a new attachment comes from.
enum Source<'a> {
    File(&'a Path),
    Bytes(&'a [u8]),
}

const ATTACHMENTS_SQL: &str = "SELECT id, issue_id, name, size, created_at FROM attachments
     WHERE ?1 IS NULL OR issue_id = ?1
     ORDER BY issue_id, id";

impl Db {
    /// Where attachments are stored, or `None` for an in-memory database.
    pub fn attachments_dir(&self) -> Option<&Path> {
        self.attachments_dir.as_deref()
    }

    /// The directory holding an issue's attachments (it may not exist yet).
    pub fn issue_attachments_dir(&self, issue: IssueId) -> Result<PathBuf> {
        Ok(self.require_attachments_dir()?.join(issue.to_string()))
    }

    pub(crate) fn require_attachments_dir(&self) -> Result<&Path> {
        self.attachments_dir()
            .context("attachments need a database file (this one is in memory)")
    }

    /// Copies files into an issue's attachments, all or nothing, and
    /// returns the new attachments, in the order given. Each keeps its file
    /// name, numbered if the issue already has one by that name.
    pub fn add_attachments(&self, issue: IssueId, files: &[PathBuf]) -> Result<Vec<Attachment>> {
        let mut sources = Vec::with_capacity(files.len());
        for file in files {
            let meta = fs::metadata(file).with_context(|| format!("reading {}", file.display()))?;
            anyhow::ensure!(
                meta.is_file(),
                "{} is not a file (folders can't be attached)",
                file.display()
            );
            let name = file
                .file_name()
                .with_context(|| format!("{} has no file name", file.display()))?
                .to_string_lossy();
            sources.push((normalize_attachment_name(&name)?, Source::File(file)));
        }
        self.insert_attachments(issue, sources)
    }

    /// Adds data (e.g. a pasted image) as an attachment named `name`
    /// (numbered if taken), and returns it.
    pub fn add_attachment_data(
        &self,
        issue: IssueId,
        name: &str,
        data: &[u8],
    ) -> Result<Attachment> {
        let name = normalize_attachment_name(name)?;
        let mut added = self.insert_attachments(issue, vec![(name, Source::Bytes(data))])?;
        Ok(added.remove(0))
    }

    /// Writes the files first (never replacing one), then records them in
    /// one short transaction. Files written are removed if anything fails.
    fn insert_attachments(
        &self,
        issue: IssueId,
        sources: Vec<(String, Source)>,
    ) -> Result<Vec<Attachment>> {
        self.get_issue(issue)?;
        let dir = self.issue_attachments_dir(issue)?;
        crate::db::ensure_dir(&dir)?;
        let mut written: Vec<(String, PathBuf, u64)> = Vec::new();
        let result = (|| -> Result<Vec<AttachmentId>> {
            for (name, source) in &sources {
                let taken: Vec<&str> = written.iter().map(|(n, ..)| n.as_str()).collect();
                let (name, path, size) =
                    self.write_attachment(issue, &dir, name, source, &taken)?;
                written.push((name, path, size));
            }
            let tx = self.conn.unchecked_transaction()?;
            let mut ids = Vec::with_capacity(written.len());
            for (name, _, size) in &written {
                ids.push(self.conn.query_row(
                    "INSERT INTO attachments (issue_id, name, size) VALUES (?1, ?2, ?3)
                     RETURNING id",
                    params![issue, name, *size as i64],
                    |row| row.get(0),
                )?);
            }
            if !written.is_empty() {
                let names: Vec<&str> = written.iter().map(|(n, ..)| n.as_str()).collect();
                self.record_activity(issue, ActivityKind::AttachmentAdded, &names.join(", "))?;
            }
            tx.commit()?;
            Ok(ids)
        })();
        match result {
            Ok(ids) => ids.into_iter().map(|id| self.get_attachment(id)).collect(),
            Err(err) => {
                for (_, path, _) in &written {
                    let _ = fs::remove_file(path);
                }
                Err(err)
            }
        }
    }

    /// Writes one file under the first free name: not used by the issue's
    /// attachments, by `taken`, or by a file already in the directory.
    fn write_attachment(
        &self,
        issue: IssueId,
        dir: &Path,
        name: &str,
        source: &Source,
        taken: &[&str],
    ) -> Result<(String, PathBuf, u64)> {
        for n in 1.. {
            let candidate = if n == 1 {
                name.to_owned()
            } else {
                numbered(name, n)
            };
            let used: bool = self.conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM attachments
                 WHERE issue_id = ?1 AND name = ?2 COLLATE NOCASE)",
                params![issue, candidate],
                |row| row.get(0),
            )?;
            if used || taken.iter().any(|t| t.eq_ignore_ascii_case(&candidate)) {
                continue;
            }
            let path = dir.join(&candidate);
            let mut file = match File::options().write(true).create_new(true).open(&path) {
                Ok(file) => file,
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(err) => {
                    return Err(err).with_context(|| format!("creating {}", path.display()));
                }
            };
            let copied = match source {
                Source::File(from) => File::open(from)
                    .and_then(|mut from| io::copy(&mut from, &mut file))
                    .with_context(|| format!("copying {}", from.display())),
                Source::Bytes(data) => file
                    .write_all(data)
                    .map(|()| data.len() as u64)
                    .with_context(|| format!("writing {}", path.display())),
            };
            return match copied {
                Ok(size) => Ok((candidate, path, size)),
                Err(err) => {
                    drop(file);
                    let _ = fs::remove_file(&path);
                    Err(err)
                }
            };
        }
        unreachable!()
    }

    pub fn get_attachment(&self, id: AttachmentId) -> Result<Attachment> {
        self.conn
            .query_row(
                "SELECT id, issue_id, name, size, created_at FROM attachments WHERE id = ?1",
                [id],
                |row| self.attachment_from_row(row),
            )
            .optional()?
            .with_context(|| format!("no attachment {id}"))
    }

    /// An issue's attachments, in the order they were added.
    pub fn list_attachments(&self, issue: IssueId) -> Result<Vec<Attachment>> {
        self.get_issue(issue)?;
        Ok(self
            .attachments_per_issue(Some(issue))?
            .remove(&issue)
            .unwrap_or_default())
    }

    /// Deletes an attachment and its file. Returns it as it was.
    pub fn delete_attachment(&self, id: AttachmentId) -> Result<Attachment> {
        let attachment = self.get_attachment(id)?;
        let tx = self.conn.unchecked_transaction()?;
        self.conn
            .execute("DELETE FROM attachments WHERE id = ?1", [id])?;
        self.record_activity(
            attachment.issue_id,
            ActivityKind::AttachmentRemoved,
            &attachment.name,
        )?;
        if let Some(path) = &attachment.path {
            remove_if_present(path, |p| fs::remove_file(p))?;
            // Leave no empty directory behind.
            if let Some(dir) = path.parent() {
                let _ = fs::remove_dir(dir);
            }
        }
        tx.commit()?;
        Ok(attachment)
    }

    /// Removes the attachment directories of deleted issues. Their rows are
    /// gone with the issues.
    pub(crate) fn remove_attachment_dirs(&self, issues: &[IssueId]) -> Result<()> {
        let Some(dir) = self.attachments_dir() else {
            return Ok(());
        };
        for issue in issues {
            remove_if_present(&dir.join(issue.to_string()), |p| fs::remove_dir_all(p))?;
        }
        Ok(())
    }

    /// Attachments per issue, for one issue or all of them.
    pub(crate) fn attachments_per_issue(
        &self,
        only: Option<IssueId>,
    ) -> Result<HashMap<IssueId, Vec<Attachment>>> {
        let mut map: HashMap<IssueId, Vec<Attachment>> = HashMap::new();
        let mut stmt = self.conn.prepare_cached(ATTACHMENTS_SQL)?;
        let rows = stmt.query_map([only], |row| self.attachment_from_row(row))?;
        for row in rows {
            let attachment = row?;
            map.entry(attachment.issue_id).or_default().push(attachment);
        }
        Ok(map)
    }

    fn attachment_from_row(&self, row: &rusqlite::Row) -> rusqlite::Result<Attachment> {
        let issue_id: IssueId = row.get(1)?;
        let name: String = row.get(2)?;
        Ok(Attachment {
            id: row.get(0)?,
            issue_id,
            kind: AttachmentKind::of(&name),
            size: row.get::<_, i64>(3)?.max(0) as u64,
            created_at: row.get(4)?,
            path: self
                .attachments_dir()
                .map(|dir| dir.join(issue_id.to_string()).join(&name)),
            name,
        })
    }
}

fn remove_if_present(path: &Path, remove: fn(&Path) -> io::Result<()>) -> Result<()> {
    match remove(path) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => {
            Err(err).with_context(|| format!("removing {}", path.display()))
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IssueFilter, NewIssue};

    /// A database file in a fresh temporary directory, removed on drop.
    struct TempDb {
        dir: PathBuf,
        db: Db,
    }

    impl TempDb {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("solilog-attachments-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let db = Db::open(&dir.join("test.db")).unwrap();
            Self { dir, db }
        }

        fn file(&self, name: &str, content: &str) -> PathBuf {
            let path = self.dir.join("src").join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, content).unwrap();
            path
        }
    }

    impl Drop for TempDb {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    fn names(attachments: &[Attachment]) -> Vec<&str> {
        attachments.iter().map(|a| a.name.as_str()).collect()
    }

    #[test]
    fn ids_names_kinds_sizes() {
        assert_eq!("#4".parse(), Ok(AttachmentId(4)));
        assert!("0".parse::<AttachmentId>().is_err());
        assert_eq!(AttachmentKind::of("shot.PNG"), AttachmentKind::Image);
        assert_eq!(AttachmentKind::of("notes.txt"), AttachmentKind::File);
        assert_eq!(AttachmentKind::of("png"), AttachmentKind::File);
        assert_eq!(numbered("a.tar.gz", 2), "a.tar (2).gz");
        assert_eq!(numbered("README", 3), "README (3)");
        assert_eq!(numbered(".env", 2), ".env (2)");
        assert!(normalize_attachment_name("a/b").is_err());
        assert!(normalize_attachment_name("..").is_err());
        assert!(normalize_attachment_name(" ").is_err());
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(999), "999 B");
        assert_eq!(format_size(1000), "1.0 KB");
        assert_eq!(format_size(12_345), "12 KB");
        assert_eq!(format_size(999_960), "1.0 MB");
        assert_eq!(format_size(3_400_000), "3.4 MB");
        assert_eq!(
            attachments_dir_for(Path::new("/data/solilog.db")),
            Path::new("/data/solilog-attachments")
        );
    }

    #[test]
    fn add_list_delete() {
        let t = TempDb::new("add");
        let db = &t.db;
        let issue = db.create_issue(&NewIssue::new("x")).unwrap().id;
        let notes = t.file("notes.txt", "hello");
        let shot = t.file("shot.png", "png!");

        let added = db
            .add_attachments(issue, &[notes.clone(), shot.clone()])
            .unwrap();
        assert_eq!(names(&added), ["notes.txt", "shot.png"]);
        assert_eq!(added[0].size, 5);
        assert_eq!(added[1].kind, AttachmentKind::Image);
        let path = added[0].path.clone().unwrap();
        assert_eq!(
            path,
            db.issue_attachments_dir(issue).unwrap().join("notes.txt")
        );
        assert!(path.starts_with(t.dir.join("test-attachments").join("SOL-1")));
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello");
        assert!(notes.exists(), "the original stays");

        // Taken names get a number, ignoring case.
        let again = db
            .add_attachments(issue, std::slice::from_ref(&notes))
            .unwrap();
        assert_eq!(names(&again), ["notes (2).txt"]);
        let pasted = db.add_attachment_data(issue, "NOTES.txt", b"abc").unwrap();
        assert_eq!(pasted.name, "NOTES (3).txt");
        assert_eq!(pasted.size, 3);

        // Issues carry them; listings too.
        let all = db.get_issue(issue).unwrap().attachments;
        assert_eq!(
            names(&all),
            ["notes.txt", "shot.png", "notes (2).txt", "NOTES (3).txt"]
        );
        assert_eq!(db.list_attachments(issue).unwrap(), all);
        let listed = db.list_issues(&IssueFilter::default()).unwrap();
        assert_eq!(listed[0].attachments, all);

        let deleted = db.delete_attachment(added[0].id).unwrap();
        assert_eq!(deleted.name, "notes.txt");
        assert!(!path.exists());
        assert!(db.delete_attachment(added[0].id).is_err());
        assert_eq!(db.list_attachments(issue).unwrap().len(), 3);
    }

    #[test]
    fn all_or_nothing() {
        let t = TempDb::new("atomic");
        let db = &t.db;
        let issue = db.create_issue(&NewIssue::new("x")).unwrap().id;
        let ok = t.file("ok.txt", "ok");
        let missing = t.dir.join("missing.txt");
        assert!(db.add_attachments(issue, &[ok.clone(), missing]).is_err());
        assert!(db.add_attachments(issue, &[t.dir.join("src")]).is_err());
        assert!(db.list_attachments(issue).unwrap().is_empty());
        assert!(
            db.add_attachments(IssueId(99), std::slice::from_ref(&ok))
                .is_err()
        );
        // Nothing left in the directory either.
        let dir = db.issue_attachments_dir(issue).unwrap();
        assert_eq!(fs::read_dir(&dir).map_or(0, |d| d.count()), 0);

        // A stray file in the directory is not replaced.
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("ok.txt"), "stray").unwrap();
        let added = db.add_attachments(issue, &[ok]).unwrap();
        assert_eq!(names(&added), ["ok (2).txt"]);
        assert_eq!(fs::read_to_string(dir.join("ok.txt")).unwrap(), "stray");
    }

    #[test]
    fn deleting_an_issue_deletes_its_files() {
        let t = TempDb::new("delete");
        let db = &t.db;
        let a = db.create_issue(&NewIssue::new("a")).unwrap().id;
        let b = db.create_issue(&NewIssue::new("b")).unwrap().id;
        let file = t.file("f.txt", "f");
        db.add_attachments(a, std::slice::from_ref(&file)).unwrap();
        db.add_attachments(b, &[file]).unwrap();
        let (dir_a, dir_b) = (
            db.issue_attachments_dir(a).unwrap(),
            db.issue_attachments_dir(b).unwrap(),
        );
        // Archiving keeps them.
        db.set_archived(a, true).unwrap();
        assert!(dir_a.join("f.txt").exists());
        assert_eq!(db.get_issue(a).unwrap().attachments.len(), 1);

        let deleted = db.delete_issue(a).unwrap();
        assert_eq!(deleted.attachments.len(), 1);
        assert!(!dir_a.exists());
        assert!(dir_b.join("f.txt").exists());
        // Deleting an issue without attachments is fine.
        let c = db.create_issue(&NewIssue::new("c")).unwrap().id;
        db.delete_issue(c).unwrap();
    }

    #[test]
    fn in_memory_databases_have_none() {
        let db = Db::open_in_memory().unwrap();
        let issue = db.create_issue(&NewIssue::new("x")).unwrap();
        assert!(issue.attachments.is_empty());
        assert!(db.add_attachment_data(issue.id, "a.txt", b"a").is_err());
        db.delete_issue(issue.id).unwrap();
    }
}
