//! Revisions: earlier text of an issue's title, description and checklist
//! items, to restore it when an edit (by hand, or by an agent) overwrote it.
//!
//! Before one of these is overwritten, or a checklist item deleted, the old
//! text is saved as a revision: a full copy, not a diff (issue text is
//! small). Edits are coalesced: the GUI saves while typing, so a field gets
//! no new revision while its last one is under [`COALESCE`] old, except
//! before a destructive edit ([`is_destructive`]). Every new revision thins
//! out the field's older ones ([`to_thin`]). Restoring saves the current
//! text first, so a restore can be undone. Deleting an issue deletes its
//! revisions; archiving keeps them.

use std::fmt;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use rusqlite::types::{FromSql, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use rusqlite::{OptionalExtension as _, Row, params};
use serde::{Deserialize, Serialize};

use crate::Db;
use crate::checklist::ChecklistItemId;
use crate::issue::{Issue, IssueId};
use crate::order::Ordered;
use crate::thinning::{DAY, HOUR, Keep, thin_out};

/// A field gets no new revision while its last one is younger than this
/// (seconds), unless the edit is destructive.
const COALESCE: i64 = 10 * 60;

/// The most revisions kept per field.
const MAX_PER_FIELD: usize = 50;

/// Identifies a revision, across all issues. Displayed as a plain number;
/// parsed from `12` or `#12`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RevisionId(pub i64);

impl fmt::Display for RevisionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for RevisionId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        s.strip_prefix('#')
            .unwrap_or(s)
            .parse::<i64>()
            .ok()
            .filter(|n| *n > 0)
            .map(RevisionId)
            .ok_or_else(|| format!("invalid revision id {s:?} (expected a number)"))
    }
}

impl ToSql for RevisionId {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        self.0.to_sql()
    }
}

impl FromSql for RevisionId {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        i64::column_result(value).map(RevisionId)
    }
}

/// Which text of an issue a revision is of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RevisionField {
    Title,
    Description,
    ChecklistItem,
}

impl RevisionField {
    pub fn key(self) -> &'static str {
        match self {
            RevisionField::Title => "title",
            RevisionField::Description => "description",
            RevisionField::ChecklistItem => "checklist_item",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            RevisionField::Title => "Title",
            RevisionField::Description => "Description",
            RevisionField::ChecklistItem => "Checklist item",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        [Self::Title, Self::Description, Self::ChecklistItem]
            .into_iter()
            .find(|f| f.key() == key)
    }
}

/// Why a revision was saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RevisionReason {
    /// The text was about to be edited.
    Edit,
    /// The checklist item was about to be deleted.
    Delete,
    /// Another revision was about to be restored over it.
    Restore,
}

impl RevisionReason {
    pub fn key(self) -> &'static str {
        match self {
            RevisionReason::Edit => "edit",
            RevisionReason::Delete => "delete",
            RevisionReason::Restore => "restore",
        }
    }

    /// What the text was: "before edit", "deleted", "before restore".
    pub fn label(self) -> &'static str {
        match self {
            RevisionReason::Edit => "before edit",
            RevisionReason::Delete => "deleted",
            RevisionReason::Restore => "before restore",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        [Self::Edit, Self::Delete, Self::Restore]
            .into_iter()
            .find(|r| r.key() == key)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Revision {
    pub id: RevisionId,
    pub issue_id: IssueId,
    pub field: RevisionField,
    /// The checklist item, for a `checklist_item` revision.
    pub checklist_item_id: Option<ChecklistItemId>,
    /// Whether that checklist item no longer exists: restoring the
    /// revision brings it back.
    pub item_deleted: bool,
    /// The text as it was.
    pub text: String,
    pub reason: RevisionReason,
    /// Unix time, seconds.
    pub created_at: i64,
}

impl Revision {
    /// "Title", "Checklist item 41", "Checklist item 41 (deleted)".
    pub fn field_label(&self) -> String {
        match self.checklist_item_id {
            Some(item) if self.item_deleted => format!("{} {item} (deleted)", self.field.label()),
            Some(item) => format!("{} {item}", self.field.label()),
            None => self.field.label().to_owned(),
        }
    }

    /// The text on one line, cut to `max_chars` (with `…`).
    pub fn preview(&self, max_chars: usize) -> String {
        let text = self.text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.chars().count() <= max_chars {
            return text;
        }
        let cut: String = text.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{}…", cut.trim_end())
    }
}

/// How long ago `timestamp` was: "just now", "5 min ago", "3 hours ago",
/// "2 days ago".
pub fn format_age(timestamp: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    age_text(now - timestamp)
}

fn age_text(seconds: i64) -> String {
    let plural = |n: i64, unit: &str| {
        let s = if n == 1 { "" } else { "s" };
        format!("{n} {unit}{s} ago")
    };
    if seconds < 60 {
        "just now".to_owned()
    } else if seconds < HOUR {
        format!("{} min ago", seconds / 60)
    } else if seconds < DAY {
        plural(seconds / HOUR, "hour")
    } else {
        plural(seconds / DAY, "day")
    }
}

/// Which text a revision is of, with the checklist item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Field {
    Title,
    Description,
    ChecklistItem(ChecklistItemId),
}

impl Field {
    fn of(revision: &Revision) -> Self {
        match (revision.field, revision.checklist_item_id) {
            (RevisionField::Title, _) => Field::Title,
            (RevisionField::Description, _) => Field::Description,
            (RevisionField::ChecklistItem, item) => {
                Field::ChecklistItem(item.unwrap_or(ChecklistItemId(0)))
            }
        }
    }

    fn key(self) -> &'static str {
        match self {
            Field::Title => RevisionField::Title.key(),
            Field::Description => RevisionField::Description.key(),
            Field::ChecklistItem(_) => RevisionField::ChecklistItem.key(),
        }
    }

    fn item(self) -> Option<ChecklistItemId> {
        match self {
            Field::ChecklistItem(item) => Some(item),
            Field::Title | Field::Description => None,
        }
    }
}

/// Whether replacing `old` with `new` loses much of it: more than half its
/// length, or over 200 characters. Such an edit always saves a revision.
fn is_destructive(old: &str, new: &str) -> bool {
    let (old, new) = (old.chars().count(), new.chars().count());
    new * 2 < old || old.saturating_sub(new) > 200
}

/// Which revisions of a field to delete, given their times (newest first):
/// indices into `times`, ascending. Kept: every revision of the last day,
/// then the newest of each hour for a week, then of each day; at most
/// [`MAX_PER_FIELD`] (the newest).
fn to_thin(times: &[i64], now: i64) -> Vec<usize> {
    let mut prune = thin_out(times.iter().map(|&time| {
        let age = now - time;
        if age < DAY {
            Keep::Always
        } else if age < 7 * DAY {
            Keep::OnePer((0, time.div_euclid(HOUR)))
        } else {
            Keep::OnePer((1, time.div_euclid(DAY)))
        }
    }));
    let kept: Vec<usize> = (0..times.len())
        .filter(|i| prune.binary_search(i).is_err())
        .collect();
    prune.extend(kept.into_iter().skip(MAX_PER_FIELD));
    prune.sort_unstable();
    prune
}

const REVISION_COLUMNS: &str = "r.id, r.issue_id, r.field, r.checklist_item_id, r.text, r.reason,
     r.created_at, r.position,
     r.checklist_item_id IS NOT NULL
         AND NOT EXISTS (SELECT 1 FROM checklist_items c WHERE c.id = r.checklist_item_id)";

/// A revision, and where its checklist item was when deleted.
fn revision_from_row(row: &Row) -> rusqlite::Result<(Revision, Option<i64>)> {
    let invalid = |i: usize, what: &str, key: String| {
        rusqlite::Error::FromSqlConversionFailure(
            i,
            rusqlite::types::Type::Text,
            format!("unknown revision {what} {key:?}").into(),
        )
    };
    let field: String = row.get(2)?;
    let reason: String = row.get(5)?;
    Ok((
        Revision {
            id: row.get(0)?,
            issue_id: row.get(1)?,
            field: RevisionField::from_key(&field).ok_or_else(|| invalid(2, "field", field))?,
            checklist_item_id: row.get(3)?,
            text: row.get(4)?,
            reason: RevisionReason::from_key(&reason)
                .ok_or_else(|| invalid(5, "reason", reason))?,
            created_at: row.get(6)?,
            item_deleted: row.get(8)?,
        },
        row.get(7)?,
    ))
}

impl Db {
    /// An issue's revisions, newest first.
    pub fn issue_revisions(&self, issue: IssueId) -> Result<Vec<Revision>> {
        self.get_issue(issue)?;
        Ok(self
            .conn
            .prepare_cached(&format!(
                "SELECT {REVISION_COLUMNS} FROM revisions r
                 WHERE r.issue_id = ?1 ORDER BY r.created_at DESC, r.id DESC"
            ))?
            .query_map([issue], revision_from_row)?
            .map(|row| row.map(|(revision, _)| revision))
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn get_revision(&self, id: RevisionId) -> Result<Revision> {
        Ok(self.revision_and_position(id)?.0)
    }

    fn revision_and_position(&self, id: RevisionId) -> Result<(Revision, Option<i64>)> {
        self.conn
            .query_row(
                &format!("SELECT {REVISION_COLUMNS} FROM revisions r WHERE r.id = ?1"),
                [id],
                revision_from_row,
            )
            .optional()?
            .with_context(|| format!("no revision {id}"))
    }

    /// Puts a revision's text back in place, and returns the issue. The
    /// current text is saved as a revision first, so this can be undone. A
    /// deleted checklist item comes back, unchecked, where it was (or last).
    pub fn restore_revision(&self, id: RevisionId) -> Result<Issue> {
        let (revision, position) = self.revision_and_position(id)?;
        let field = Field::of(&revision);
        let tx = self.conn.unchecked_transaction()?;
        let issue = self.get_issue(revision.issue_id)?;
        let current = match field {
            Field::Title => Some(issue.title.clone()),
            Field::Description => Some(issue.description.clone()),
            Field::ChecklistItem(item) => issue
                .checklist
                .iter()
                .find(|i| i.id == item)
                .map(|i| i.text.clone()),
        };
        match (field, current) {
            (_, Some(current)) if current == revision.text => {}
            (field, Some(current)) => {
                if !current.is_empty() {
                    self.save_revision(issue.id, field, &current, RevisionReason::Restore, None)?;
                }
                match field {
                    Field::Title => self.set_issue_column(issue.id, "title", &revision.text)?,
                    Field::Description => {
                        self.set_issue_column(issue.id, "description", &revision.text)?;
                    }
                    Field::ChecklistItem(item) => {
                        self.conn.execute(
                            "UPDATE checklist_items SET text = ?2 WHERE id = ?1",
                            params![item, revision.text],
                        )?;
                    }
                }
            }
            (Field::ChecklistItem(item), None) => {
                self.conn.execute(
                    "INSERT INTO checklist_items (id, issue_id, text, position)
                     VALUES (?1, ?2, ?3, 0)",
                    params![item, issue.id, revision.text],
                )?;
                let mut order = self.checklist_order(issue.id)?;
                order.retain(|&i| i != item.0);
                let at = position.map_or(order.len(), |p| (p.max(0) as usize).min(order.len()));
                order.insert(at, item.0);
                self.renumber(Ordered::ChecklistItems, &order)?;
            }
            (Field::Title | Field::Description, None) => unreachable!("issues have both"),
        }
        tx.commit()?;
        self.get_issue(issue.id)
    }

    /// Before `old` is replaced with `new`: saves it as a revision, unless
    /// the field has a recent one and the edit isn't destructive. Call
    /// inside a transaction.
    pub(crate) fn record_edit(
        &self,
        issue: IssueId,
        field: Field,
        old: &str,
        new: &str,
    ) -> Result<()> {
        if old == new || old.is_empty() {
            return Ok(());
        }
        if !is_destructive(old, new) && self.has_recent_revision(issue, field)? {
            return Ok(());
        }
        self.save_revision(issue, field, old, RevisionReason::Edit, None)
    }

    /// Before a checklist item at `position` (0-based) is deleted: saves its
    /// text as a revision. Call inside a transaction.
    pub(crate) fn record_delete(
        &self,
        issue: IssueId,
        item: ChecklistItemId,
        text: &str,
        position: usize,
    ) -> Result<()> {
        let field = Field::ChecklistItem(item);
        self.save_revision(issue, field, text, RevisionReason::Delete, Some(position))
    }

    fn has_recent_revision(&self, issue: IssueId, field: Field) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM revisions
                 WHERE issue_id = ?1 AND field = ?2 AND checklist_item_id IS ?3
                     AND created_at > unixepoch() - ?4)",
            params![issue, field.key(), field.item(), COALESCE],
            |row| row.get(0),
        )?)
    }

    /// Saves a revision and thins out the field's older ones.
    fn save_revision(
        &self,
        issue: IssueId,
        field: Field,
        text: &str,
        reason: RevisionReason,
        position: Option<usize>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO revisions (issue_id, field, checklist_item_id, text, reason, position)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                issue,
                field.key(),
                field.item(),
                text,
                reason.key(),
                position.map(|p| p as i64)
            ],
        )?;
        self.thin_revisions(issue, field)
    }

    fn thin_revisions(&self, issue: IssueId, field: Field) -> Result<()> {
        let revisions: Vec<(i64, i64)> = self
            .conn
            .prepare_cached(
                "SELECT id, created_at FROM revisions
                 WHERE issue_id = ?1 AND field = ?2 AND checklist_item_id IS ?3
                 ORDER BY created_at DESC, id DESC",
            )?
            .query_map(params![issue, field.key(), field.item()], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let now: i64 = self
            .conn
            .query_row("SELECT unixepoch()", [], |row| row.get(0))?;
        let times: Vec<i64> = revisions.iter().map(|&(_, time)| time).collect();
        let mut delete = self
            .conn
            .prepare_cached("DELETE FROM revisions WHERE id = ?1")?;
        for i in to_thin(&times, now) {
            delete.execute([revisions[i].0])?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IssueEdit, NewIssue};

    fn texts(db: &Db, issue: IssueId) -> Vec<(String, String)> {
        db.issue_revisions(issue)
            .unwrap()
            .into_iter()
            .map(|r| (r.field_label(), r.text))
            .collect()
    }

    fn edit_description(db: &Db, issue: IssueId, text: &str) -> Issue {
        let edit = IssueEdit {
            description: Some(text.into()),
            ..Default::default()
        };
        db.edit_issue(issue, &edit).unwrap()
    }

    /// Makes every revision `seconds` older.
    fn age(db: &Db, seconds: i64) {
        db.conn
            .execute(
                "UPDATE revisions SET created_at = created_at - ?1",
                [seconds],
            )
            .unwrap();
    }

    #[test]
    fn ids_and_labels() {
        assert_eq!("12".parse(), Ok(RevisionId(12)));
        assert_eq!("#3".parse(), Ok(RevisionId(3)));
        assert!("0".parse::<RevisionId>().is_err());
        assert_eq!(age_text(5), "just now");
        assert_eq!(age_text(5 * 60), "5 min ago");
        assert_eq!(age_text(HOUR), "1 hour ago");
        assert_eq!(age_text(3 * DAY), "3 days ago");
    }

    #[test]
    fn edits_are_coalesced() {
        let db = Db::open_in_memory().unwrap();
        let id = db
            .create_issue(&NewIssue::new("x").description("one"))
            .unwrap()
            .id;
        // An empty description isn't worth a revision.
        let plain = db.create_issue(&NewIssue::new("plain")).unwrap().id;
        edit_description(&db, plain, "new");
        assert!(texts(&db, plain).is_empty());

        edit_description(&db, id, "one two");
        edit_description(&db, id, "one two three");
        // Unchanged: nothing to save.
        edit_description(&db, id, "one two three");
        assert_eq!(texts(&db, id), [("Description".into(), "one".into())]);
        // Ten minutes later, the next edit saves one.
        age(&db, COALESCE);
        edit_description(&db, id, "one two three four");
        assert_eq!(
            texts(&db, id),
            [
                ("Description".into(), "one two three".into()),
                ("Description".into(), "one".into())
            ]
        );
        // Each field on its own.
        let edit = IssueEdit {
            title: Some("y".into()),
            ..Default::default()
        };
        db.edit_issue(id, &edit).unwrap();
        assert_eq!(texts(&db, id)[0], ("Title".into(), "x".into()));
    }

    #[test]
    fn destructive_edits_always_save() {
        let db = Db::open_in_memory().unwrap();
        let long = "word ".repeat(100);
        let id = db
            .create_issue(&NewIssue::new("x").description("short"))
            .unwrap()
            .id;
        edit_description(&db, id, &long);
        assert_eq!(db.issue_revisions(id).unwrap().len(), 1);
        // Losing over 200 characters (but not half).
        let shorter = "word ".repeat(55);
        edit_description(&db, id, &shorter);
        assert_eq!(db.issue_revisions(id).unwrap().len(), 2);
        // Losing more than half (fewer than 200 characters).
        edit_description(&db, id, "word word");
        assert_eq!(db.issue_revisions(id).unwrap().len(), 3);
        // Emptied.
        edit_description(&db, id, "");
        let revisions = texts(&db, id);
        assert_eq!(revisions.len(), 4);
        assert_eq!(revisions[0].1, "word word");
        // A small loss within ten minutes: coalesced.
        edit_description(&db, id, "a b c d e f");
        edit_description(&db, id, "a b c d e");
        assert_eq!(db.issue_revisions(id).unwrap().len(), 4);

        assert!(is_destructive("abcd", "a"));
        assert!(!is_destructive("abcd", "ab"));
        assert!(!is_destructive("", "x"));
    }

    #[test]
    fn checklist_items() {
        let db = Db::open_in_memory().unwrap();
        let issue = db
            .create_issue(&NewIssue::new("x").checklist(["a", "b", "c"]))
            .unwrap();
        let [a, b, c] = [0, 1, 2].map(|i| issue.checklist[i].id);
        db.edit_checklist_item(a, "a2").unwrap();
        db.set_checklist_item_done(b, true).unwrap();
        db.delete_checklist_item(b).unwrap();
        let revisions = db.issue_revisions(issue.id).unwrap();
        assert_eq!(
            revisions
                .iter()
                .map(|r| (r.field_label(), r.text.as_str(), r.reason))
                .collect::<Vec<_>>(),
            [
                (
                    format!("Checklist item {b} (deleted)"),
                    "b",
                    RevisionReason::Delete
                ),
                (format!("Checklist item {a}"), "a", RevisionReason::Edit),
            ]
        );

        // A deleted item comes back where it was, with its id.
        let restored = db.restore_revision(revisions[0].id).unwrap();
        let items: Vec<_> = restored
            .checklist
            .iter()
            .map(|i| (i.id, i.text.as_str(), i.done))
            .collect();
        assert_eq!(items, [(a, "a2", false), (b, "b", false), (c, "c", false)]);
        assert!(!db.get_revision(revisions[0].id).unwrap().item_deleted);
        // Or last, if the checklist got shorter.
        db.delete_checklist_item(c).unwrap();
        let deleted_c = db.issue_revisions(issue.id).unwrap()[0].id;
        db.delete_checklist_item(a).unwrap();
        db.delete_checklist_item(b).unwrap();
        let restored = db.restore_revision(deleted_c).unwrap();
        assert_eq!(restored.checklist.len(), 1);
        assert_eq!(restored.checklist[0].id, c);

        // An edit's revision of a deleted item brings it back, last.
        let restored = db.restore_revision(revisions[1].id).unwrap();
        let items: Vec<_> = restored
            .checklist
            .iter()
            .map(|i| (i.id, &*i.text))
            .collect();
        assert_eq!(items, [(c, "c"), (a, "a")]);
        // An item gets its text back; the current text is saved first.
        db.edit_checklist_item(a, "a3").unwrap();
        let before = db.issue_revisions(issue.id).unwrap().len();
        let undone = db.restore_revision(revisions[1].id).unwrap();
        assert_eq!(undone.checklist[1].text, "a");
        let after = db.issue_revisions(issue.id).unwrap();
        assert_eq!(after.len(), before + 1);
        assert_eq!(
            (after[0].text.as_str(), after[0].reason),
            ("a3", RevisionReason::Restore)
        );
    }

    #[test]
    fn item_ids_are_not_reused() {
        let db = Db::open_in_memory().unwrap();
        let issue = db
            .create_issue(&NewIssue::new("x").checklist(["a", "b"]))
            .unwrap();
        let b = issue.checklist[1].id;
        db.delete_checklist_item(b).unwrap();
        let issue = db.add_checklist_items(issue.id, &["c".into()]).unwrap();
        assert!(issue.checklist[1].id > b);
        let revisions = db.issue_revisions(issue.id).unwrap();
        assert!(revisions[0].item_deleted);
    }

    #[test]
    fn restore_title_and_description() {
        let db = Db::open_in_memory().unwrap();
        let id = db
            .create_issue(&NewIssue::new("old").description("kept text"))
            .unwrap()
            .id;
        let edit = IssueEdit {
            title: Some("new".into()),
            description: Some(String::new()),
        };
        db.edit_issue(id, &edit).unwrap();
        let revisions = db.issue_revisions(id).unwrap();
        let find = |field| revisions.iter().find(|r| r.field == field).unwrap().id;

        let issue = db
            .restore_revision(find(RevisionField::Description))
            .unwrap();
        assert_eq!(issue.description, "kept text");
        // The empty description wasn't worth saving.
        assert_eq!(db.issue_revisions(id).unwrap().len(), 2);

        let issue = db.restore_revision(find(RevisionField::Title)).unwrap();
        assert_eq!(issue.title, "old");
        let latest = &db.issue_revisions(id).unwrap()[0];
        assert_eq!(
            (latest.field, latest.text.as_str(), latest.reason),
            (RevisionField::Title, "new", RevisionReason::Restore)
        );
        // The restore can be undone, even within the coalescing time.
        assert_eq!(db.restore_revision(latest.id).unwrap().title, "new");
        // Restoring the text already there changes nothing.
        let count = db.issue_revisions(id).unwrap().len();
        db.restore_revision(latest.id).unwrap();
        assert_eq!(db.issue_revisions(id).unwrap().len(), count);

        assert!(db.restore_revision(RevisionId(999)).is_err());
        assert!(db.issue_revisions(IssueId(999)).is_err());
    }

    #[test]
    fn deleting_an_issue_drops_its_revisions() {
        let db = Db::open_in_memory().unwrap();
        let id = db
            .create_issue(&NewIssue::new("x").checklist(["a"]))
            .unwrap()
            .id;
        let item = db.get_issue(id).unwrap().checklist[0].id;
        db.edit_checklist_item(item, "b").unwrap();
        let revision = db.issue_revisions(id).unwrap()[0].id;
        // Archiving keeps them.
        db.set_archived(id, true).unwrap();
        assert_eq!(db.issue_revisions(id).unwrap().len(), 1);
        db.delete_issue(id).unwrap();
        assert!(db.get_revision(revision).is_err());
        let left: i64 = db
            .conn
            .query_row("SELECT count(*) FROM revisions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn thinning_out() {
        let now = 1000 * DAY;
        // One revision every ten minutes for 30 days.
        let times: Vec<i64> = (0..30 * 24 * 6).map(|i| now - i * 600).collect();
        let pruned = to_thin(&times, now);
        let kept: Vec<i64> = (0..times.len())
            .filter(|i| pruned.binary_search(i).is_err())
            .map(|i| now - times[i])
            .collect();
        assert_eq!(kept.len(), MAX_PER_FIELD);
        // The newest are kept: the whole last day fills it.
        assert!(kept.iter().all(|age| *age < DAY));

        // Fewer of them: all of the last day, hourly for a week, then daily.
        let times: Vec<i64> = (0..30 * 24).map(|h| now - h * HOUR).collect();
        let pruned = to_thin(&times, now);
        let kept: Vec<i64> = (0..times.len())
            .filter(|i| pruned.binary_search(i).is_err())
            .map(|i| now - times[i])
            .collect();
        assert_eq!(kept.len(), MAX_PER_FIELD);
        assert_eq!(kept.iter().filter(|a| **a < DAY).count(), 24);

        let few: Vec<i64> = [
            0,
            2 * DAY + 60,
            2 * DAY + 120,
            10 * DAY + HOUR,
            10 * DAY + 2 * HOUR,
        ]
        .map(|age| now - age)
        .to_vec();
        assert_eq!(to_thin(&few, now), [2, 4]);
    }

    #[test]
    fn revisions_are_thinned_out_when_saved() {
        let db = Db::open_in_memory().unwrap();
        let id = db.create_issue(&NewIssue::new("t0")).unwrap().id;
        for i in 1..=(MAX_PER_FIELD + 5) {
            let edit = IssueEdit {
                title: Some(format!("t{i}")),
                ..Default::default()
            };
            db.edit_issue(id, &edit).unwrap();
            age(&db, COALESCE);
        }
        let revisions = db.issue_revisions(id).unwrap();
        assert_eq!(revisions.len(), MAX_PER_FIELD);
        assert_eq!(revisions[0].text, format!("t{}", MAX_PER_FIELD + 4));
    }
}
