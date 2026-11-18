//! Databases made by each release (`tests/fixtures/v*.db`, made with
//! `scripts/upgrade-fixture.sh` when the release is cut) open with the
//! current code, and their data survives the migrations.
//!
//! Every fixture has the same data; [`check_data`] is what the script puts
//! in. When the script adds to it for a new feature, check the new data
//! only in the fixtures that have it.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use solilog_core::{
    BackupReason, Db, IssueFilter, IssueId, LabelColor, ProjectStatus, RevisionField, Status,
};

fn fixtures() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut fixtures: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|e| e == "db"))
        .collect();
    fixtures.sort();
    fixtures
}

fn user_version(path: &Path) -> i64 {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    conn.pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap()
}

#[test]
fn every_release_upgrades() {
    let fixtures = fixtures();
    assert!(!fixtures.is_empty(), "no fixtures");
    // The schema version of a new database.
    let current = {
        let dir = temp_dir("new");
        Db::open(&dir.join("new.db")).unwrap();
        let version = user_version(&dir.join("new.db"));
        fs::remove_dir_all(&dir).unwrap();
        version
    };
    for fixture in fixtures {
        let name = fixture.file_stem().unwrap().to_string_lossy().into_owned();
        let dir = temp_dir(&name);
        let path = dir.join("solilog.db");
        fs::copy(&fixture, &path).unwrap();
        let before = user_version(&path);
        assert!(before <= current, "{name}: newer than this build");

        let db = Db::open(&path).unwrap_or_else(|e| panic!("{name}: {e:#}"));
        check_data(&db);
        if before >= REVISIONS_SCHEMA {
            check_revisions(&db);
        }
        if before >= DEPENDENCIES_SCHEMA {
            check_dependencies(&db);
        }
        assert_eq!(user_version(&path), current, "{name}: not upgraded");
        let upgrade_backups = db
            .list_backups()
            .unwrap()
            .into_iter()
            .filter(|b| b.reason == BackupReason::BeforeUpgrade)
            .count();
        let expected = usize::from(before < current);
        assert_eq!(upgrade_backups, expected, "{name}: backups before upgrade");
        drop(db);

        let conn = Connection::open(&path).unwrap();
        let check: String = conn
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(check, "ok", "{name}");
        let broken: i64 = conn
            .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(broken, 0, "{name}: rows pointing nowhere");
        drop(conn);
        fs::remove_dir_all(&dir).unwrap();
    }
}

/// What `scripts/upgrade-fixture.sh` puts in.
fn check_data(db: &Db) {
    let issues = db.list_issues(&IssueFilter::default()).unwrap();
    let titles: Vec<&str> = issues.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(titles, ["Fix the header layout", "Write the launch post"]);

    let post = db.get_issue(IssueId(1)).unwrap();
    assert_eq!(post.status, Status::InProgress);
    assert_eq!(post.description, "Two paragraphs.\n\nMention SOL-2.");
    assert_eq!(post.labels, ["docs", "web"]);
    assert_eq!(post.projects, ["Website v2"]);
    let checklist: Vec<(&str, bool)> = post
        .checklist
        .iter()
        .map(|item| (item.text.as_str(), item.done))
        .collect();
    assert_eq!(checklist, [("Draft", true), ("Review", false)]);

    let header = db.get_issue(IssueId(2)).unwrap();
    assert_eq!(header.status, Status::Blocked);
    assert_eq!(header.labels, ["web/frontend"]);
    assert_eq!(header.attachments.len(), 1);
    assert_eq!(header.attachments[0].name, "notes.txt");
    assert_eq!(header.attachments[0].size, 13);

    let archived = db.list_archived_issues(&IssueFilter::default()).unwrap();
    assert_eq!(archived.len(), 1);
    assert_eq!(archived[0].title, "Old idea");
    assert_eq!(archived[0].status, Status::Cancelled);

    let web = db.get_label("web").unwrap();
    assert_eq!(web.color, Some(LabelColor::Teal));
    assert!(web.collapsed);
    assert_eq!(
        db.get_label("web/frontend").unwrap().display_color,
        Some(LabelColor::Teal)
    );

    let site = db.get_project("Website v2").unwrap();
    assert_eq!(site.status, ProjectStatus::Active);
    assert_eq!(site.description, "Launch the new site");
    assert_eq!(site.labels, ["web"]);
    assert_eq!(site.issue_count, 1);
    assert_eq!(
        db.get_project("Old site").unwrap().status,
        ProjectStatus::Done
    );

    let view = db.get_view("Open web").unwrap();
    assert!(view.filter.open_only);
    assert_eq!(view.filter.labels, ["web"]);
    let in_view = db.list_issues(&view.filter).unwrap();
    assert_eq!(in_view.len(), 2);
}

/// The first schema version with revisions.
const REVISIONS_SCHEMA: i64 = 17;

/// What `scripts/upgrade-fixture.sh` puts in since revisions exist.
fn check_revisions(db: &Db) {
    let revisions = db.issue_revisions(IssueId(1)).unwrap();
    let revisions: Vec<(RevisionField, Option<i64>, &str, bool)> = revisions
        .iter()
        .map(|r| {
            (
                r.field,
                r.checklist_item_id.map(|i| i.0),
                r.text.as_str(),
                r.item_deleted,
            )
        })
        .collect();
    assert_eq!(
        revisions,
        [
            (RevisionField::ChecklistItem, Some(3), "Proofread", true),
            (RevisionField::Title, None, "Write the post", false),
        ]
    );
    // A new item doesn't take the deleted one's id.
    let issue = db
        .add_checklist_items(IssueId(1), &["Publish".into()])
        .unwrap();
    assert!(issue.checklist.iter().all(|i| i.id.0 != 3));
}

/// The first schema version with dependencies.
const DEPENDENCIES_SCHEMA: i64 = 18;

/// What `scripts/upgrade-fixture.sh` puts in since dependencies exist.
fn check_dependencies(db: &Db) {
    let post = db.get_issue(IssueId(1)).unwrap();
    assert_eq!(post.depends_on, [IssueId(2)]);
    assert_eq!(post.waiting_on, [IssueId(2)]);
    assert_eq!(db.get_issue(IssueId(2)).unwrap().dependents, [IssueId(1)]);
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("solilog-upgrade-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}
