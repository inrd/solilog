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
    BackupReason, Check, Db, IssueFilter, IssueId, LabelColor, NoteKind, PathOwnerKind,
    ProjectStatus, Receipt, RevisionField, Status, Verdict,
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
        if before >= ACTIVITY_SCHEMA {
            check_activity(&db);
        }
        if before >= NOTES_SCHEMA {
            check_notes(&db);
        }
        if before >= REVIEW_SCHEMA {
            check_review(&db);
        }
        if before >= PATHS_SCHEMA {
            check_paths(&db);
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
    // Issues added for later features come after these.
    let titles: Vec<&str> = issues
        .iter()
        .filter(|i| i.id.0 <= 3)
        .map(|i| i.title.as_str())
        .collect();
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

/// The first schema version with authorship, activity log and review tracking.
const ACTIVITY_SCHEMA: i64 = 19;

/// What `scripts/upgrade-fixture.sh` puts in since activity tracking exists.
fn check_activity(db: &Db) {
    let post = db.get_issue(IssueId(1)).unwrap();
    assert!(!post.unreviewed);
    assert!(post.reviewed_at.is_some());
    assert!(!db.issue_activities(IssueId(1)).unwrap().is_empty());

    let header = db.get_issue(IssueId(2)).unwrap();
    assert!(header.unreviewed);
    assert_eq!(header.description_by.as_str(), "claude:fix-header");
    assert_eq!(
        header
            .unreviewed_actors
            .iter()
            .map(|a| a.as_str())
            .collect::<Vec<_>>(),
        ["claude:fix-header"]
    );
}

/// The first schema version with notes.
const NOTES_SCHEMA: i64 = 20;

/// What `scripts/upgrade-fixture.sh` puts in since notes exist.
fn check_notes(db: &Db) {
    let notes = db.issue_notes(IssueId(2)).unwrap();
    assert!(notes.iter().all(|n| n.kind == NoteKind::Note));
    let notes: Vec<(&str, &str)> = notes
        .iter()
        .map(|n| (n.author.as_str(), n.text.as_str()))
        .collect();
    assert_eq!(
        notes,
        [
            ("user", "Check it on a phone too."),
            (
                "claude:fix-header",
                "Tried flex-wrap.\nThe logo still overflows."
            ),
        ]
    );
}

/// The first schema version with the review handoff.
const REVIEW_SCHEMA: i64 = 21;

/// What `scripts/upgrade-fixture.sh` puts in since the review handoff exists.
fn check_review(db: &Db) {
    let footer = db.get_issue(IssueId(4)).unwrap();
    assert_eq!(footer.status, Status::InReview);
    assert!(footer.unreviewed);
    let notes = db.issue_notes(IssueId(4)).unwrap();
    let notes: Vec<(&str, &str, &NoteKind)> = notes
        .iter()
        .map(|n| (n.author.as_str(), n.text.as_str(), &n.kind))
        .collect();
    let check = |command: &str, passed| Check {
        command: command.into(),
        passed,
    };
    assert_eq!(
        notes,
        [
            (
                "claude:footer",
                "Footer updated.",
                &NoteKind::Receipt(Receipt {
                    checks: vec![check("make test", true), check("make lint", false)],
                    commits: vec!["abc1234".into()],
                    files: vec!["footer.html".into()],
                })
            ),
            (
                "user",
                "Fix the lint first.",
                &NoteKind::Review {
                    verdict: Verdict::ChangesRequested
                }
            ),
            (
                "claude:footer",
                "Lint fixed.",
                &NoteKind::Receipt(Receipt {
                    checks: vec![check("make lint", true)],
                    ..Default::default()
                })
            ),
        ]
    );
}

/// Mapped paths: `user_version` from migration 22 on.
const PATHS_SCHEMA: i64 = 22;

fn check_paths(db: &Db) {
    let paths: Vec<(PathOwnerKind, String, String)> = db
        .list_paths()
        .unwrap()
        .into_iter()
        .map(|m| (m.kind, m.name, m.path))
        .collect();
    assert_eq!(
        paths,
        [
            (PathOwnerKind::Label, "web".into(), "~/code/website".into()),
            (
                PathOwnerKind::Project,
                "Website v2".into(),
                "/srv/solilog-fixture/website-v2".into()
            ),
        ]
    );
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("solilog-upgrade-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}
