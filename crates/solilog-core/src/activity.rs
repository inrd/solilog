//! Authorship, lightweight change log (activities), and per-issue review
//! tracking.
//!
//! Every write carries an [`Actor`]: `"user"` in the GUI, `"cli"` by default
//! in the CLI, or an explicit identifier such as `"claude:fix-auth"`. Any
//! non-user write marks the issue unreviewed until the user marks it reviewed
//! (`Db::mark_issue_reviewed`).

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use anyhow::{Context as _, Result};
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use rusqlite::{OptionalExtension as _, Row, params};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::Db;
use crate::filter::IssueFilter;
use crate::issue::{Issue, IssueId};

/// Coalescing window (seconds) for consecutive text-edit activities by the
/// same actor on the same issue (so GUI typing doesn't flood the log).
const ACTIVITY_COALESCE: i64 = 60;

/// Who made a change: `"user"` (the human in the app), `"cli"` (default for
/// CLI writes without `SOLILOG_ACTOR` / `--actor`), or an agent identifier
/// such as `"claude:fix-auth"`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, JsonSchema)]
#[schemars(transparent)]
pub struct Actor(String);

impl Actor {
    pub const USER: &'static str = "user";
    pub const CLI: &'static str = "cli";

    pub fn user() -> Self {
        Self(Self::USER.to_owned())
    }

    pub fn cli() -> Self {
        Self(Self::CLI.to_owned())
    }

    /// Validates and normalizes an actor string: trimmed, single-line, no
    /// control characters, non-empty. `"user"` and `"cli"` are normalized to
    /// lowercase.
    pub fn parse(raw: &str) -> Result<Self> {
        let trimmed = raw.trim();
        anyhow::ensure!(!trimmed.is_empty(), "actor cannot be empty");
        anyhow::ensure!(
            !trimmed.chars().any(char::is_control),
            "invalid actor {raw:?} (control characters are not allowed)"
        );
        if trimmed.eq_ignore_ascii_case(Self::USER) {
            Ok(Self::user())
        } else if trimmed.eq_ignore_ascii_case(Self::CLI) {
            Ok(Self::cli())
        } else {
            Ok(Self(trimmed.to_owned()))
        }
    }

    /// The raw actor string (`"user"`, `"cli"`, `"claude:fix-auth"`, …).
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `true` when this actor is the human user (`"user"`).
    pub fn is_user(&self) -> bool {
        self.0 == Self::USER
    }

    /// `true` for any non-user actor (agents and unidentified `"cli"` writes),
    /// whose changes need review.
    pub fn is_agent(&self) -> bool {
        !self.is_user()
    }
}

impl Default for Actor {
    fn default() -> Self {
        Self::user()
    }
}

impl fmt::Display for Actor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for Actor {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).map_err(|e| e.to_string())
    }
}

impl Serialize for Actor {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Actor {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

impl ToSql for Actor {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        self.0.to_sql()
    }
}

impl FromSql for Actor {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let s = value.as_str()?;
        Self::parse(s).map_err(|e| FromSqlError::Other(e.into()))
    }
}

/// Identifies a recorded activity entry, across all issues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ActivityId(pub i64);

impl fmt::Display for ActivityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl ToSql for ActivityId {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        self.0.to_sql()
    }
}

impl FromSql for ActivityId {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        i64::column_result(value).map(ActivityId)
    }
}

/// What kind of write an [`Activity`] records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityKind {
    Created,
    TitleEdited,
    DescriptionEdited,
    StatusChanged,
    Archived,
    Unarchived,
    LabelsAdded,
    LabelsRemoved,
    ProjectsAdded,
    ProjectsRemoved,
    DependenciesAdded,
    DependenciesRemoved,
    ChecklistAdded,
    ChecklistEdited,
    ChecklistChecked,
    ChecklistUnchecked,
    ChecklistDeleted,
    AttachmentAdded,
    AttachmentRemoved,
    RevisionRestored,
    NoteAdded,
    NoteDeleted,
    ReviewRequested,
    Approved,
    SentBack,
    DecisionRecorded,
    DecisionReplaced,
    DecisionWithdrawn,
    LinkAdded,
    LinkRemoved,
    Declined,
}

impl ActivityKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::TitleEdited => "title_edited",
            Self::DescriptionEdited => "description_edited",
            Self::StatusChanged => "status_changed",
            Self::Archived => "archived",
            Self::Unarchived => "unarchived",
            Self::LabelsAdded => "labels_added",
            Self::LabelsRemoved => "labels_removed",
            Self::ProjectsAdded => "projects_added",
            Self::ProjectsRemoved => "projects_removed",
            Self::DependenciesAdded => "dependencies_added",
            Self::DependenciesRemoved => "dependencies_removed",
            Self::ChecklistAdded => "checklist_added",
            Self::ChecklistEdited => "checklist_edited",
            Self::ChecklistChecked => "checklist_checked",
            Self::ChecklistUnchecked => "checklist_unchecked",
            Self::ChecklistDeleted => "checklist_deleted",
            Self::AttachmentAdded => "attachment_added",
            Self::AttachmentRemoved => "attachment_removed",
            Self::RevisionRestored => "revision_restored",
            Self::NoteAdded => "note_added",
            Self::NoteDeleted => "note_deleted",
            Self::ReviewRequested => "review_requested",
            Self::Approved => "approved",
            Self::SentBack => "sent_back",
            Self::DecisionRecorded => "decision_recorded",
            Self::DecisionReplaced => "decision_replaced",
            Self::DecisionWithdrawn => "decision_withdrawn",
            Self::LinkAdded => "link_added",
            Self::LinkRemoved => "link_removed",
            Self::Declined => "declined",
        }
    }

    /// Every kind, in declaration order.
    pub(crate) const ALL: &[Self] = &[
        Self::Created,
        Self::TitleEdited,
        Self::DescriptionEdited,
        Self::StatusChanged,
        Self::Archived,
        Self::Unarchived,
        Self::LabelsAdded,
        Self::LabelsRemoved,
        Self::ProjectsAdded,
        Self::ProjectsRemoved,
        Self::DependenciesAdded,
        Self::DependenciesRemoved,
        Self::ChecklistAdded,
        Self::ChecklistEdited,
        Self::ChecklistChecked,
        Self::ChecklistUnchecked,
        Self::ChecklistDeleted,
        Self::AttachmentAdded,
        Self::AttachmentRemoved,
        Self::RevisionRestored,
        Self::NoteAdded,
        Self::NoteDeleted,
        Self::ReviewRequested,
        Self::Approved,
        Self::SentBack,
        Self::DecisionRecorded,
        Self::DecisionReplaced,
        Self::DecisionWithdrawn,
        Self::LinkAdded,
        Self::LinkRemoved,
        Self::Declined,
    ];

    fn from_key(key: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|k| k.key() == key)
    }

    /// Consecutive edits of the same kind by the same actor within
    /// [`ACTIVITY_COALESCE`] update the latest row rather than flooding the log.
    fn is_coalescable(self) -> bool {
        matches!(
            self,
            Self::TitleEdited | Self::DescriptionEdited | Self::ChecklistEdited
        )
    }
}

/// One recorded write on an issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Activity {
    pub id: ActivityId,
    pub issue_id: IssueId,
    pub actor: Actor,
    pub kind: ActivityKind,
    /// Short human-readable detail (e.g. `"todo → in_progress"`, label names,
    /// checklist item text). Empty when the kind says enough.
    pub detail: String,
    /// Unix timestamp (seconds).
    pub created_at: i64,
}

impl Activity {
    /// Compact description of what happened, without the actor or timestamp.
    pub fn summary(&self) -> String {
        let with_detail = |prefix: &str| {
            if self.detail.is_empty() {
                prefix.to_owned()
            } else {
                format!("{prefix}: {}", self.detail)
            }
        };
        match self.kind {
            ActivityKind::Created => "created issue".to_owned(),
            ActivityKind::TitleEdited => with_detail("edited title"),
            ActivityKind::DescriptionEdited => {
                if self.detail.is_empty() {
                    "edited description".to_owned()
                } else {
                    format!("description {}", self.detail)
                }
            }
            ActivityKind::StatusChanged => {
                if self.detail.is_empty() {
                    "changed status".to_owned()
                } else {
                    format!("status {}", self.detail)
                }
            }
            ActivityKind::Archived => "archived issue".to_owned(),
            ActivityKind::Unarchived => "unarchived issue".to_owned(),
            ActivityKind::LabelsAdded => with_detail("added labels"),
            ActivityKind::LabelsRemoved => with_detail("removed labels"),
            ActivityKind::ProjectsAdded => with_detail("added to projects"),
            ActivityKind::ProjectsRemoved => with_detail("removed from projects"),
            ActivityKind::DependenciesAdded => with_detail("added dependencies"),
            ActivityKind::DependenciesRemoved => with_detail("removed dependencies"),
            ActivityKind::ChecklistAdded => with_detail("added checklist"),
            ActivityKind::ChecklistEdited => with_detail("edited checklist"),
            ActivityKind::ChecklistChecked => with_detail("checked"),
            ActivityKind::ChecklistUnchecked => with_detail("unchecked"),
            ActivityKind::ChecklistDeleted => with_detail("deleted checklist"),
            ActivityKind::AttachmentAdded => with_detail("attached"),
            ActivityKind::AttachmentRemoved => with_detail("removed attachment"),
            ActivityKind::RevisionRestored => with_detail("restored"),
            ActivityKind::NoteAdded => with_detail("added note"),
            ActivityKind::NoteDeleted => with_detail("deleted note"),
            ActivityKind::ReviewRequested => with_detail("requested review"),
            ActivityKind::Approved => with_detail("approved"),
            ActivityKind::SentBack => with_detail("sent back"),
            ActivityKind::DecisionRecorded => with_detail("recorded decision"),
            ActivityKind::DecisionReplaced => with_detail("replaced decision"),
            ActivityKind::DecisionWithdrawn => with_detail("withdrew decision"),
            ActivityKind::LinkAdded => with_detail("added link"),
            ActivityKind::LinkRemoved => with_detail("removed link"),
            ActivityKind::Declined => with_detail("declined"),
        }
    }
}

/// Which activities [`Db::activities`] returns. The default is all of them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActivityQuery {
    /// Only on this issue.
    pub issue: Option<IssueId>,
    /// Only at or after this Unix time.
    pub since: Option<i64>,
    /// Only by these actors (any actor if empty).
    pub by: Vec<Actor>,
    /// Not by these actors.
    pub not_by: Vec<Actor>,
    /// Only on issues this filter matches, archived ones included.
    pub issues: Option<IssueFilter>,
    /// At most this many, newest first.
    pub limit: Option<usize>,
}

/// `30m`, `2h`, `3d`, `1w` (also `45s`) in seconds.
fn parse_age(s: &str) -> Option<i64> {
    let unit = match s.chars().last()? {
        's' => 1,
        'm' => 60,
        'h' => 3600,
        'd' => 86_400,
        'w' => 7 * 86_400,
        _ => return None,
    };
    let n: i64 = s[..s.len() - 1].trim().parse().ok().filter(|n| *n >= 0)?;
    n.checked_mul(unit)
}

fn activity_from_row(row: &Row) -> rusqlite::Result<Activity> {
    let kind: String = row.get(3)?;
    Ok(Activity {
        id: row.get(0)?,
        issue_id: row.get(1)?,
        actor: row.get(2)?,
        kind: ActivityKind::from_key(&kind).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Text,
                format!("unknown activity kind {kind:?}").into(),
            )
        })?,
        detail: row.get(4)?,
        created_at: row.get(5)?,
    })
}

impl Db {
    /// Sets the actor attributed to subsequent writes on this handle.
    pub fn with_actor(mut self, actor: Actor) -> Self {
        self.actor = actor;
        self
    }

    /// Changes the actor attributed to subsequent writes on this handle.
    pub fn set_actor(&mut self, actor: Actor) {
        self.actor = actor;
    }

    /// The actor attributed to writes on this handle.
    pub fn actor(&self) -> &Actor {
        &self.actor
    }

    /// All recorded activities on an issue, newest first.
    pub fn issue_activities(&self, issue: IssueId) -> Result<Vec<Activity>> {
        self.issue_state(issue)?;
        Ok(self
            .conn
            .prepare_cached(
                "SELECT id, issue_id, actor, kind, detail, created_at
                 FROM activities
                 WHERE issue_id = ?1
                 ORDER BY id DESC",
            )?
            .query_map([issue], activity_from_row)?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Recorded activities across issues (archived ones included), newest
    /// first, narrowed by `query`. A text edit soon after the same actor's
    /// previous one updates that activity (see [`ACTIVITY_COALESCE`]), so its
    /// `created_at` is the time of the latest edit.
    pub fn activities(&self, query: &ActivityQuery) -> Result<Vec<Activity>> {
        if let Some(issue) = query.issue {
            self.issue_state(issue)?;
        }
        let issues: Option<std::collections::HashSet<IssueId>> = match &query.issues {
            Some(filter) => Some(
                self.list_issues(filter)?
                    .into_iter()
                    .chain(self.list_archived_issues(filter)?)
                    .map(|i| i.id)
                    .collect(),
            ),
            None => None,
        };
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, issue_id, actor, kind, detail, created_at
             FROM activities
             WHERE (?1 IS NULL OR issue_id = ?1) AND (?2 IS NULL OR created_at >= ?2)
             ORDER BY created_at DESC, id DESC",
        )?;
        let mut activities = Vec::new();
        for activity in stmt.query_map(params![query.issue, query.since], activity_from_row)? {
            let activity = activity?;
            if (query.by.is_empty() || query.by.contains(&activity.actor))
                && !query.not_by.contains(&activity.actor)
                && issues
                    .as_ref()
                    .is_none_or(|ids| ids.contains(&activity.issue_id))
            {
                activities.push(activity);
                if query.limit.is_some_and(|limit| activities.len() >= limit) {
                    break;
                }
            }
        }
        Ok(activities)
    }

    /// A time given by a person or an agent, as a Unix timestamp: a number
    /// (already one), an age before now (`30m`, `2h`, `3d`, `1w`), or a
    /// date and time (`2026-11-25`, `2026-11-25 14:30`,
    /// `2026-11-25T14:30:00`), in local time unless it ends with `Z` or an
    /// offset (`+02:00`).
    pub fn parse_time(&self, raw: &str) -> Result<i64> {
        let s = raw.trim();
        if let Ok(timestamp) = s.parse::<i64>() {
            return Ok(timestamp);
        }
        if let Some(age) = parse_age(s) {
            let now: i64 = self
                .conn
                .query_row("SELECT unixepoch()", [], |r| r.get(0))?;
            return Ok(now - age);
        }
        // SQLite reads the formats above; `utc` takes a time without a zone
        // as local time.
        let has_zone = s.ends_with(['Z', 'z'])
            || s.rfind(['+', '-'])
                .is_some_and(|i| i > 10 && s[i + 1..].contains(':'));
        let sql = if has_zone {
            "SELECT unixepoch(?1)"
        } else {
            "SELECT unixepoch(?1, 'utc')"
        };
        let timestamp: Option<i64> = self.conn.query_row(sql, [s], |r| r.get(0))?;
        timestamp.with_context(|| {
            format!(
                "invalid time {raw:?}: give a Unix timestamp, an age (30m, 2h, 3d, 1w) \
                 or a date and time (2026-11-25, 2026-11-25 14:30)"
            )
        })
    }

    /// Non-user activities on an issue since it was last marked reviewed,
    /// newest first.
    pub fn unreviewed_activities(&self, issue: IssueId) -> Result<Vec<Activity>> {
        self.issue_state(issue)?;
        Ok(self
            .conn
            .prepare_cached(
                "SELECT a.id, a.issue_id, a.actor, a.kind, a.detail, a.created_at
                 FROM activities a
                 JOIN issues i ON i.id = a.issue_id
                 WHERE a.issue_id = ?1
                   AND a.id > i.reviewed_activity_id
                   AND a.actor != ?2
                 ORDER BY a.id DESC",
            )?
            .query_map(params![issue, Actor::USER], activity_from_row)?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Marks an issue as reviewed up to its latest recorded activity and
    /// returns the updated issue.
    pub fn mark_issue_reviewed(&self, id: IssueId) -> Result<Issue> {
        let mut issues = self.mark_issues_reviewed(&[id])?;
        Ok(issues.remove(0))
    }

    /// Marks every given issue as reviewed, all or nothing. Returns the
    /// updated issues in the order given.
    pub fn mark_issues_reviewed(&self, ids: &[IssueId]) -> Result<Vec<Issue>> {
        let tx = self.conn.unchecked_transaction()?;
        for &id in ids {
            self.set_reviewed(id)?;
        }
        tx.commit()?;
        ids.iter().map(|&id| self.get_issue(id)).collect()
    }

    /// Marks an issue as reviewed up to its latest activity. Call inside a
    /// transaction.
    pub(crate) fn set_reviewed(&self, id: IssueId) -> Result<()> {
        let updated = self.conn.execute(
            "UPDATE issues
             SET reviewed_at = unixepoch(),
                 reviewed_activity_id = coalesce(
                     (SELECT max(id) FROM activities WHERE issue_id = ?1),
                     reviewed_activity_id
                 )
             WHERE id = ?1",
            [id],
        )?;
        anyhow::ensure!(updated == 1, "no issue {id}");
        self.set_issue_decisions_reviewed(id)?;
        Ok(())
    }

    /// Marks every non-archived unreviewed issue as reviewed, all or nothing,
    /// and returns the updated issues in list order.
    pub fn mark_all_issues_reviewed(&self) -> Result<Vec<Issue>> {
        let ids: Vec<IssueId> = self
            .list_issues(&IssueFilter {
                unreviewed: true,
                ..Default::default()
            })?
            .into_iter()
            .map(|i| i.id)
            .collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        self.mark_issues_reviewed(&ids)
    }

    /// Records a write on `issue` by `self.actor` and updates `updated_by` /
    /// `updated_at`. Call inside a transaction.
    pub(crate) fn record_activity(
        &self,
        issue: IssueId,
        kind: ActivityKind,
        detail: &str,
    ) -> Result<()> {
        let coalesced = if kind.is_coalescable() {
            let latest: Option<(i64, String, String, i64, i64)> = self
                .conn
                .query_row(
                    "SELECT id, actor, kind, created_at, unixepoch()
                     FROM activities
                     WHERE issue_id = ?1
                     ORDER BY id DESC
                     LIMIT 1",
                    [issue],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                        ))
                    },
                )
                .optional()?;
            if let Some((id, actor, latest_kind, created_at, now)) = latest
                && actor == self.actor.as_str()
                && latest_kind == kind.key()
                && now - created_at < ACTIVITY_COALESCE
            {
                self.conn.execute(
                    "UPDATE activities SET detail = ?2, created_at = unixepoch() WHERE id = ?1",
                    params![id, detail],
                )?;
                true
            } else {
                false
            }
        } else {
            false
        };

        if !coalesced {
            self.conn.execute(
                "INSERT INTO activities (issue_id, actor, kind, detail)
                 VALUES (?1, ?2, ?3, ?4)",
                params![issue, self.actor, kind.key(), detail],
            )?;
        }

        self.conn.execute(
            "UPDATE issues SET updated_by = ?2, updated_at = unixepoch() WHERE id = ?1",
            params![issue, self.actor],
        )?;
        Ok(())
    }

    /// Distinct non-user actors with unreviewed activities per issue, ordered
    /// most-recent first.
    pub(crate) fn unreviewed_actors_per_issue(
        &self,
        only: Option<IssueId>,
    ) -> Result<HashMap<IssueId, Vec<Actor>>> {
        let mut map: HashMap<IssueId, Vec<Actor>> = HashMap::new();
        let mut stmt = self.conn.prepare_cached(
            "SELECT a.issue_id, a.actor
             FROM activities a
             JOIN issues i ON i.id = a.issue_id
             WHERE (?1 IS NULL OR a.issue_id = ?1)
               AND a.id > i.reviewed_activity_id
               AND a.actor != ?2
             ORDER BY a.issue_id, a.id DESC",
        )?;
        let rows = stmt.query_map(params![only, Actor::USER], |row| {
            Ok((row.get::<_, IssueId>(0)?, row.get::<_, Actor>(1)?))
        })?;
        for row in rows {
            let (issue_id, actor) = row?;
            let list = map.entry(issue_id).or_default();
            if !list.contains(&actor) {
                list.push(actor);
            }
        }
        Ok(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IssueChange, IssueEdit, IssueFilter, NewIssue, Status};

    #[test]
    fn actor_parsing() {
        assert_eq!(Actor::parse("user").unwrap(), Actor::user());
        assert_eq!(Actor::parse("  USER  ").unwrap(), Actor::user());
        assert_eq!(Actor::parse("CLI").unwrap(), Actor::cli());
        assert!(Actor::user().is_user());
        assert!(!Actor::user().is_agent());
        assert!(Actor::cli().is_agent());
        let claude = Actor::parse("claude:fix-auth").unwrap();
        assert_eq!(claude.as_str(), "claude:fix-auth");
        assert!(claude.is_agent());
        assert!(Actor::parse("  ").is_err());
        assert!(Actor::parse("claude\nfix").is_err());
    }

    #[test]
    fn agent_writes_are_unreviewed_until_marked() {
        let mut db = Db::open_in_memory().unwrap();
        // User-created issue starts reviewed.
        let user_issue = db.create_issue(&NewIssue::new("user issue")).unwrap();
        assert_eq!(user_issue.created_by, Actor::user());
        assert!(!user_issue.unreviewed);
        assert!(user_issue.unreviewed_actors.is_empty());

        // Agent-created issue is unreviewed.
        let agent = Actor::parse("claude:fix-auth").unwrap();
        db.set_actor(agent.clone());
        let agent_issue = db
            .create_issue(&NewIssue::new("agent issue").checklist(["step 1"]))
            .unwrap();
        assert_eq!(agent_issue.created_by, agent);
        assert_eq!(agent_issue.title_by, agent);
        assert_eq!(agent_issue.checklist[0].author, agent);
        assert!(agent_issue.unreviewed);
        assert_eq!(agent_issue.unreviewed_actors, vec![agent.clone()]);

        // Filter for unreviewed finds only the agent issue.
        let unreviewed = db
            .list_issues(&IssueFilter {
                unreviewed: true,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(unreviewed.len(), 1);
        assert_eq!(unreviewed[0].id, agent_issue.id);

        // User editing the issue does NOT silently mark it reviewed.
        db.set_actor(Actor::user());
        let after_user_edit = db
            .update_issue(agent_issue.id, &IssueChange::SetStatus(Status::InProgress))
            .unwrap();
        assert!(after_user_edit.unreviewed);
        assert_eq!(after_user_edit.updated_by, Actor::user());

        // Marking reviewed clears unreviewed state.
        let reviewed = db.mark_issue_reviewed(agent_issue.id).unwrap();
        assert!(!reviewed.unreviewed);
        assert!(reviewed.reviewed_at.is_some());
        assert!(db.unreviewed_activities(agent_issue.id).unwrap().is_empty());

        // A subsequent agent write (even in the same second!) marks it unreviewed again.
        db.set_actor(Actor::cli());
        let edited = db
            .edit_issue(
                agent_issue.id,
                &IssueEdit {
                    description: Some("agent notes".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(edited.unreviewed);
        assert_eq!(edited.description_by, Actor::cli());
        assert_eq!(edited.unreviewed_actors, vec![Actor::cli()]);
        let pending = db.unreviewed_activities(agent_issue.id).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].kind, ActivityKind::DescriptionEdited);
    }

    #[test]
    fn activities_record_all_write_kinds_and_coalesce_edits() {
        let mut db = Db::open_in_memory().unwrap();
        let agent = Actor::parse("codex:triage").unwrap();
        db.set_actor(agent.clone());

        let issue = db
            .create_issue(&NewIssue::new("triage").checklist(["item"]))
            .unwrap();
        let item_id = issue.checklist[0].id;
        // Consecutive description edits by the same actor coalesce.
        db.edit_issue(
            issue.id,
            &IssueEdit {
                description: Some("first".into()),
                ..Default::default()
            },
        )
        .unwrap();
        db.edit_issue(
            issue.id,
            &IssueEdit {
                description: Some("second".into()),
                ..Default::default()
            },
        )
        .unwrap();
        db.set_status(issue.id, Status::InProgress).unwrap();
        db.set_checklist_item_done(item_id, true).unwrap();
        db.update_issue(issue.id, &IssueChange::AddLabels(vec!["bug".into()]))
            .unwrap();

        let activities = db.issue_activities(issue.id).unwrap();
        let kinds: Vec<ActivityKind> = activities.iter().map(|a| a.kind).collect();
        assert_eq!(
            kinds,
            vec![
                ActivityKind::LabelsAdded,
                ActivityKind::ChecklistChecked,
                ActivityKind::StatusChanged,
                ActivityKind::DescriptionEdited,
                ActivityKind::Created,
            ]
        );
        assert_eq!(activities.len(), 5);
    }

    #[test]
    fn different_actors_do_not_coalesce_revisions() {
        let mut db = Db::open_in_memory().unwrap();
        let id = db
            .create_issue(&NewIssue::new("x").description("v0"))
            .unwrap()
            .id;
        db.edit_issue(
            id,
            &IssueEdit {
                description: Some("v1".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let agent = Actor::parse("claude:edit").unwrap();
        db.set_actor(agent.clone());
        db.edit_issue(
            id,
            &IssueEdit {
                description: Some("v2".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let revisions = db.issue_revisions(id).unwrap();
        assert_eq!(revisions.len(), 2);
        assert_eq!(revisions[0].text, "v1");
        assert_eq!(revisions[0].author, agent);
        assert_eq!(revisions[1].text, "v0");
        assert_eq!(revisions[1].author, Actor::user());
    }

    #[test]
    fn activities_across_issues() {
        let mut db = Db::open_in_memory().unwrap();
        let acme = db
            .create_issue(&NewIssue::new("acme").labels(["acme"]))
            .unwrap();
        let agent = Actor::parse("claude:triage").unwrap();
        db.set_actor(agent.clone());
        let other = db.create_issue(&NewIssue::new("other")).unwrap();
        db.set_status(acme.id, Status::InProgress).unwrap();
        db.set_archived(other.id, true).unwrap();
        // Long ago.
        db.conn
            .execute(
                "UPDATE activities SET created_at = 1000 WHERE issue_id = ?1 AND kind = 'created'",
                [acme.id],
            )
            .unwrap();

        let kinds = |q: &ActivityQuery| -> Vec<(IssueId, ActivityKind)> {
            db.activities(q)
                .unwrap()
                .into_iter()
                .map(|a| (a.issue_id, a.kind))
                .collect()
        };
        assert_eq!(kinds(&ActivityQuery::default()).len(), 4);
        // Newest first; the archived issue's activities are there.
        assert_eq!(
            kinds(&ActivityQuery {
                since: Some(2000),
                ..Default::default()
            }),
            [
                (other.id, ActivityKind::Archived),
                (acme.id, ActivityKind::StatusChanged),
                (other.id, ActivityKind::Created),
            ]
        );
        assert_eq!(
            kinds(&ActivityQuery {
                by: vec![Actor::user()],
                ..Default::default()
            }),
            [(acme.id, ActivityKind::Created)]
        );
        assert_eq!(
            kinds(&ActivityQuery {
                not_by: vec![agent.clone()],
                ..Default::default()
            }),
            [(acme.id, ActivityKind::Created)]
        );
        assert_eq!(
            kinds(&ActivityQuery {
                issues: Some(IssueFilter {
                    labels: vec!["acme".into()],
                    ..Default::default()
                }),
                ..Default::default()
            }),
            [
                (acme.id, ActivityKind::StatusChanged),
                (acme.id, ActivityKind::Created),
            ]
        );
        assert_eq!(
            kinds(&ActivityQuery {
                issue: Some(other.id),
                limit: Some(1),
                ..Default::default()
            }),
            [(other.id, ActivityKind::Archived)]
        );
        assert!(
            db.activities(&ActivityQuery {
                issue: Some(IssueId(999)),
                ..Default::default()
            })
            .is_err()
        );

        // `updated_since` on issue filters.
        db.conn
            .execute(
                "UPDATE issues SET updated_at = 1000 WHERE id = ?1",
                [acme.id],
            )
            .unwrap();
        let changed = db
            .list_issues(&IssueFilter {
                updated_since: Some(2000),
                ..Default::default()
            })
            .unwrap();
        assert!(changed.is_empty());
        let archived = db
            .list_archived_issues(&IssueFilter {
                updated_since: Some(2000),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(archived.len(), 1);
    }

    #[test]
    fn times() {
        let db = Db::open_in_memory().unwrap();
        let now: i64 = db
            .conn
            .query_row("SELECT unixepoch()", [], |r| r.get(0))
            .unwrap();
        assert_eq!(db.parse_time("1795424326").unwrap(), 1_795_424_326);
        let two_hours = db.parse_time("2h").unwrap();
        assert!((now - 7200 - two_hours).abs() <= 1);
        assert!((now - 3 * 86_400 - db.parse_time(" 3d ").unwrap()).abs() <= 1);
        assert!((now - 604_800 - db.parse_time("1w").unwrap()).abs() <= 1);
        assert!((now - 1800 - db.parse_time("30m").unwrap()).abs() <= 1);
        // With a zone: exact.
        assert_eq!(
            db.parse_time("2026-11-25T00:00:00Z").unwrap(),
            1_795_564_800
        );
        assert_eq!(
            db.parse_time("2026-11-25T02:00:00+02:00").unwrap(),
            1_795_564_800
        );
        // Without one: local time, within a day of UTC.
        let local = db.parse_time("2026-11-25").unwrap();
        assert!((local - 1_795_564_800).abs() <= 14 * 3600);
        assert_eq!(
            db.parse_time("2026-11-25 10:30").unwrap() - local,
            10 * 3600 + 30 * 60
        );
        for bad in ["", "yesterday", "2h ago", "-3d", "2026-13-45"] {
            assert!(db.parse_time(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn mark_all_issues_reviewed_clears_non_archived_unreviewed() {
        let mut db = Db::open_in_memory().unwrap();
        let user_issue = db.create_issue(&NewIssue::new("user")).unwrap();

        db.set_actor(Actor::parse("claude:triage").unwrap());
        let first = db.create_issue(&NewIssue::new("first")).unwrap();
        let second = db
            .create_issue(&NewIssue::new("second").status(Status::Done))
            .unwrap();
        let archived = db.create_issue(&NewIssue::new("archived")).unwrap();
        db.set_archived(archived.id, true).unwrap();

        db.set_actor(Actor::user());
        let marked = db.mark_all_issues_reviewed().unwrap();
        assert_eq!(marked.len(), 2);
        assert!(marked.iter().all(|i| !i.unreviewed));
        let marked_ids: Vec<_> = marked.iter().map(|i| i.id).collect();
        assert!(marked_ids.contains(&first.id));
        assert!(marked_ids.contains(&second.id));
        assert!(!marked_ids.contains(&user_issue.id));
        assert!(!marked_ids.contains(&archived.id));

        assert!(
            db.list_issues(&IssueFilter {
                unreviewed: true,
                ..Default::default()
            })
            .unwrap()
            .is_empty()
        );
        let archived_list = db.list_archived_issues(&IssueFilter::default()).unwrap();
        assert_eq!(archived_list.len(), 1);
        assert!(archived_list[0].unreviewed);

        assert!(db.mark_all_issues_reviewed().unwrap().is_empty());
    }

    /// Every kind of activity has a write that records it, and that write
    /// does record it. The match has no catch-all arm: a new kind doesn't
    /// compile until it has a step here.
    #[test]
    fn every_activity_kind_is_recorded_by_its_write() {
        use crate::{DecisionOwner, Receipt};

        let dir = std::env::temp_dir().join(format!("solilog-activities-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("file.txt");
        std::fs::write(&file, "content").unwrap();
        let mut db = Db::open(&dir.join("test.db")).unwrap();
        let other = db.create_issue(&NewIssue::new("other")).unwrap().id;
        let mut id = IssueId(0);
        let (mut item, mut attachment, mut note, mut decision, mut link) =
            (None, None, None, None, None);
        let review = |db: &Db, id| db.request_review(id, "done", &Receipt::default()).unwrap();

        // The id of the newest activity on `id` (0 before it exists).
        let newest = |db: &Db, id| {
            db.issue_activities(id)
                .map(|a| a.first().map_or(0, |a| a.id.0))
                .unwrap_or(0)
        };

        for &kind in ActivityKind::ALL {
            let before = newest(&db, id);
            match kind {
                ActivityKind::Created => id = db.create_issue(&NewIssue::new("a")).unwrap().id,
                ActivityKind::TitleEdited => {
                    let edit = IssueEdit {
                        title: Some("b".into()),
                        ..Default::default()
                    };
                    db.edit_issue(id, &edit).unwrap();
                }
                ActivityKind::DescriptionEdited => {
                    let edit = IssueEdit {
                        description: Some("text".into()),
                        ..Default::default()
                    };
                    db.edit_issue(id, &edit).unwrap();
                }
                ActivityKind::StatusChanged => drop(db.set_status(id, Status::Blocked).unwrap()),
                ActivityKind::Archived => drop(db.set_archived(id, true).unwrap()),
                ActivityKind::Unarchived => drop(db.set_archived(id, false).unwrap()),
                ActivityKind::LabelsAdded => {
                    db.update_issue(id, &IssueChange::AddLabels(vec!["x".into()]))
                        .unwrap();
                }
                ActivityKind::LabelsRemoved => {
                    db.update_issue(id, &IssueChange::RemoveLabels(vec!["x".into()]))
                        .unwrap();
                }
                ActivityKind::ProjectsAdded => {
                    db.create_project(&crate::NewProject::new("p")).unwrap();
                    db.update_issue(id, &IssueChange::AddToProjects(vec!["p".into()]))
                        .unwrap();
                }
                ActivityKind::ProjectsRemoved => {
                    db.update_issue(id, &IssueChange::RemoveFromProjects(vec!["p".into()]))
                        .unwrap();
                }
                ActivityKind::DependenciesAdded => drop(db.add_dependencies(id, &[other]).unwrap()),
                ActivityKind::DependenciesRemoved => {
                    db.update_issue(id, &IssueChange::RemoveDependencies(vec![other]))
                        .unwrap();
                }
                ActivityKind::ChecklistAdded => {
                    let issue = db.add_checklist_items(id, &["item".into()]).unwrap();
                    item = Some(issue.checklist[0].id);
                }
                ActivityKind::ChecklistEdited => {
                    db.edit_checklist_item(item.unwrap(), "edited").unwrap();
                }
                ActivityKind::ChecklistChecked => {
                    db.set_checklist_item_done(item.unwrap(), true).unwrap();
                }
                ActivityKind::ChecklistUnchecked => {
                    db.set_checklist_item_done(item.unwrap(), false).unwrap();
                }
                ActivityKind::ChecklistDeleted => {
                    db.delete_checklist_item(item.unwrap()).unwrap();
                }
                ActivityKind::AttachmentAdded => {
                    attachment =
                        Some(db.add_attachments(id, std::slice::from_ref(&file)).unwrap()[0].id);
                }
                ActivityKind::AttachmentRemoved => {
                    db.delete_attachment(attachment.unwrap()).unwrap();
                }
                ActivityKind::RevisionRestored => {
                    let revision = db.issue_revisions(id).unwrap()[0].id;
                    db.restore_revision(revision).unwrap();
                }
                ActivityKind::NoteAdded => note = Some(db.add_note(id, "note").unwrap().id),
                ActivityKind::NoteDeleted => drop(db.delete_note(note.unwrap()).unwrap()),
                ActivityKind::ReviewRequested => drop(review(&db, id)),
                ActivityKind::Approved => drop(db.approve_issues(&[id], "").unwrap()),
                ActivityKind::SentBack => {
                    db.set_status(id, Status::InProgress).unwrap();
                    review(&db, id);
                    db.send_back_issues(&[id], "again").unwrap();
                }
                ActivityKind::DecisionRecorded => {
                    let owner = DecisionOwner::Issue { id };
                    decision = Some(db.record_decision(&owner, "keep it").unwrap().id);
                }
                ActivityKind::DecisionReplaced => {
                    decision = Some(
                        db.replace_decision(decision.unwrap(), "change it")
                            .unwrap()
                            .id,
                    );
                }
                ActivityKind::DecisionWithdrawn => {
                    db.withdraw_decision(decision.unwrap()).unwrap();
                }
                ActivityKind::LinkAdded => {
                    let issue = db.add_link(id, &crate::NewLink::new("main")).unwrap();
                    link = Some(issue.links[0].id);
                }
                ActivityKind::LinkRemoved => drop(db.remove_link(link.unwrap()).unwrap()),
                ActivityKind::Declined => {
                    // Only an issue an agent filed can be declined.
                    db.set_actor(Actor::parse("claude:x").unwrap());
                    id = db.create_issue(&NewIssue::new("filed")).unwrap().id;
                    db.set_actor(Actor::user());
                    db.decline_issues(&[id], "").unwrap();
                }
            }
            // A write may record more than one (requesting a review also
            // changes the status).
            let recorded: Vec<ActivityKind> = db
                .issue_activities(id)
                .unwrap()
                .into_iter()
                .take_while(|a| a.id.0 > before)
                .map(|a| a.kind)
                .collect();
            assert!(
                recorded.contains(&kind),
                "the write for {} recorded {recorded:?}",
                kind.key()
            );
        }
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
