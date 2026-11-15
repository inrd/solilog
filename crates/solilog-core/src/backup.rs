//! Backups: zip archives of the database and its attachments.
//!
//! A backup is `solilog-<date>-<time>-<database id>.zip` (UTC) in the
//! backup directory: by default next to the database (`solilog.db` →
//! `solilog-backups/`), else wherever [`Db::set_backup_dir`] put it (e.g. a
//! synced folder), along with the existing backups. It is a plain zip, so it
//! can be opened without Solilog, and holds:
//!
//! - `manifest.json`: what it is (a [`Manifest`]);
//! - `solilog.db`: a consistent snapshot of the database (`VACUUM INTO`);
//! - `attachments/SOL-12/<name>`: the attachment files.
//!
//! [`Db::auto_backup`] makes one when the last one is an hour old and the
//! data changed since. Every new backup thins out the older ones (see
//! [`to_prune`]). The settings live in the database (`settings` table), so
//! a scratch database (`SOLILOG_DB`) keeps its backups next to it.
//!
//! A directory can hold backups of other databases, e.g. one synced folder
//! for several computers: each database has an id (in the file name), and
//! only its own backups are counted, thinned out or moved. Any backup can
//! be restored.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use rusqlite::{Connection, MAIN_DB, OpenFlags, OptionalExtension as _, backup::Progress, params};
use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::Db;
use crate::db::SCHEMA_VERSION;
use crate::issue::IssueId;

/// Minimum time between automatic backups, in seconds.
pub const AUTO_BACKUP_INTERVAL: i64 = 60 * 60;

const PREFIX: &str = "solilog-";
const EXTENSION: &str = ".zip";
const MANIFEST: &str = "manifest.json";
const DATABASE: &str = "solilog.db";
const ATTACHMENTS: &str = "attachments";
/// Version of the archive layout; newer ones can't be restored.
const FORMAT: u32 = 1;

const DATABASE_ID: &str = "database_id";
const BACKUP_DIR: &str = "backup_dir";

const DAY: i64 = 24 * 60 * 60;

/// File types that are compressed already: stored as is.
const COMPRESSED_EXTENSIONS: &[&str] = &[
    "7z", "avif", "bz2", "docx", "gif", "gz", "heic", "heif", "jpeg", "jpg", "m4a", "mkv", "mov",
    "mp3", "mp4", "ogg", "pdf", "png", "pptx", "rar", "webm", "webp", "xlsx", "xz", "zip", "zst",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupReason {
    /// Made automatically ([`Db::auto_backup`]).
    Auto,
    /// Asked for.
    Manual,
    /// Made by [`Db::restore_backup`] before restoring.
    BeforeRestore,
}

impl BackupReason {
    pub fn label(self) -> &'static str {
        match self {
            BackupReason::Auto => "automatic",
            BackupReason::Manual => "manual",
            BackupReason::BeforeRestore => "before restore",
        }
    }
}

/// `manifest.json`: describes a backup without unpacking it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    /// The Solilog version that made it.
    pub app_version: String,
    /// The database's schema version (number of migrations).
    pub schema_version: i64,
    pub database_id: String,
    /// Unix timestamp (seconds).
    pub created_at: i64,
    pub reason: BackupReason,
    /// Identifies the database content, to tell whether it changed since.
    pub fingerprint: String,
    pub issue_count: u64,
    pub attachment_count: u64,
}

/// A backup archive.
#[derive(Debug, Clone, Serialize)]
pub struct Backup {
    /// File name; identifies it within the backup directory.
    pub name: String,
    pub path: PathBuf,
    /// Archive size in bytes.
    pub size: u64,
    /// Unix timestamp (seconds).
    pub created_at: i64,
    pub reason: BackupReason,
    pub database_id: String,
    /// Made from this database (rather than another one sharing the directory).
    pub own: bool,
    pub schema_version: i64,
    pub app_version: String,
    pub issue_count: u64,
    pub attachment_count: u64,
}

/// Where backups go.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BackupDir {
    pub path: PathBuf,
    /// The default: next to the database.
    pub is_default: bool,
}

/// The result of [`Db::set_backup_dir`].
#[derive(Debug, Clone, Serialize)]
pub struct BackupDirChange {
    #[serde(flatten)]
    pub dir: BackupDir,
    /// This database's backups moved from the previous directory.
    pub moved: usize,
    /// Those left there because the new directory has a file by that name.
    pub not_moved: usize,
}

/// The result of [`Db::restore_backup`].
#[derive(Debug, Clone, Serialize)]
pub struct Restored {
    pub restored: Backup,
    /// The data as it was before, to undo the restore.
    pub before_restore: Backup,
}

/// `2026-11-14 15:30 UTC`.
pub fn format_utc(timestamp: i64) -> String {
    let (y, mo, d, h, mi, _) = civil(timestamp);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02} UTC")
}

impl Db {
    pub(crate) fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?)
    }

    /// Sets a setting, or removes it (back to its default) with `None`.
    pub(crate) fn set_setting(&self, key: &str, value: Option<&str>) -> Result<()> {
        match value {
            Some(value) => self.conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )?,
            None => self
                .conn
                .execute("DELETE FROM settings WHERE key = ?1", [key])?,
        };
        Ok(())
    }

    /// Tells this database's backups from others in the same directory.
    pub fn database_id(&self) -> Result<String> {
        self.setting(DATABASE_ID)?
            .context("the database has no id (settings table damaged?)")
    }

    fn require_path(&self) -> Result<PathBuf> {
        self.path()
            .context("backups need a database file (this one is in memory)")
    }

    /// Next to the database, named after it: `solilog.db` → `solilog-backups/`.
    pub fn default_backup_dir(&self) -> Result<PathBuf> {
        let path = self.require_path()?;
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "solilog".into());
        Ok(path.with_file_name(format!("{stem}-backups")))
    }

    pub fn backup_dir(&self) -> Result<BackupDir> {
        Ok(match self.setting(BACKUP_DIR)? {
            Some(path) => BackupDir {
                path: path.into(),
                is_default: false,
            },
            None => BackupDir {
                path: self.default_backup_dir()?,
                is_default: true,
            },
        })
    }

    /// Makes backups go to `dir` (`None`: the default directory), creating
    /// it, and moves this database's backups there. A backup whose name is
    /// taken there stays where it was.
    pub fn set_backup_dir(&self, dir: Option<&Path>) -> Result<BackupDirChange> {
        let old = self.backup_dir()?.path;
        let default = self.default_backup_dir()?;
        let new = match dir {
            Some(dir) => std::path::absolute(dir)
                .with_context(|| format!("invalid directory {}", dir.display()))?,
            None => default.clone(),
        };
        fs::create_dir_all(&new).with_context(|| format!("creating {}", new.display()))?;
        let is_default = same_dir(&new, &default);
        let setting = if is_default {
            None
        } else {
            Some(
                new.to_str()
                    .context("the directory path must be valid UTF-8")?,
            )
        };
        self.set_setting(BACKUP_DIR, setting)?;

        let (mut moved, mut not_moved) = (0, 0);
        if !same_dir(&old, &new) {
            let id = self.database_id()?;
            for (_, from) in own_backup_files(&old, &id)? {
                let to = new.join(from.file_name().unwrap_or_default());
                if to.exists() {
                    not_moved += 1;
                    continue;
                }
                move_file(&from, &to).with_context(|| {
                    format!(
                        "backups now go to {}, but moving {} there failed",
                        new.display(),
                        from.display()
                    )
                })?;
                moved += 1;
            }
            if same_dir(&old, &default) {
                // Only removed if empty.
                let _ = fs::remove_dir(&old);
            }
        }
        Ok(BackupDirChange {
            dir: BackupDir {
                path: new,
                is_default,
            },
            moved,
            not_moved,
        })
    }

    /// Every backup in the backup directory, this database's and others',
    /// newest first. Files that aren't readable backups are left out.
    pub fn list_backups(&self) -> Result<Vec<Backup>> {
        let dir = self.backup_dir()?.path;
        let id = self.database_id()?;
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err).with_context(|| format!("reading {}", dir.display())),
        };
        let mut backups: Vec<Backup> = entries
            .filter_map(|entry| {
                let path = entry.ok()?.path();
                let name = path.file_name()?.to_str()?;
                if !name.starts_with(PREFIX) || !name.ends_with(EXTENSION) {
                    return None;
                }
                read_backup(&path, &id).ok()
            })
            .collect();
        backups.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.name.cmp(&a.name)));
        Ok(backups)
    }

    /// A backup by file name (in the backup directory) or path.
    pub fn find_backup(&self, name_or_path: &str) -> Result<Backup> {
        let given = Path::new(name_or_path);
        let in_dir = self.backup_dir()?.path.join(given);
        let path = if given.components().count() == 1 && (in_dir.exists() || !given.exists()) {
            in_dir
        } else {
            given.to_path_buf()
        };
        anyhow::ensure!(
            path.exists(),
            "no backup {name_or_path} (see the backup list)"
        );
        read_backup(&path, &self.database_id()?)
    }

    /// Backs up the database and its attachments now, then thins out older
    /// backups; returns the new one.
    pub fn create_backup(&self, reason: BackupReason) -> Result<Backup> {
        let snapshot = self.snapshot()?;
        self.write_backup(reason, &snapshot)
    }

    /// Backs up if the last backup of this database is an hour old (see
    /// [`AUTO_BACKUP_INTERVAL`]), or there is none, and the data changed
    /// since. Returns the new backup, if any.
    pub fn auto_backup(&self) -> Result<Option<Backup>> {
        self.auto_backup_after(AUTO_BACKUP_INTERVAL)
    }

    fn auto_backup_after(&self, interval: i64) -> Result<Option<Backup>> {
        let dir = self.backup_dir()?.path;
        let latest = own_backup_files(&dir, &self.database_id()?)?
            .into_iter()
            .next();
        if let Some((created_at, _)) = &latest
            && now() - created_at < interval
        {
            return Ok(None);
        }
        let snapshot = self.snapshot()?;
        if let Some((_, path)) = &latest
            && read_manifest(path).is_ok_and(|m| m.fingerprint == snapshot.fingerprint)
        {
            return Ok(None);
        }
        self.write_backup(BackupReason::Auto, &snapshot).map(Some)
    }

    /// Replaces the data with a backup's (by name or path; see
    /// [`Db::find_backup`]): the database, then the attachments. The current
    /// data is backed up first. The backup directory and database id stay.
    pub fn restore_backup(&mut self, name_or_path: &str) -> Result<Restored> {
        let backup = self.find_backup(name_or_path)?;
        let manifest = read_manifest(&backup.path)?;
        anyhow::ensure!(
            manifest.format <= FORMAT && manifest.schema_version <= SCHEMA_VERSION,
            "{} was made by a newer version of Solilog ({}); update solilog",
            backup.name,
            manifest.app_version
        );
        let attachments_dir = self.require_attachments_dir()?.to_path_buf();

        // Unpack first: nothing changes if the archive is unusable.
        let work = TempPath(sibling(&attachments_dir, "restoring"));
        remove_dir_if_present(&work.0)?;
        fs::create_dir_all(&work.0).with_context(|| format!("creating {}", work.0.display()))?;
        extract(&backup.path, &work.0).with_context(|| format!("reading {}", backup.name))?;
        let db_file = work.0.join(DATABASE);
        let check: String = Connection::open_with_flags(&db_file, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .and_then(|conn| conn.query_row("PRAGMA quick_check", [], |row| row.get(0)))
            .with_context(|| format!("checking the database in {}", backup.name))?;
        anyhow::ensure!(
            check == "ok",
            "the database in {} is damaged: {check}",
            backup.name
        );

        let before_restore = self.create_backup(BackupReason::BeforeRestore)?;
        let id = self.database_id()?;
        let backup_dir = self.setting(BACKUP_DIR)?;
        self.conn
            .restore(MAIN_DB, &db_file, None::<fn(Progress)>)
            .with_context(|| format!("restoring the database from {}", backup.name))?;
        self.conn.pragma_update(None, "journal_mode", "WAL")?;
        self.migrate()?;
        self.set_setting(DATABASE_ID, Some(&id))?;
        self.set_setting(BACKUP_DIR, backup_dir.as_deref())?;

        let old = TempPath(sibling(&attachments_dir, "old"));
        remove_dir_if_present(&old.0)?;
        let restored = work.0.join(ATTACHMENTS);
        (|| -> io::Result<()> {
            if attachments_dir.exists() {
                fs::rename(&attachments_dir, &old.0)?;
            }
            if restored.exists() {
                fs::rename(&restored, &attachments_dir)?;
            }
            Ok(())
        })()
        .with_context(|| {
            format!(
                "the database was restored but not its attachments; {} has the data as before",
                before_restore.name
            )
        })?;

        let mut restored = backup;
        restored.own = restored.database_id == id;
        Ok(Restored {
            restored,
            before_restore,
        })
    }

    /// A copy of the database in a temporary file, with what a backup
    /// needs to know about it.
    fn snapshot(&self) -> Result<Snapshot> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "solilog-snapshot-{}-{}.db",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        remove_file_if_present(&path)?;
        let file = TempPath(path);
        self.conn
            .execute("VACUUM INTO ?1", [file.0.to_string_lossy()])
            .context("copying the database")?;
        let fingerprint = format!("{:016x}", fnv1a(&fs::read(&file.0)?));
        let conn = Connection::open_with_flags(&file.0, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let issue_count: i64 = conn.query_row("SELECT count(*) FROM issues", [], |r| r.get(0))?;
        let attachments = conn
            .prepare("SELECT issue_id, name FROM attachments ORDER BY issue_id, id")?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(Snapshot {
            file,
            fingerprint,
            issue_count: issue_count as u64,
            attachments,
        })
    }

    fn write_backup(&self, reason: BackupReason, snapshot: &Snapshot) -> Result<Backup> {
        let dir = self.backup_dir()?.path;
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let id = self.database_id()?;
        // Names have the time to the second: a second backup within the
        // same second waits for the next.
        let (created_at, name, path) = loop {
            let created_at = now();
            let name = backup_name(created_at, &id);
            let path = dir.join(&name);
            if !path.exists() {
                break (created_at, name, path);
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        };

        let mut manifest = Manifest {
            format: FORMAT,
            app_version: env!("CARGO_PKG_VERSION").into(),
            schema_version: SCHEMA_VERSION,
            database_id: id.clone(),
            created_at,
            reason,
            fingerprint: snapshot.fingerprint.clone(),
            issue_count: snapshot.issue_count,
            attachment_count: 0,
        };
        // Written under another name, renamed once complete: a synced
        // folder never gets a partial backup.
        let partial = TempPath(dir.join(format!(".{name}.partial")));
        let file = File::create(&partial.0)
            .with_context(|| format!("creating {}", partial.0.display()))?;
        let mut zip = ZipWriter::new(file);
        let (y, mo, d, h, mi, s) = civil(created_at);
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .last_modified_time(
                zip::DateTime::from_date_and_time(
                    y as u16, mo as u8, d as u8, h as u8, mi as u8, s as u8,
                )
                .unwrap_or_default(),
            );
        let add = |zip: &mut ZipWriter<File>, name: &str, path: &Path| -> Result<bool> {
            let mut file = match File::open(path) {
                Ok(file) => file,
                // An attachment file deleted by hand: nothing to back up.
                Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
                Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
            };
            let size = file.metadata()?.len();
            let compression = if is_compressed(name) {
                CompressionMethod::Stored
            } else {
                CompressionMethod::Deflated
            };
            let options = options
                .compression_method(compression)
                .large_file(size >= u32::MAX as u64);
            zip.start_file(name, options)?;
            io::copy(&mut file, zip).with_context(|| format!("reading {}", path.display()))?;
            Ok(true)
        };
        add(&mut zip, DATABASE, &snapshot.file.0)?;
        if let Some(attachments_dir) = self.attachments_dir() {
            for (issue, name) in &snapshot.attachments {
                let from = attachments_dir.join(issue.to_string()).join(name);
                if add(&mut zip, &format!("{ATTACHMENTS}/{issue}/{name}"), &from)? {
                    manifest.attachment_count += 1;
                }
            }
        }
        zip.start_file(MANIFEST, options)?;
        serde_json::to_writer_pretty(&mut zip, &manifest)?;
        let file = zip.finish()?;
        file.sync_all()?;
        drop(file);
        fs::rename(&partial.0, &path).with_context(|| format!("creating {}", path.display()))?;

        // Best effort: the backup is made; thinning out runs again next time.
        let _ = prune(&dir, &id);
        let size = fs::metadata(&path)?.len();
        Ok(backup_from(name, path, size, manifest, &id))
    }
}

/// See [`Db::snapshot`]. The file is deleted on drop.
struct Snapshot {
    file: TempPath,
    fingerprint: String,
    issue_count: u64,
    attachments: Vec<(IssueId, String)>,
}

/// A file or directory deleted on drop.
struct TempPath(PathBuf);

impl Drop for TempPath {
    fn drop(&mut self) {
        if self.0.is_dir() {
            let _ = fs::remove_dir_all(&self.0);
        } else {
            let _ = fs::remove_file(&self.0);
        }
    }
}

fn read_manifest(path: &Path) -> Result<Manifest> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut zip =
        ZipArchive::new(file).with_context(|| format!("{} is not a backup", path.display()))?;
    let manifest = zip
        .by_name(MANIFEST)
        .with_context(|| format!("{} is not a backup (no {MANIFEST})", path.display()))?;
    serde_json::from_reader(manifest).with_context(|| format!("reading {}", path.display()))
}

fn read_backup(path: &Path, own_id: &str) -> Result<Backup> {
    let manifest = read_manifest(path)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let size = fs::metadata(path)?.len();
    Ok(backup_from(
        name,
        path.to_path_buf(),
        size,
        manifest,
        own_id,
    ))
}

fn backup_from(name: String, path: PathBuf, size: u64, m: Manifest, own_id: &str) -> Backup {
    Backup {
        name,
        path,
        size,
        created_at: m.created_at,
        reason: m.reason,
        own: m.database_id == own_id,
        database_id: m.database_id,
        schema_version: m.schema_version,
        app_version: m.app_version,
        issue_count: m.issue_count,
        attachment_count: m.attachment_count,
    }
}

/// Unpacks the database and attachments of a backup into `dir`.
fn extract(backup: &Path, dir: &Path) -> Result<()> {
    let mut zip = ZipArchive::new(File::open(backup)?)?;
    let mut has_database = false;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        // Paths that would leave `dir` give `None`.
        let Some(relative) = entry.enclosed_name() else {
            continue;
        };
        let wanted = relative == Path::new(DATABASE)
            || (relative.starts_with(ATTACHMENTS) && relative.components().count() == 3);
        if entry.is_dir() || !wanted {
            continue;
        }
        has_database |= relative == Path::new(DATABASE);
        let to = dir.join(&relative);
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = File::create(&to).with_context(|| format!("creating {}", to.display()))?;
        io::copy(&mut entry, &mut file)?;
    }
    anyhow::ensure!(has_database, "no {DATABASE} in it");
    Ok(())
}

/// This database's backups in `dir`, by file name, newest first.
fn own_backup_files(dir: &Path, id: &str) -> Result<Vec<(i64, PathBuf)>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err).with_context(|| format!("reading {}", dir.display())),
    };
    let mut files: Vec<(i64, PathBuf)> = entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let (created_at, file_id) = parse_backup_name(path.file_name()?.to_str()?)?;
            (file_id == id).then_some((created_at, path))
        })
        .collect();
    files.sort_by(|a, b| b.cmp(a));
    Ok(files)
}

/// Deletes the backups of database `id` in `dir` that [`to_prune`] picks.
fn prune(dir: &Path, id: &str) -> Result<()> {
    let files = own_backup_files(dir, id)?;
    let times: Vec<i64> = files.iter().map(|(t, _)| *t).collect();
    for i in to_prune(&times, now()) {
        remove_file_if_present(&files[i].1)?;
    }
    Ok(())
}

/// Which backups to delete, given their times (newest first): indices into
/// `times`. Kept: every backup of the last day, then the newest of each day
/// for two weeks, of each week for three months, and of each month for a
/// year. The newest backup is always kept.
fn to_prune(times: &[i64], now: i64) -> Vec<usize> {
    let mut buckets = HashSet::new();
    let mut prune = Vec::new();
    for (i, &time) in times.iter().enumerate() {
        let age = now - time;
        let bucket = if i == 0 || age < DAY {
            continue;
        } else if age < 14 * DAY {
            (0, time.div_euclid(DAY))
        } else if age < 90 * DAY {
            (1, time.div_euclid(7 * DAY))
        } else if age < 365 * DAY {
            let (y, m, ..) = civil(time);
            (2, y * 12 + m)
        } else {
            prune.push(i);
            continue;
        };
        if !buckets.insert(bucket) {
            prune.push(i);
        }
    }
    prune
}

fn backup_name(created_at: i64, id: &str) -> String {
    let (y, mo, d, h, mi, s) = civil(created_at);
    format!("{PREFIX}{y:04}-{mo:02}-{d:02}-{h:02}{mi:02}{s:02}-{id}{EXTENSION}")
}

/// The time and database id in a backup's file name.
fn parse_backup_name(name: &str) -> Option<(i64, &str)> {
    let rest = name.strip_prefix(PREFIX)?.strip_suffix(EXTENSION)?;
    let mut parts = rest.splitn(5, '-');
    let mut number = |len: usize| -> Option<i64> {
        let part = parts.next()?;
        (part.len() == len && part.bytes().all(|b| b.is_ascii_digit()))
            .then(|| part.parse().ok())?
    };
    let (y, mo, d, time) = (number(4)?, number(2)?, number(2)?, number(6)?);
    let id = parts.next()?;
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    let (h, mi, s) = (time / 10000, time / 100 % 100, time % 100);
    Some((days_from_civil(y, mo, d) * DAY + h * 3600 + mi * 60 + s, id))
}

/// Unix time → (year, month, day, hour, minute, second), UTC.
fn civil(timestamp: i64) -> (i64, i64, i64, i64, i64, i64) {
    // Howard Hinnant's `civil_from_days`.
    let z = timestamp.div_euclid(DAY) + 719_468;
    let secs = timestamp.rem_euclid(DAY);
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d, secs / 3600, secs % 3600 / 60, secs % 60)
}

/// Days since 1970-01-01 of a date (Howard Hinnant's `days_from_civil`).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// 64-bit FNV-1a: stable across builds, unlike `std`'s hasher.
fn fnv1a(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn is_compressed(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| {
            COMPRESSED_EXTENSIONS
                .iter()
                .any(|c| c.eq_ignore_ascii_case(e))
        })
}

/// `…/solilog-attachments` → `…/.solilog-attachments.<suffix>`.
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!(".{name}.{suffix}"))
}

/// Whether two paths are the same directory (`false` if either is missing).
fn same_dir(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Moves a file, across file systems too (copied, then deleted).
fn move_file(from: &Path, to: &Path) -> Result<()> {
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    let partial = TempPath(sibling(to, "partial"));
    fs::copy(from, &partial.0)?;
    fs::rename(&partial.0, to)?;
    fs::remove_file(from)?;
    Ok(())
}

fn remove_file_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => {
            Err(err).with_context(|| format!("deleting {}", path.display()))
        }
        _ => Ok(()),
    }
}

fn remove_dir_if_present(path: &Path) -> Result<()> {
    match fs::remove_dir_all(path) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => {
            Err(err).with_context(|| format!("deleting {}", path.display()))
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
            let dir =
                std::env::temp_dir().join(format!("solilog-backup-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            // As SQLite reports it (`/var` is a link to `/private/var` on macOS).
            let dir = fs::canonicalize(dir).unwrap();
            let db = Db::open(&dir.join("test.db")).unwrap();
            Self { dir, db }
        }
    }

    impl Drop for TempDb {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    fn titles(db: &Db) -> Vec<String> {
        let issues = db.list_issues(&IssueFilter::default()).unwrap();
        issues.into_iter().map(|i| i.title).collect()
    }

    #[test]
    fn dates() {
        assert_eq!(civil(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(civil(1_700_000_000), (2023, 11, 14, 22, 13, 20));
        assert_eq!(civil(951_782_400), (2000, 2, 29, 0, 0, 0));
        assert_eq!(format_utc(1_700_000_000), "2023-11-14 22:13 UTC");
        let name = backup_name(1_700_000_000, "a1b2c3d4");
        assert_eq!(name, "solilog-2023-11-14-221320-a1b2c3d4.zip");
        assert_eq!(parse_backup_name(&name), Some((1_700_000_000, "a1b2c3d4")));
        for bad in [
            "solilog-2023-11-14-221320-.zip",
            "solilog-2023-11-14-2213-a1b2.zip",
            "solilog-2023-11-14-221320-a1b2.zip.partial",
            ".solilog-2023-11-14-221320-a1b2.zip.partial",
            "notes.zip",
        ] {
            assert_eq!(parse_backup_name(bad), None, "{bad}");
        }
    }

    #[test]
    fn thinning_out() {
        let now = 1000 * DAY;
        let hour = 3600;
        // Hourly backups for 400 days.
        let times: Vec<i64> = (0..400 * 24).map(|h| now - h * hour).collect();
        let pruned: HashSet<usize> = to_prune(&times, now).into_iter().collect();
        let kept: Vec<i64> = (0..times.len())
            .filter(|i| !pruned.contains(i))
            .map(|i| now - times[i])
            .collect();
        let count = |from: i64, to: i64| kept.iter().filter(|a| (from..to).contains(*a)).count();
        assert_eq!(count(0, DAY), 24);
        // A day per calendar date (UTC).
        assert!((13..=14).contains(&count(DAY, 14 * DAY)));
        assert!((10..=13).contains(&count(14 * DAY, 90 * DAY)));
        assert!((8..=10).contains(&count(90 * DAY, 365 * DAY)));
        assert_eq!(count(365 * DAY, i64::MAX), 0);
        assert!(kept.len() < 70);

        // The newest is kept however old.
        assert_eq!(to_prune(&[now - 500 * DAY, now - 600 * DAY], now), [1]);
    }

    #[test]
    fn backup_and_restore() {
        let mut t = TempDb::new("restore");
        let src = t.dir.join("notes.txt");
        fs::write(&src, "first").unwrap();
        let kept = t.db.create_issue(&NewIssue::new("kept")).unwrap();
        t.db.add_attachments(kept.id, &[src]).unwrap();
        let first = t.db.create_backup(BackupReason::Manual).unwrap();
        assert_eq!((first.issue_count, first.attachment_count), (1, 1));
        assert!(first.own);
        assert_eq!(
            first.path.parent(),
            Some(t.dir.join("test-backups").as_path())
        );

        // Unchanged: not backed up again, even when due.
        assert!(t.db.auto_backup_after(0).unwrap().is_none());
        // Not due yet.
        t.db.create_issue(&NewIssue::new("later")).unwrap();
        assert!(t.db.auto_backup().unwrap().is_none());

        t.db.delete_issue(kept.id).unwrap();
        let restored = t.db.restore_backup(&first.name).unwrap();
        assert_eq!(restored.restored.name, first.name);
        assert_eq!(restored.before_restore.reason, BackupReason::BeforeRestore);
        assert_eq!(restored.before_restore.issue_count, 1);
        assert_eq!(titles(&t.db), ["kept"]);
        let attachment = &t.db.get_issue(kept.id).unwrap().attachments[0];
        assert_eq!(
            fs::read_to_string(attachment.path.as_ref().unwrap()).unwrap(),
            "first"
        );

        // And back.
        t.db.restore_backup(&restored.before_restore.path.to_string_lossy())
            .unwrap();
        assert_eq!(titles(&t.db), ["later"]);
        assert!(!t.db.attachments_dir().unwrap().join("SOL-1").exists());

        let backups = t.db.list_backups().unwrap();
        assert_eq!(backups.len(), 3);
        assert!(backups.iter().all(|b| b.own));
    }

    #[test]
    fn restore_keeps_id_and_directory() {
        let mut t = TempDb::new("settings");
        let backup = t.db.create_backup(BackupReason::Manual).unwrap();
        let id = t.db.database_id().unwrap();
        let elsewhere = t.dir.join("elsewhere");
        t.db.set_backup_dir(Some(&elsewhere)).unwrap();
        t.db.set_setting(DATABASE_ID, Some("changed")).unwrap();
        let restored = t.db.restore_backup(&backup.name).unwrap();
        assert_eq!(restored.restored.database_id, id);
        assert!(!restored.restored.own);
        assert_eq!(t.db.database_id().unwrap(), "changed");
        assert_eq!(t.db.backup_dir().unwrap().path, elsewhere);
    }

    #[test]
    fn changing_directory_moves_own_backups() {
        let t = TempDb::new("move");
        let backup = t.db.create_backup(BackupReason::Manual).unwrap();
        let default = t.db.backup_dir().unwrap();
        assert!(default.is_default);
        let foreign = default.path.join("solilog-2023-11-14-221320-0ther.zip");
        fs::copy(&backup.path, &foreign).unwrap();

        let dropbox = t.dir.join("Dropbox").join("backups");
        let change = t.db.set_backup_dir(Some(&dropbox)).unwrap();
        assert_eq!((change.moved, change.not_moved), (1, 0));
        assert!(!change.dir.is_default);
        assert!(dropbox.join(&backup.name).exists());
        assert!(foreign.exists(), "other databases' backups stay");
        let listed = t.db.list_backups().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].path, dropbox.join(&backup.name));

        // Back to the default, which is removed when left empty.
        fs::remove_file(&foreign).unwrap();
        let change = t.db.set_backup_dir(Some(&dropbox)).unwrap();
        assert_eq!(change.moved, 0);
        let change = t.db.set_backup_dir(None).unwrap();
        assert!(change.dir.is_default);
        assert_eq!(change.moved, 1);
        let change = t.db.set_backup_dir(Some(&dropbox)).unwrap();
        assert_eq!(change.moved, 1);
        assert!(!default.path.exists());
    }

    #[test]
    fn in_memory_databases_have_none() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.create_backup(BackupReason::Manual).is_err());
        assert!(db.backup_dir().is_err());
    }
}
