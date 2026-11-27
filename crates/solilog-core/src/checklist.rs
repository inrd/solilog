//! Checklists: an ordered list of single-line items, each done or not, on an
//! issue. They track progress only: an issue can be closed with unchecked
//! items.

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use anyhow::{Context as _, Result};
use rusqlite::types::{FromSql, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use rusqlite::{OptionalExtension as _, params};
use serde::{Deserialize, Serialize};

use crate::Db;
use crate::activity::{ActivityKind, Actor};
use crate::issue::{Issue, IssueId};
use crate::order::{Ordered, Placement};
use crate::revision::Field;

/// Identifies a checklist item, across all issues. Displayed as a plain
/// number; parsed from `12` or `#12`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ChecklistItemId(pub i64);

impl fmt::Display for ChecklistItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for ChecklistItemId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        s.strip_prefix('#')
            .unwrap_or(s)
            .parse::<i64>()
            .ok()
            .filter(|n| *n > 0)
            .map(ChecklistItemId)
            .ok_or_else(|| format!("invalid checklist item id {s:?} (expected a number)"))
    }
}

impl ToSql for ChecklistItemId {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        self.0.to_sql()
    }
}

impl FromSql for ChecklistItemId {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        i64::column_result(value).map(ChecklistItemId)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChecklistItem {
    pub id: ChecklistItemId,
    /// One line, never empty.
    pub text: String,
    pub done: bool,
    /// Who created or last edited this item's text.
    pub author: Actor,
}

/// Done and total items of an issue's checklist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ChecklistProgress {
    pub done: usize,
    pub total: usize,
}

/// Done and total items of a checklist, or `None` when it has no items.
pub fn checklist_progress(items: &[ChecklistItem]) -> Option<(usize, usize)> {
    (!items.is_empty()).then(|| (items.iter().filter(|i| i.done).count(), items.len()))
}

/// Makes item text one line (line breaks become spaces), trims it and checks
/// that it is not empty.
pub fn normalize_item_text(text: &str) -> Result<String> {
    let text = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    anyhow::ensure!(!text.is_empty(), "checklist item text cannot be empty");
    Ok(text)
}

/// Checklist items per issue, in order.
const ITEMS_SQL: &str = "SELECT issue_id, id, text, done, author FROM checklist_items
     WHERE ?1 IS NULL OR issue_id = ?1
     ORDER BY issue_id, position, id";

impl Db {
    /// Appends items to an issue's checklist, all or nothing, and returns
    /// the issue.
    pub fn add_checklist_items(&self, issue: IssueId, texts: &[String]) -> Result<Issue> {
        let texts = texts
            .iter()
            .map(|t| normalize_item_text(t))
            .collect::<Result<Vec<_>>>()?;
        let tx = self.conn.unchecked_transaction()?;
        self.get_issue(issue)?;
        self.insert_checklist_items(issue, &texts)?;
        if !texts.is_empty() {
            let detail = if texts.len() == 1 {
                texts[0].clone()
            } else {
                format!("{} items", texts.len())
            };
            self.record_activity(issue, ActivityKind::ChecklistAdded, &detail)?;
        }
        tx.commit()?;
        self.get_issue(issue)
    }

    /// Appends normalized items. Call inside a transaction.
    pub(crate) fn insert_checklist_items(&self, issue: IssueId, texts: &[String]) -> Result<()> {
        let mut position: i64 = self.conn.query_row(
            "SELECT coalesce(max(position), 0) FROM checklist_items WHERE issue_id = ?1",
            [issue],
            |row| row.get(0),
        )?;
        let mut insert = self.conn.prepare_cached(
            "INSERT INTO checklist_items (issue_id, text, position, author) VALUES (?1, ?2, ?3, ?4)",
        )?;
        for text in texts {
            position += 1;
            insert.execute(params![issue, text, position, self.actor])?;
        }
        Ok(())
    }

    /// Changes the text of a checklist item and returns its issue. The text
    /// it replaces is kept as a revision (see [`Db::issue_revisions`]).
    pub fn edit_checklist_item(&self, item: ChecklistItemId, text: &str) -> Result<Issue> {
        let text = normalize_item_text(text)?;
        let tx = self.conn.unchecked_transaction()?;
        let (issue, old) = self.checklist_item(item)?;
        if old != text {
            let author: Actor = self.conn.query_row(
                "SELECT author FROM checklist_items WHERE id = ?1",
                [item],
                |row| row.get(0),
            )?;
            self.record_edit(issue, Field::ChecklistItem(item), &old, &author, &text)?;
            self.conn.execute(
                "UPDATE checklist_items SET text = ?2, author = ?3 WHERE id = ?1",
                params![item, text, self.actor],
            )?;
            self.record_activity(issue, ActivityKind::ChecklistEdited, &text)?;
        }
        tx.commit()?;
        self.get_issue(issue)
    }

    /// Checks (`true`) or unchecks a checklist item and returns its issue.
    pub fn set_checklist_item_done(&self, item: ChecklistItemId, done: bool) -> Result<Issue> {
        let tx = self.conn.unchecked_transaction()?;
        let (issue, text, was_done): (IssueId, String, bool) = self
            .conn
            .query_row(
                "SELECT issue_id, text, done FROM checklist_items WHERE id = ?1",
                [item],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
            .with_context(|| format!("no checklist item {item}"))?;
        if was_done != done {
            self.conn.execute(
                "UPDATE checklist_items SET done = ?2 WHERE id = ?1",
                params![item, done],
            )?;
            let kind = if done {
                ActivityKind::ChecklistChecked
            } else {
                ActivityKind::ChecklistUnchecked
            };
            self.record_activity(issue, kind, &text)?;
        }
        tx.commit()?;
        self.get_issue(issue)
    }

    /// Moves a checklist item before or after another item of the same
    /// checklist, and returns its issue.
    pub fn move_checklist_item(
        &self,
        item: ChecklistItemId,
        placement: &Placement<ChecklistItemId>,
    ) -> Result<Issue> {
        let issue = self.checklist_item_issue(item)?;
        let anchor = *placement.anchor();
        anyhow::ensure!(
            self.checklist_item_issue(anchor)? == issue,
            "checklist items {item} and {anchor} are not on the same issue"
        );
        let tx = self.conn.unchecked_transaction()?;
        let order = self.checklist_order(issue)?;
        self.place(Ordered::ChecklistItems, order, item.0, anchor.0, placement)?;
        tx.commit()?;
        self.get_issue(issue)
    }

    /// Deletes a checklist item and returns its issue. Its text is kept as
    /// a revision, to bring it back (see [`Db::restore_revision`]).
    pub fn delete_checklist_item(&self, item: ChecklistItemId) -> Result<Issue> {
        let tx = self.conn.unchecked_transaction()?;
        let (issue, text) = self.checklist_item(item)?;
        let position = self
            .checklist_order(issue)?
            .iter()
            .position(|&id| id == item.0)
            .context("checklist item vanished")?;
        self.record_delete(issue, item, &text, position)?;
        self.conn
            .execute("DELETE FROM checklist_items WHERE id = ?1", [item])?;
        self.record_activity(issue, ActivityKind::ChecklistDeleted, &text)?;
        tx.commit()?;
        self.get_issue(issue)
    }

    /// The issue a checklist item is on.
    fn checklist_item_issue(&self, item: ChecklistItemId) -> Result<IssueId> {
        Ok(self.checklist_item(item)?.0)
    }

    /// The issue a checklist item is on, and the item's text.
    pub(crate) fn checklist_item(&self, item: ChecklistItemId) -> Result<(IssueId, String)> {
        self.conn
            .query_row(
                "SELECT issue_id, text FROM checklist_items WHERE id = ?1",
                [item],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .with_context(|| format!("no checklist item {item}"))
    }

    /// The ids of an issue's checklist items, in order.
    pub(crate) fn checklist_order(&self, issue: IssueId) -> Result<Vec<i64>> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT id FROM checklist_items WHERE issue_id = ?1 ORDER BY position, id",
            )?
            .query_map([issue], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Checklist items per issue, for one issue or all of them.
    pub(crate) fn checklists_per_issue(
        &self,
        only: Option<IssueId>,
    ) -> Result<HashMap<IssueId, Vec<ChecklistItem>>> {
        let mut map: HashMap<IssueId, Vec<ChecklistItem>> = HashMap::new();
        let mut stmt = self.conn.prepare_cached(ITEMS_SQL)?;
        let rows = stmt.query_map([only], |row| {
            Ok((
                row.get::<_, IssueId>(0)?,
                ChecklistItem {
                    id: row.get(1)?,
                    text: row.get(2)?,
                    done: row.get(3)?,
                    author: row.get(4)?,
                },
            ))
        })?;
        for row in rows {
            let (issue, item) = row?;
            map.entry(issue).or_default().push(item);
        }
        Ok(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IssueFilter, NewIssue, Status};

    fn texts(issue: &Issue) -> Vec<(&str, bool)> {
        issue
            .checklist
            .iter()
            .map(|i| (i.text.as_str(), i.done))
            .collect()
    }

    #[test]
    fn item_ids() {
        assert_eq!("12".parse(), Ok(ChecklistItemId(12)));
        assert_eq!(" #3".parse(), Ok(ChecklistItemId(3)));
        assert!("0".parse::<ChecklistItemId>().is_err());
        assert!("SOL-1".parse::<ChecklistItemId>().is_err());
        assert_eq!(serde_json::to_value(ChecklistItemId(4)).unwrap(), 4);
    }

    #[test]
    fn text_normalization() {
        assert_eq!(normalize_item_text("  buy milk ").unwrap(), "buy milk");
        assert_eq!(normalize_item_text("a\r\n  b\n\n").unwrap(), "a b");
        assert!(normalize_item_text(" \n ").is_err());
    }

    #[test]
    fn add_check_edit_delete() {
        let db = Db::open_in_memory().unwrap();
        let issue = db.create_issue(&NewIssue::new("x")).unwrap();
        assert!(issue.checklist.is_empty());
        assert_eq!(checklist_progress(&issue.checklist), None);

        let issue = db
            .add_checklist_items(issue.id, &["one".into(), " two ".into()])
            .unwrap();
        assert_eq!(texts(&issue), [("one", false), ("two", false)]);
        let issue = db.add_checklist_items(issue.id, &["three".into()]).unwrap();
        let [one, two, three] = [0, 1, 2].map(|i| issue.checklist[i].id);

        let issue = db.set_checklist_item_done(two, true).unwrap();
        assert_eq!(checklist_progress(&issue.checklist), Some((1, 3)));
        let issue = db.edit_checklist_item(one, "first").unwrap();
        assert_eq!(
            texts(&issue),
            [("first", false), ("two", true), ("three", false)]
        );
        assert!(db.edit_checklist_item(one, "  ").is_err());

        let issue = db.delete_checklist_item(three).unwrap();
        assert_eq!(texts(&issue), [("first", false), ("two", true)]);
        assert!(db.delete_checklist_item(three).is_err());
        assert!(db.set_checklist_item_done(three, true).is_err());

        // All or nothing; unknown issues fail.
        assert!(
            db.add_checklist_items(issue.id, &["ok".into(), "".into()])
                .is_err()
        );
        assert_eq!(db.get_issue(issue.id).unwrap().checklist.len(), 2);
        assert!(db.add_checklist_items(IssueId(999), &["x".into()]).is_err());

        // Listings carry checklists; deleting the issue deletes them.
        let other = db.create_issue(&NewIssue::new("other")).unwrap();
        let listed = db.list_issues(&IssueFilter::default()).unwrap();
        assert_eq!(listed[0].id, other.id);
        assert!(listed[0].checklist.is_empty());
        assert_eq!(texts(&listed[1]), [("first", false), ("two", true)]);
        db.delete_issue(issue.id).unwrap();
        assert!(db.edit_checklist_item(one, "gone").is_err());
    }

    #[test]
    fn closing_ignores_the_checklist() {
        let db = Db::open_in_memory().unwrap();
        let issue = db
            .create_issue(&NewIssue::new("x").checklist(["a", "b"]))
            .unwrap();
        assert_eq!(texts(&issue), [("a", false), ("b", false)]);
        let done = db.set_status(issue.id, Status::Done).unwrap();
        assert_eq!(done.status, Status::Done);
        assert_eq!(checklist_progress(&done.checklist), Some((0, 2)));
    }

    #[test]
    fn reorder() {
        let db = Db::open_in_memory().unwrap();
        let issue = db
            .create_issue(&NewIssue::new("x").checklist(["a", "b", "c"]))
            .unwrap();
        let [a, b, c] = [0, 1, 2].map(|i| issue.checklist[i].id);
        let order = |issue: Issue| -> String { issue.checklist.iter().map(|i| &*i.text).collect() };

        assert_eq!(
            order(db.move_checklist_item(c, &Placement::Before(a)).unwrap()),
            "cab"
        );
        assert_eq!(
            order(db.move_checklist_item(c, &Placement::After(b)).unwrap()),
            "abc"
        );
        assert_eq!(
            order(db.move_checklist_item(a, &Placement::After(a)).unwrap()),
            "abc"
        );
        // New items still go last.
        assert_eq!(
            order(db.add_checklist_items(issue.id, &["d".into()]).unwrap()),
            "abcd"
        );

        let other = db
            .create_issue(&NewIssue::new("y").checklist(["z"]))
            .unwrap();
        let z = other.checklist[0].id;
        assert!(db.move_checklist_item(a, &Placement::Before(z)).is_err());
    }
}
