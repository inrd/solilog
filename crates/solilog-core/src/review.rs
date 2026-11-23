//! Review handoff: an agent that has finished an issue hands it to the user
//! with a receipt ([`Db::request_review`]) and moves on; the user approves
//! it (done) or sends it back with a reason.
//!
//! The issue goes to the `in_review` status (open, but not ready: no agent
//! should pick it up meanwhile). The receipt and the user's answer are notes
//! (`note.rs`) of kinds `receipt` and `review`, in the issue's worklog with
//! everything else, so each attempt keeps its own receipt and the history
//! reads in order. Notes are never edited: a new attempt is a new receipt.
//!
//! Approving and sending back are separate from marking an issue reviewed
//! ("I saw these changes"), though when the user does either, the issue is
//! marked reviewed too.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::Db;
use crate::issue::{Issue, IssueChange, IssueId, Status};
use crate::note::{Note, NoteKind, normalize_note_text};

/// One check run before handing work off: a command or test, and whether it
/// passed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    /// What was run, as written by the agent (`cargo test --workspace`),
    /// possibly with a short result (`cargo test: 2 failures in parser`).
    pub command: String,
    pub passed: bool,
}

/// What a `receipt` note holds besides its summary: the checks run and
/// what changed. Every list may be empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Receipt {
    pub checks: Vec<Check>,
    /// Commits (hashes or other references), as given.
    pub commits: Vec<String>,
    /// Files changed (paths), as given.
    pub files: Vec<String>,
}

impl Receipt {
    /// `true` when every check passed (or there is none).
    pub fn all_passed(&self) -> bool {
        self.checks.iter().all(|c| c.passed)
    }
}

/// The user's answer to a receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The work is accepted: the issue is done.
    Approved,
    /// The work goes back to `todo`, with the reason in the note.
    ChangesRequested,
}

/// The result of a review step: the issue as it now is, and the note the
/// step added to its worklog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Handoff {
    pub issue: Issue,
    pub note: Note,
}

/// The latest receipt in a worklog (oldest first, as
/// [`Db::issue_notes`] lists it).
pub fn latest_receipt(notes: &[Note]) -> Option<(&Note, &Receipt)> {
    notes.iter().rev().find_map(|note| match &note.kind {
        NoteKind::Receipt(receipt) => Some((note, receipt)),
        _ => None,
    })
}

/// Trims every entry of a receipt and checks none is empty.
fn normalize_receipt(receipt: &Receipt) -> Result<Receipt> {
    let clean = |what: &str, s: &str| -> Result<String> {
        let s = s.trim();
        anyhow::ensure!(!s.is_empty(), "a receipt's {what} cannot be empty");
        Ok(s.to_owned())
    };
    Ok(Receipt {
        checks: receipt
            .checks
            .iter()
            .map(|c| {
                Ok(Check {
                    command: clean("check", &c.command)?,
                    passed: c.passed,
                })
            })
            .collect::<Result<_>>()?,
        commits: receipt
            .commits
            .iter()
            .map(|c| clean("commit", c))
            .collect::<Result<_>>()?,
        files: receipt
            .files
            .iter()
            .map(|f| clean("file", f))
            .collect::<Result<_>>()?,
    })
}

impl Db {
    /// Hands an open issue to the user for review: adds a receipt note
    /// (`summary` is its text) and sets the status to `in_review`. Handing
    /// off again (after being sent back, or to add to the receipt) adds a
    /// new receipt; earlier ones stay.
    pub fn request_review(&self, id: IssueId, summary: &str, receipt: &Receipt) -> Result<Handoff> {
        let summary = normalize_note_text(summary)
            .map_err(|_| anyhow::anyhow!("a receipt needs a summary"))?;
        let receipt = normalize_receipt(receipt)?;
        let tx = self.conn.unchecked_transaction()?;
        let issue = self.get_issue(id)?;
        anyhow::ensure!(!issue.archived, "{id} is archived");
        anyhow::ensure!(
            !issue.status.is_closed(),
            "{id} is {}: set it back to an open status first",
            issue.status
        );
        let note = self.insert_note(id, &summary, &NoteKind::Receipt(receipt))?;
        self.apply_change(id, &IssueChange::SetStatus(Status::InReview))?;
        tx.commit()?;
        Ok(Handoff {
            issue: self.get_issue(id)?,
            note: self.get_note(note)?,
        })
    }

    /// Approves issues in review, all or nothing: adds a review note
    /// (`reason` may be empty) and sets them done. When the user does it,
    /// the issues are also marked reviewed. Returns them in the order given.
    pub fn approve_issues(&self, ids: &[IssueId], reason: &str) -> Result<Vec<Handoff>> {
        let reason = crate::issue::normalize_description(reason);
        self.answer_reviews(ids, &reason, Verdict::Approved, Status::Done)
    }

    /// Sends issues in review back to `todo`, all or nothing, with the
    /// reason (required) in a review note. When the user does it, the
    /// issues are also marked reviewed. Returns them in the order given.
    pub fn send_back_issues(&self, ids: &[IssueId], reason: &str) -> Result<Vec<Handoff>> {
        let reason = normalize_note_text(reason)
            .map_err(|_| anyhow::anyhow!("sending work back needs a reason"))?;
        self.answer_reviews(ids, &reason, Verdict::ChangesRequested, Status::Todo)
    }

    fn answer_reviews(
        &self,
        ids: &[IssueId],
        reason: &str,
        verdict: Verdict,
        status: Status,
    ) -> Result<Vec<Handoff>> {
        let tx = self.conn.unchecked_transaction()?;
        let mut notes = Vec::with_capacity(ids.len());
        // Last first, so that the first one ends up on top of its new
        // status (as `update_issues` does).
        for &id in ids.iter().rev() {
            let issue = self.get_issue(id)?;
            anyhow::ensure!(
                issue.status == Status::InReview,
                "{id} isn't in review (it is {})",
                issue.status
            );
            notes.push(self.insert_note(id, reason, &NoteKind::Review { verdict })?);
            self.apply_change(id, &IssueChange::SetStatus(status))?;
            if self.actor.is_user() {
                self.set_reviewed(id)?;
            }
        }
        tx.commit()?;
        notes.reverse();
        ids.iter()
            .zip(notes)
            .map(|(&id, note)| {
                Ok(Handoff {
                    issue: self.get_issue(id)?,
                    note: self.get_note(note)?,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ActivityKind, Actor, IssueFilter, NewIssue};

    fn receipt() -> Receipt {
        Receipt {
            checks: vec![
                Check {
                    command: " cargo test ".into(),
                    passed: true,
                },
                Check {
                    command: "make screenshot".into(),
                    passed: false,
                },
            ],
            commits: vec!["ee459b7".into()],
            files: vec!["src/note.rs".into()],
        }
    }

    #[test]
    fn hand_off_send_back_and_approve() {
        let mut db = Db::open_in_memory().unwrap();
        let issue = db.create_issue(&NewIssue::new("x")).unwrap();
        let agent = Actor::parse("claude:x").unwrap();
        db.set_actor(agent.clone());
        db.set_status(issue.id, Status::InProgress).unwrap();

        let first = db
            .request_review(issue.id, "Added it.\n", &receipt())
            .unwrap();
        assert_eq!(first.issue.status, Status::InReview);
        assert!(first.issue.unreviewed);
        assert_eq!(first.note.text, "Added it.");
        assert_eq!(first.note.author, agent);
        let NoteKind::Receipt(stored) = &first.note.kind else {
            panic!("not a receipt: {:?}", first.note.kind);
        };
        assert_eq!(stored.checks[0].command, "cargo test");
        assert!(!stored.all_passed());
        let json = serde_json::to_value(&first.note).unwrap();
        assert_eq!(json["kind"], "receipt");
        assert_eq!(json["checks"][1]["passed"], false);
        assert_eq!(json["commits"][0], "ee459b7");

        // In review: open, not ready, and issues depending on it still wait.
        let ready = IssueFilter {
            ready: true,
            ..Default::default()
        };
        assert!(db.list_issues(&ready).unwrap().is_empty());

        // Only the user's answers mark it reviewed.
        db.set_actor(Actor::user());
        assert!(db.send_back_issues(&[issue.id], " ").is_err());
        let back = db
            .send_back_issues(&[issue.id], "The test is missing.")
            .unwrap();
        assert_eq!(back[0].issue.status, Status::Todo);
        assert!(!back[0].issue.unreviewed);
        assert_eq!(
            back[0].note.kind,
            NoteKind::Review {
                verdict: Verdict::ChangesRequested
            }
        );
        // Not in review any more.
        assert!(db.approve_issues(&[issue.id], "").is_err());

        db.set_actor(agent);
        let second = db
            .request_review(issue.id, "Added the test.", &Receipt::default())
            .unwrap();
        db.set_actor(Actor::user());
        let done = db.approve_issues(&[issue.id], "").unwrap();
        assert_eq!(done[0].issue.status, Status::Done);
        assert!(!done[0].issue.unreviewed);
        assert_eq!(done[0].note.text, "");

        // Every attempt keeps its receipt; the latest is the second.
        let notes = db.issue_notes(issue.id).unwrap();
        assert_eq!(notes.len(), 4);
        let (latest, _) = latest_receipt(&notes).unwrap();
        assert_eq!(latest.id, second.note.id);

        let kinds: Vec<_> = db
            .issue_activities(issue.id)
            .unwrap()
            .into_iter()
            .map(|a| a.kind)
            .filter(|k| *k != ActivityKind::StatusChanged)
            .collect();
        assert_eq!(
            &kinds[..4],
            [
                ActivityKind::Approved,
                ActivityKind::ReviewRequested,
                ActivityKind::SentBack,
                ActivityKind::ReviewRequested,
            ]
        );

        // Closed issues can't be handed off.
        assert!(
            db.request_review(issue.id, "again", &Receipt::default())
                .is_err()
        );
    }

    #[test]
    fn receipts_are_checked() {
        let db = Db::open_in_memory().unwrap();
        let id = db.create_issue(&NewIssue::new("x")).unwrap().id;
        assert!(db.request_review(id, "  ", &Receipt::default()).is_err());
        let mut bad = receipt();
        bad.files.push(" ".into());
        assert!(db.request_review(id, "s", &bad).is_err());
        assert!(
            db.request_review(IssueId(99), "s", &Receipt::default())
                .is_err()
        );
        db.set_archived(id, true).unwrap();
        assert!(db.request_review(id, "s", &Receipt::default()).is_err());
        // Nothing was written.
        assert!(db.issue_notes(id).unwrap().is_empty());
    }

    #[test]
    fn answers_are_all_or_nothing_and_keep_the_order_given() {
        let db = Db::open_in_memory().unwrap();
        let a = db.create_issue(&NewIssue::new("a")).unwrap().id;
        let b = db.create_issue(&NewIssue::new("b")).unwrap().id;
        let c = db.create_issue(&NewIssue::new("c")).unwrap().id;
        for id in [a, b] {
            db.request_review(id, "done", &Receipt::default()).unwrap();
        }
        assert!(db.approve_issues(&[a, c], "").is_err());
        assert_eq!(db.get_issue(a).unwrap().status, Status::InReview);

        let done = db.approve_issues(&[b, a], "Looks good.").unwrap();
        assert_eq!(done[0].issue.id, b);
        assert_eq!(done[1].issue.id, a);
        assert_eq!(done[0].note.issue_id, b);
        assert_eq!(done[1].note.text, "Looks good.");
        let listed: Vec<_> = db
            .list_issues(&IssueFilter {
                statuses: vec![Status::Done],
                ..Default::default()
            })
            .unwrap()
            .into_iter()
            .map(|i| i.id)
            .collect();
        assert_eq!(listed, [b, a]);
    }
}
