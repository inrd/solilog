//! Decisions: short standing statements ("Keep the v1 API; breaking changes
//! wait for 2.0") recorded on one issue, label or project, so that later
//! sessions read what was settled instead of reopening it.
//!
//! A decision applies where it is recorded and below: a label's decisions
//! apply to its sublabels and their issues, a project's to its issues
//! ([`Db::inherited_decisions`]). Notes stay the issue's worklog; a decision
//! is apart from it because it can live on labels and projects and stays in
//! force until it is replaced or withdrawn.
//!
//! Decisions are never edited: [`Db::replace_decision`] records a new one in
//! its place (the old one is kept, superseded, pointing to it) and
//! [`Db::withdraw_decision`] ends one. Only decisions in force are listed by
//! default; ended ones stay, so what changed and when stays readable.
//!
//! Each decision is reviewed on its own: one an agent records (or
//! withdraws) waits for the user to confirm it ([`Db::mark_decisions_reviewed`]).
//! Agents still read it meanwhile, with its author. On an issue it is also
//! an activity, so it marks the issue unreviewed like any agent write, and
//! marking the issue reviewed confirms its decisions.

use std::fmt;
use std::str::FromStr;

use anyhow::{Context as _, Result};
use rusqlite::types::{FromSql, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use rusqlite::{OptionalExtension as _, Row, params};
use serde::{Deserialize, Serialize};

use crate::Db;
use crate::activity::{ActivityKind, Actor};
use crate::issue::{IssueId, normalize_description};
use crate::label::LABEL_SEPARATOR;
use crate::note::note_excerpt;

/// Identifies a decision, across all owners. Displayed as a plain number;
/// parsed from `12` or `#12`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DecisionId(pub i64);

impl fmt::Display for DecisionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for DecisionId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        s.strip_prefix('#')
            .unwrap_or(s)
            .parse::<i64>()
            .ok()
            .filter(|n| *n > 0)
            .map(DecisionId)
            .ok_or_else(|| format!("invalid decision id {s:?} (expected a number)"))
    }
}

impl ToSql for DecisionId {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        self.0.to_sql()
    }
}

impl FromSql for DecisionId {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        i64::column_result(value).map(DecisionId)
    }
}

/// What a decision is recorded on. In JSON: `{"kind": "issue", "id":
/// "SOL-12"}`, `{"kind": "label", "name": "acme/reverb"}` or `{"kind":
/// "project", "name": "reverb v1.1"}`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DecisionOwner {
    Issue {
        id: IssueId,
    },
    /// A label's full name.
    Label {
        name: String,
    },
    Project {
        name: String,
    },
}

impl DecisionOwner {
    pub fn issue(id: IssueId) -> Self {
        Self::Issue { id }
    }

    pub fn label(name: impl Into<String>) -> Self {
        Self::Label { name: name.into() }
    }

    pub fn project(name: impl Into<String>) -> Self {
        Self::Project { name: name.into() }
    }

    /// `issue`, `label` or `project`.
    pub fn kind_key(&self) -> &'static str {
        match self {
            Self::Issue { .. } => "issue",
            Self::Label { .. } => "label",
            Self::Project { .. } => "project",
        }
    }
}

impl fmt::Display for DecisionOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Issue { id } => write!(f, "{id}"),
            Self::Label { name } => write!(f, "label {name}"),
            Self::Project { name } => write!(f, "project {name}"),
        }
    }
}

/// Whether a decision still holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionState {
    InForce,
    /// Replaced by another decision (`replaced_by`).
    Superseded,
    Withdrawn,
}

impl DecisionState {
    pub fn key(self) -> &'static str {
        match self {
            Self::InForce => "in_force",
            Self::Superseded => "superseded",
            Self::Withdrawn => "withdrawn",
        }
    }
}

/// A standing statement on an issue, label or project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Decision {
    pub id: DecisionId,
    /// What it is recorded on.
    pub owner: DecisionOwner,
    /// Plain text, kept as written (like a description). Never empty.
    pub text: String,
    /// Who recorded it.
    pub author: Actor,
    /// Unix timestamp (seconds).
    pub created_at: i64,
    pub state: DecisionState,
    /// The decision this one replaced, if any.
    pub replaces: Option<DecisionId>,
    /// The decision that replaced this one, when superseded.
    pub replaced_by: Option<DecisionId>,
    /// When it was withdrawn or replaced (Unix timestamp), and by whom.
    pub ended_at: Option<i64>,
    pub ended_by: Option<Actor>,
    /// `false` while an agent's decision (or withdrawal) awaits the user.
    pub reviewed: bool,
}

impl Decision {
    pub fn in_force(&self) -> bool {
        self.state == DecisionState::InForce
    }
}

/// Cleans up a decision's whitespace like a description's and checks that
/// it is not empty.
pub fn normalize_decision_text(text: &str) -> Result<String> {
    let text = normalize_description(text);
    anyhow::ensure!(!text.is_empty(), "a decision cannot be empty");
    Ok(text)
}

/// `acme`, `acme/reverb` for `acme/reverb/eq`: a label's ancestors, broadest
/// first, without the label itself.
fn label_ancestors(name: &str) -> impl Iterator<Item = &str> {
    name.match_indices(LABEL_SEPARATOR).map(|(i, _)| &name[..i])
}

const DECISION_SELECT: &str = "SELECT d.id, d.issue_id, l.name, p.name, d.text, d.author,
        d.created_at, d.ended_at, d.ended_by, d.replaced_by, d.reviewed_at IS NOT NULL,
        (SELECT o.id FROM decisions o WHERE o.replaced_by = d.id)
    FROM decisions d
    LEFT JOIN labels l ON l.id = d.label_id
    LEFT JOIN projects p ON p.id = d.project_id";

fn decision_from_row(row: &Row) -> rusqlite::Result<Decision> {
    let issue: Option<IssueId> = row.get(1)?;
    let label: Option<String> = row.get(2)?;
    let project: Option<String> = row.get(3)?;
    let owner = match (issue, label, project) {
        (Some(id), _, _) => DecisionOwner::Issue { id },
        (_, Some(name), _) => DecisionOwner::Label { name },
        (_, _, Some(name)) => DecisionOwner::Project { name },
        _ => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Null,
                "decision without an owner".into(),
            ));
        }
    };
    let ended_at: Option<i64> = row.get(7)?;
    let replaced_by: Option<DecisionId> = row.get(9)?;
    let state = match (ended_at, replaced_by) {
        (None, _) => DecisionState::InForce,
        (Some(_), Some(_)) => DecisionState::Superseded,
        (Some(_), None) => DecisionState::Withdrawn,
    };
    Ok(Decision {
        id: row.get(0)?,
        owner,
        text: row.get(4)?,
        author: row.get(5)?,
        created_at: row.get(6)?,
        state,
        replaces: row.get(11)?,
        replaced_by,
        ended_at,
        ended_by: row.get(8)?,
        reviewed: row.get(10)?,
    })
}

/// The owner as stored: which column, and the row id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OwnerRow {
    Issue(IssueId),
    Label(i64),
    Project(i64),
}

impl OwnerRow {
    fn condition(self) -> (&'static str, i64) {
        match self {
            Self::Issue(id) => ("d.issue_id = ?1", id.0),
            Self::Label(id) => ("d.label_id = ?1", id),
            Self::Project(id) => ("d.project_id = ?1", id),
        }
    }
}

impl Db {
    /// Records a decision on an issue, label or project and returns it. A
    /// decision an agent records is unreviewed until the user confirms it.
    pub fn record_decision(&self, owner: &DecisionOwner, text: &str) -> Result<Decision> {
        let text = normalize_decision_text(text)?;
        let tx = self.conn.unchecked_transaction()?;
        let owner = self.owner_row(owner)?;
        let id = self.insert_decision(owner, &text)?;
        if let OwnerRow::Issue(issue) = owner {
            self.record_activity(issue, ActivityKind::DecisionRecorded, &note_excerpt(&text))?;
        }
        tx.commit()?;
        self.get_decision(id)
    }

    /// Replaces a decision in force with a new one on the same owner and
    /// returns the new one. The old one is kept, superseded; replaced by the
    /// user, it counts as reviewed (they saw it).
    pub fn replace_decision(&self, id: DecisionId, text: &str) -> Result<Decision> {
        let text = normalize_decision_text(text)?;
        let tx = self.conn.unchecked_transaction()?;
        let old = self.get_decision(id)?;
        anyhow::ensure!(
            old.in_force(),
            "decision {id} is {}: only a decision in force can be replaced",
            old.state.key().replace('_', " ")
        );
        let owner = self.owner_row(&old.owner)?;
        let new = self.insert_decision(owner, &text)?;
        self.conn.execute(
            "UPDATE decisions
             SET ended_at = unixepoch(), ended_by = ?2, replaced_by = ?3,
                 reviewed_at = CASE WHEN ?4 THEN coalesce(reviewed_at, unixepoch())
                                    ELSE reviewed_at END
             WHERE id = ?1",
            params![id, self.actor, new, self.actor.is_user()],
        )?;
        if let OwnerRow::Issue(issue) = owner {
            self.record_activity(issue, ActivityKind::DecisionReplaced, &note_excerpt(&text))?;
        }
        tx.commit()?;
        self.get_decision(new)
    }

    /// Withdraws a decision in force and returns it. Withdrawn by an agent,
    /// it is unreviewed until the user confirms; by the user, reviewed.
    pub fn withdraw_decision(&self, id: DecisionId) -> Result<Decision> {
        let tx = self.conn.unchecked_transaction()?;
        let decision = self.get_decision(id)?;
        anyhow::ensure!(
            decision.in_force(),
            "decision {id} is already {}",
            decision.state.key().replace('_', " ")
        );
        self.conn.execute(
            "UPDATE decisions
             SET ended_at = unixepoch(), ended_by = ?2,
                 reviewed_at = CASE WHEN ?3 THEN coalesce(reviewed_at, unixepoch())
                                    ELSE NULL END
             WHERE id = ?1",
            params![id, self.actor, self.actor.is_user()],
        )?;
        if let DecisionOwner::Issue { id: issue } = decision.owner {
            self.record_activity(
                issue,
                ActivityKind::DecisionWithdrawn,
                &note_excerpt(&decision.text),
            )?;
        }
        tx.commit()?;
        self.get_decision(id)
    }

    /// One decision.
    pub fn get_decision(&self, id: DecisionId) -> Result<Decision> {
        self.conn
            .query_row(
                &format!("{DECISION_SELECT} WHERE d.id = ?1"),
                [id],
                decision_from_row,
            )
            .optional()?
            .with_context(|| format!("no decision {id}"))
    }

    /// The decisions recorded on exactly this issue, label or project (not
    /// inherited ones), oldest first: those in force, or all of them with
    /// `include_ended`.
    pub fn decisions(&self, owner: &DecisionOwner, include_ended: bool) -> Result<Vec<Decision>> {
        let owner = self.owner_row(owner)?;
        self.owner_decisions(owner, include_ended)
    }

    /// The decisions in force that apply to an issue, label or project:
    /// for an issue, its own, its projects' and its labels' with their
    /// ancestors'; for a label, its own and its ancestors'; for a project,
    /// its own. Most specific first (the issue's, then projects', then
    /// labels' from the deepest level up), each owner's oldest first.
    pub fn inherited_decisions(&self, owner: &DecisionOwner) -> Result<Vec<Decision>> {
        let mut out = Vec::new();
        for owner in self.owner_chain(owner)? {
            out.extend(self.owner_decisions(owner, false)?);
        }
        Ok(out)
    }

    /// What the app shows for an issue, label or project: the decisions
    /// that apply ([`Db::inherited_decisions`]), then the ended ones of the
    /// same owners still awaiting review (an agent withdrew or replaced
    /// them), oldest first.
    pub fn shown_decisions(&self, owner: &DecisionOwner) -> Result<Vec<Decision>> {
        let chain = self.owner_chain(owner)?;
        let mut out = Vec::new();
        let mut ended = Vec::new();
        for owner in chain {
            for decision in self.owner_decisions(owner, true)? {
                if decision.in_force() {
                    out.push(decision);
                } else if !decision.reviewed {
                    ended.push(decision);
                }
            }
        }
        ended.sort_by_key(|d| d.id);
        out.extend(ended);
        Ok(out)
    }

    /// Every decision awaiting the user's review, oldest first.
    pub fn unreviewed_decisions(&self) -> Result<Vec<Decision>> {
        Ok(self
            .conn
            .prepare_cached(&format!(
                "{DECISION_SELECT} WHERE d.reviewed_at IS NULL ORDER BY d.id"
            ))?
            .query_map([], decision_from_row)?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Confirms decisions (the user saw them), all or nothing. Returns them
    /// in the order given.
    pub fn mark_decisions_reviewed(&self, ids: &[DecisionId]) -> Result<Vec<Decision>> {
        let tx = self.conn.unchecked_transaction()?;
        for &id in ids {
            let updated = self.conn.execute(
                "UPDATE decisions SET reviewed_at = coalesce(reviewed_at, unixepoch())
                 WHERE id = ?1",
                [id],
            )?;
            anyhow::ensure!(updated == 1, "no decision {id}");
        }
        tx.commit()?;
        ids.iter().map(|&id| self.get_decision(id)).collect()
    }

    // ---- helpers ------------------------------------------------------------

    /// Confirms an issue's own decisions. Call inside a transaction.
    pub(crate) fn set_issue_decisions_reviewed(&self, issue: IssueId) -> Result<()> {
        self.conn.execute(
            "UPDATE decisions SET reviewed_at = unixepoch()
             WHERE issue_id = ?1 AND reviewed_at IS NULL",
            [issue],
        )?;
        Ok(())
    }

    /// The owners whose decisions apply to `owner`, most specific first
    /// (see [`Db::inherited_decisions`]).
    fn owner_chain(&self, owner: &DecisionOwner) -> Result<Vec<OwnerRow>> {
        let mut owners = Vec::new();
        match owner {
            DecisionOwner::Issue { id } => {
                let issue = self.get_issue(*id)?;
                owners.push(OwnerRow::Issue(issue.id));
                for project in &issue.projects {
                    owners.push(OwnerRow::Project(self.project_id(project)?));
                }
                let mut labels = Vec::new();
                for label in &issue.labels {
                    self.push_label_chain(label, &mut labels)?;
                }
                labels.sort_by_key(|(depth, _)| std::cmp::Reverse(*depth));
                owners.extend(labels.into_iter().map(|(_, row)| row));
            }
            DecisionOwner::Label { name } => {
                let mut labels = Vec::new();
                self.push_label_chain(&self.label_name(name)?, &mut labels)?;
                owners.extend(labels.into_iter().map(|(_, row)| row));
            }
            DecisionOwner::Project { name } => {
                owners.push(OwnerRow::Project(self.project_id(name)?));
            }
        }
        Ok(owners)
    }

    fn owner_row(&self, owner: &DecisionOwner) -> Result<OwnerRow> {
        Ok(match owner {
            DecisionOwner::Issue { id } => OwnerRow::Issue(self.get_issue(*id)?.id),
            DecisionOwner::Label { name } => OwnerRow::Label(self.label_id(name)?),
            DecisionOwner::Project { name } => OwnerRow::Project(self.project_id(name)?),
        })
    }

    /// Adds a label and its ancestors to `out` with their depth (1 for a
    /// top-level label), deepest first, skipping those already there.
    fn push_label_chain(&self, name: &str, out: &mut Vec<(usize, OwnerRow)>) -> Result<()> {
        let ancestors: Vec<&str> = label_ancestors(name).collect();
        let chain = std::iter::once(name).chain(ancestors.into_iter().rev());
        for label in chain {
            let row = OwnerRow::Label(self.label_id(label)?);
            if !out.iter().any(|(_, r)| *r == row) {
                out.push((label.matches(LABEL_SEPARATOR).count() + 1, row));
            }
        }
        Ok(())
    }

    fn insert_decision(&self, owner: OwnerRow, text: &str) -> Result<DecisionId> {
        let (issue, label, project) = match owner {
            OwnerRow::Issue(id) => (Some(id), None, None),
            OwnerRow::Label(id) => (None, Some(id), None),
            OwnerRow::Project(id) => (None, None, Some(id)),
        };
        self.conn.execute(
            "INSERT INTO decisions (issue_id, label_id, project_id, text, author, reviewed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, CASE WHEN ?6 THEN unixepoch() END)",
            params![
                issue,
                label,
                project,
                text,
                self.actor,
                self.actor.is_user()
            ],
        )?;
        Ok(DecisionId(self.conn.last_insert_rowid()))
    }

    fn owner_decisions(&self, owner: OwnerRow, include_ended: bool) -> Result<Vec<Decision>> {
        let (condition, id) = owner.condition();
        let ended = if include_ended {
            ""
        } else {
            " AND d.ended_at IS NULL"
        };
        Ok(self
            .conn
            .prepare_cached(&format!(
                "{DECISION_SELECT} WHERE {condition}{ended} ORDER BY d.id"
            ))?
            .query_map([id], decision_from_row)?
            .collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NewIssue, NewProject};

    fn agent() -> Actor {
        Actor::parse("claude:api").unwrap()
    }

    #[test]
    fn ids_and_ancestors() {
        assert_eq!("12".parse(), Ok(DecisionId(12)));
        assert_eq!("#3".parse(), Ok(DecisionId(3)));
        assert!("0".parse::<DecisionId>().is_err());
        let a: Vec<_> = label_ancestors("acme/reverb/eq").collect();
        assert_eq!(a, ["acme", "acme/reverb"]);
        assert_eq!(label_ancestors("acme").count(), 0);
        assert_eq!(
            serde_json::to_value(DecisionOwner::issue(IssueId(4))).unwrap(),
            serde_json::json!({"kind": "issue", "id": "SOL-4"})
        );
        assert_eq!(
            serde_json::to_value(DecisionOwner::label("acme")).unwrap(),
            serde_json::json!({"kind": "label", "name": "acme"})
        );
    }

    #[test]
    fn record_replace_withdraw() {
        let db = Db::open_in_memory().unwrap();
        db.create_label("acme/reverb").unwrap();
        let first = db
            .record_decision(&DecisionOwner::label("ACME"), "  Keep the v1 API.\n")
            .unwrap();
        assert_eq!(first.text, "  Keep the v1 API.");
        assert_eq!(first.owner, DecisionOwner::label("acme"));
        assert_eq!(first.state, DecisionState::InForce);
        assert!(first.reviewed, "the user's decisions need no review");
        assert_eq!(first.author, Actor::user());

        let json = serde_json::to_value(&first).unwrap();
        assert_eq!(json["state"], "in_force");
        assert_eq!(json["owner"]["kind"], "label");

        let second = db
            .replace_decision(first.id, "Break the API in 2.0 only.")
            .unwrap();
        assert_eq!(second.replaces, Some(first.id));
        assert_eq!(second.owner, first.owner);
        let first = db.get_decision(first.id).unwrap();
        assert_eq!(first.state, DecisionState::Superseded);
        assert_eq!(first.replaced_by, Some(second.id));
        assert!(first.ended_at.is_some());
        assert!(db.replace_decision(first.id, "again").is_err());
        assert!(db.withdraw_decision(first.id).is_err());

        let owner = DecisionOwner::label("acme");
        assert_eq!(
            db.decisions(&owner, false).unwrap(),
            std::slice::from_ref(&second)
        );
        assert_eq!(db.decisions(&owner, true).unwrap().len(), 2);

        let withdrawn = db.withdraw_decision(second.id).unwrap();
        assert_eq!(withdrawn.state, DecisionState::Withdrawn);
        assert_eq!(withdrawn.replaced_by, None);
        assert!(db.decisions(&owner, false).unwrap().is_empty());

        assert!(db.record_decision(&owner, " \n").is_err());
        assert!(
            db.record_decision(&DecisionOwner::label("nope"), "x")
                .is_err()
        );
        assert!(
            db.record_decision(&DecisionOwner::issue(IssueId(9)), "x")
                .is_err()
        );
        assert!(db.get_decision(DecisionId(99)).is_err());
    }

    #[test]
    fn inheritance() {
        let db = Db::open_in_memory().unwrap();
        db.create_project(&NewProject::new("v2")).unwrap();
        let issue = db
            .create_issue(
                &NewIssue::new("x")
                    .labels(["acme/reverb/eq", "acme/delay"])
                    .projects(["v2"]),
            )
            .unwrap();
        let other = db.create_issue(&NewIssue::new("y")).unwrap();
        let rec = |owner: DecisionOwner, text: &str| db.record_decision(&owner, text).unwrap().id;
        let root = rec(DecisionOwner::label("acme"), "root");
        let reverb = rec(DecisionOwner::label("acme/reverb"), "reverb");
        let eq = rec(DecisionOwner::label("acme/reverb/eq"), "eq");
        let delay = rec(DecisionOwner::label("acme/delay"), "delay");
        let project = rec(DecisionOwner::project("V2"), "project");
        let own = rec(DecisionOwner::issue(issue.id), "own");
        rec(DecisionOwner::issue(other.id), "elsewhere");
        let ended = rec(DecisionOwner::label("acme"), "ended");
        db.withdraw_decision(ended).unwrap();

        let ids = |owner: DecisionOwner| -> Vec<DecisionId> {
            db.inherited_decisions(&owner)
                .unwrap()
                .into_iter()
                .map(|d| d.id)
                .collect()
        };
        assert_eq!(
            ids(DecisionOwner::issue(issue.id)),
            [own, project, eq, delay, reverb, root]
        );
        assert_eq!(
            ids(DecisionOwner::label("acme/reverb/EQ")),
            [eq, reverb, root]
        );
        assert_eq!(ids(DecisionOwner::label("acme")), [root]);
        assert_eq!(ids(DecisionOwner::project("v2")), [project]);
        // Ended ones show only while awaiting review.
        assert!(
            db.shown_decisions(&DecisionOwner::label("acme"))
                .unwrap()
                .iter()
                .all(|d| d.id != ended)
        );

        // Renames carry decisions along; deleting the owner deletes them.
        db.rename_label("acme", "globex/acme").unwrap();
        assert_eq!(
            db.get_decision(root).unwrap().owner,
            DecisionOwner::label("globex/acme")
        );
        db.delete_label("globex").unwrap();
        assert!(db.get_decision(eq).is_err());
        db.delete_issue(issue.id).unwrap();
        assert!(db.get_decision(own).is_err());
        db.delete_project("v2").unwrap();
        assert!(db.get_decision(project).is_err());
    }

    #[test]
    fn review() {
        let mut db = Db::open_in_memory().unwrap();
        db.create_label("acme").unwrap();
        let issue = db.create_issue(&NewIssue::new("x")).unwrap();
        let mine = db
            .record_decision(&DecisionOwner::label("acme"), "mine")
            .unwrap();

        db.set_actor(agent());
        let on_label = db
            .record_decision(&DecisionOwner::label("acme"), "agent's")
            .unwrap();
        assert!(!on_label.reviewed);
        assert_eq!(on_label.author, agent());
        let on_issue = db
            .record_decision(&DecisionOwner::issue(issue.id), "on the issue")
            .unwrap();
        assert!(db.get_issue(issue.id).unwrap().unreviewed);
        let activity = &db.unreviewed_activities(issue.id).unwrap()[0];
        assert_eq!(activity.kind, ActivityKind::DecisionRecorded);
        assert_eq!(activity.detail, "on the issue");

        // An agent's withdrawal needs review too.
        let withdrawn = db.withdraw_decision(mine.id).unwrap();
        assert!(!withdrawn.reviewed);
        assert_eq!(withdrawn.ended_by, Some(agent()));
        let ids = |list: Vec<Decision>| list.into_iter().map(|d| d.id).collect::<Vec<_>>();
        assert_eq!(
            ids(db.unreviewed_decisions().unwrap()),
            [mine.id, on_label.id, on_issue.id]
        );

        db.set_actor(Actor::user());
        let confirmed = db.mark_decisions_reviewed(&[on_label.id]).unwrap();
        assert!(confirmed[0].reviewed);
        assert!(db.mark_decisions_reviewed(&[DecisionId(99)]).is_err());
        // Marking the issue reviewed confirms its decisions.
        db.mark_issue_reviewed(issue.id).unwrap();
        assert_eq!(ids(db.unreviewed_decisions().unwrap()), [mine.id]);

        // The app shows the agent's withdrawal until it is confirmed.
        let shown = ids(db.shown_decisions(&DecisionOwner::label("acme")).unwrap());
        assert_eq!(shown, [on_label.id, mine.id]);

        // The user's withdrawal keeps the decision reviewed.
        let kept = db
            .record_decision(&DecisionOwner::label("acme"), "kept")
            .unwrap();
        assert!(db.withdraw_decision(kept.id).unwrap().reviewed);

        // The user withdrawing or replacing an agent's unreviewed decision
        // reviews it: they saw it.
        db.set_actor(agent());
        let to_withdraw = db
            .record_decision(&DecisionOwner::label("acme"), "withdraw me")
            .unwrap();
        let to_replace = db
            .record_decision(&DecisionOwner::label("acme"), "replace me")
            .unwrap();
        db.set_actor(Actor::user());
        assert!(db.withdraw_decision(to_withdraw.id).unwrap().reviewed);
        let replacement = db.replace_decision(to_replace.id, "replaced").unwrap();
        assert!(replacement.reviewed);
        assert!(db.get_decision(to_replace.id).unwrap().reviewed);

        // An agent replacing one leaves the old one's review as it was; the
        // new one waits for the user.
        db.set_actor(agent());
        let newer = db.replace_decision(replacement.id, "newer").unwrap();
        assert!(!newer.reviewed);
        assert!(db.get_decision(replacement.id).unwrap().reviewed);
        assert_eq!(ids(db.unreviewed_decisions().unwrap()), [mine.id, newer.id]);
    }
}
