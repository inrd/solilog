//! Notes: an issue's worklog. Timestamped, attributed entries added one
//! after the other, apart from the description (the spec, which agents
//! shouldn't have to rewrite) and the checklist: what was tried, what
//! failed, what was decided, handoffs to the next session, steering from the
//! user.
//!
//! Notes are never edited: a correction is a new note. A note can be
//! deleted by the user, or by the actor who wrote it, never by another
//! agent, so that an agent can't remove the user's steering or another
//! agent's log.

use std::fmt;
use std::str::FromStr;

use anyhow::{Context as _, Result};
use rusqlite::types::{FromSql, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use rusqlite::{OptionalExtension as _, Row, params};
use serde::{Deserialize, Serialize};

use crate::Db;
use crate::activity::{ActivityKind, Actor};
use crate::issue::{IssueId, normalize_description};

/// How much of a note the activity log keeps.
const ACTIVITY_DETAIL_CHARS: usize = 80;

/// Identifies a note, across all issues. Displayed as a plain number;
/// parsed from `12` or `#12`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NoteId(pub i64);

impl fmt::Display for NoteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for NoteId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        s.strip_prefix('#')
            .unwrap_or(s)
            .parse::<i64>()
            .ok()
            .filter(|n| *n > 0)
            .map(NoteId)
            .ok_or_else(|| format!("invalid note id {s:?} (expected a number)"))
    }
}

impl ToSql for NoteId {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        self.0.to_sql()
    }
}

impl FromSql for NoteId {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        i64::column_result(value).map(NoteId)
    }
}

/// One entry of an issue's worklog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Note {
    pub id: NoteId,
    pub issue_id: IssueId,
    /// Plain text, any number of lines (kept as written, like a
    /// description), never empty.
    pub text: String,
    /// Who wrote it.
    pub author: Actor,
    /// Unix timestamp (seconds).
    pub created_at: i64,
}

/// Cleans up a note's whitespace like a description's
/// ([`normalize_description`]) and checks that it is not empty.
pub fn normalize_note_text(text: &str) -> Result<String> {
    let text = normalize_description(text);
    anyhow::ensure!(!text.is_empty(), "a note cannot be empty");
    Ok(text)
}

/// The start of a note, on one line, for the activity log.
fn note_excerpt(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= ACTIVITY_DETAIL_CHARS {
        return line;
    }
    let cut: String = line.chars().take(ACTIVITY_DETAIL_CHARS - 1).collect();
    format!("{}…", cut.trim_end())
}

const NOTE_COLUMNS: &str = "id, issue_id, text, author, created_at";

fn note_from_row(row: &Row) -> rusqlite::Result<Note> {
    Ok(Note {
        id: row.get(0)?,
        issue_id: row.get(1)?,
        text: row.get(2)?,
        author: row.get(3)?,
        created_at: row.get(4)?,
    })
}

impl Db {
    /// Adds a note at the end of an issue's worklog and returns it.
    pub fn add_note(&self, issue: IssueId, text: &str) -> Result<Note> {
        let text = normalize_note_text(text)?;
        let tx = self.conn.unchecked_transaction()?;
        self.get_issue(issue)?;
        self.conn.execute(
            "INSERT INTO notes (issue_id, text, author) VALUES (?1, ?2, ?3)",
            params![issue, text, self.actor],
        )?;
        let id = NoteId(self.conn.last_insert_rowid());
        self.record_activity(issue, ActivityKind::NoteAdded, &note_excerpt(&text))?;
        tx.commit()?;
        self.get_note(id)
    }

    /// One note.
    pub fn get_note(&self, id: NoteId) -> Result<Note> {
        self.conn
            .query_row(
                &format!("SELECT {NOTE_COLUMNS} FROM notes WHERE id = ?1"),
                [id],
                note_from_row,
            )
            .optional()?
            .with_context(|| format!("no note {id}"))
    }

    /// An issue's notes, oldest first.
    pub fn issue_notes(&self, issue: IssueId) -> Result<Vec<Note>> {
        self.get_issue(issue)?;
        Ok(self
            .conn
            .prepare_cached(&format!(
                "SELECT {NOTE_COLUMNS} FROM notes WHERE issue_id = ?1 ORDER BY id"
            ))?
            .query_map([issue], note_from_row)?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Deletes a note for good and returns it as it was. Only the user and
    /// the note's author may delete it.
    pub fn delete_note(&self, id: NoteId) -> Result<Note> {
        let tx = self.conn.unchecked_transaction()?;
        let note = self.get_note(id)?;
        if !self.actor.is_user() && self.actor != note.author {
            if note.author.is_user() {
                anyhow::bail!("note {id} is by the user: only they can delete it");
            }
            anyhow::bail!(
                "note {id} is by {}: only they or the user can delete it",
                note.author
            );
        }
        self.conn.execute("DELETE FROM notes WHERE id = ?1", [id])?;
        self.record_activity(
            note.issue_id,
            ActivityKind::NoteDeleted,
            &note_excerpt(&note.text),
        )?;
        tx.commit()?;
        Ok(note)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IssueFilter, NewIssue};

    #[test]
    fn note_ids() {
        assert_eq!("12".parse(), Ok(NoteId(12)));
        assert_eq!(" #3".parse(), Ok(NoteId(3)));
        assert!("0".parse::<NoteId>().is_err());
        assert!("SOL-1".parse::<NoteId>().is_err());
        assert_eq!(serde_json::to_value(NoteId(4)).unwrap(), 4);
    }

    #[test]
    fn text_and_excerpt() {
        assert_eq!(
            normalize_note_text("\n  Tried X.  \n\n  Failed.\n\n").unwrap(),
            "  Tried X.\n\n  Failed."
        );
        assert!(normalize_note_text(" \n\t\n").is_err());
        assert_eq!(note_excerpt("a\n\nb   c"), "a b c");
        let long = note_excerpt(&"word ".repeat(40));
        assert_eq!(long.chars().count(), ACTIVITY_DETAIL_CHARS);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn add_list_delete() {
        let mut db = Db::open_in_memory().unwrap();
        let issue = db.create_issue(&NewIssue::new("x")).unwrap();
        assert!(db.issue_notes(issue.id).unwrap().is_empty());

        let first = db
            .add_note(issue.id, "Look at the parser first.\n")
            .unwrap();
        assert_eq!(first.text, "Look at the parser first.");
        assert_eq!(first.author, Actor::user());
        assert_eq!(first.issue_id, issue.id);

        let agent = Actor::parse("claude:parser").unwrap();
        db.set_actor(agent.clone());
        let second = db
            .add_note(issue.id, "Tried a fix.\nTests still fail.")
            .unwrap();
        assert_eq!(second.author, agent);
        let notes = db.issue_notes(issue.id).unwrap();
        assert_eq!(notes, [first.clone(), second.clone()]);

        // Agent notes need review, like any agent write.
        let issue_now = db.get_issue(issue.id).unwrap();
        assert!(issue_now.unreviewed);
        let pending = db.unreviewed_activities(issue.id).unwrap();
        assert_eq!(pending[0].kind, ActivityKind::NoteAdded);
        assert_eq!(pending[0].detail, "Tried a fix. Tests still fail.");

        // Empty notes and unknown issues fail.
        assert!(db.add_note(issue.id, "  \n").is_err());
        assert!(db.add_note(IssueId(999), "x").is_err());
        assert!(db.issue_notes(IssueId(999)).is_err());

        // Another agent can't delete them; the author and the user can.
        db.set_actor(Actor::parse("codex:other").unwrap());
        assert!(db.delete_note(first.id).is_err());
        assert!(db.delete_note(second.id).is_err());
        db.set_actor(agent);
        assert!(db.delete_note(first.id).is_err());
        assert_eq!(db.delete_note(second.id).unwrap(), second);
        db.set_actor(Actor::user());
        assert_eq!(db.delete_note(first.id).unwrap(), first);
        assert!(db.delete_note(first.id).is_err());
        assert!(db.issue_notes(issue.id).unwrap().is_empty());
        let kinds: Vec<_> = db
            .issue_activities(issue.id)
            .unwrap()
            .iter()
            .map(|a| a.kind)
            .collect();
        assert_eq!(
            &kinds[..2],
            [ActivityKind::NoteDeleted, ActivityKind::NoteDeleted]
        );
    }

    #[test]
    fn archiving_keeps_notes_and_deleting_the_issue_drops_them() {
        let db = Db::open_in_memory().unwrap();
        let issue = db.create_issue(&NewIssue::new("x")).unwrap();
        let note = db.add_note(issue.id, "kept").unwrap();
        db.set_archived(issue.id, true).unwrap();
        assert_eq!(
            db.issue_notes(issue.id).unwrap(),
            std::slice::from_ref(&note)
        );
        assert_eq!(
            db.list_archived_issues(&IssueFilter::default())
                .unwrap()
                .len(),
            1
        );
        db.delete_issue(issue.id).unwrap();
        assert!(db.get_note(note.id).is_err());
    }
}
