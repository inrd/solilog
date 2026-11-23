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

use anyhow::Result;
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
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        const ALL: &[ActivityKind] = &[
            ActivityKind::Created,
            ActivityKind::TitleEdited,
            ActivityKind::DescriptionEdited,
            ActivityKind::StatusChanged,
            ActivityKind::Archived,
            ActivityKind::Unarchived,
            ActivityKind::LabelsAdded,
            ActivityKind::LabelsRemoved,
            ActivityKind::ProjectsAdded,
            ActivityKind::ProjectsRemoved,
            ActivityKind::DependenciesAdded,
            ActivityKind::DependenciesRemoved,
            ActivityKind::ChecklistAdded,
            ActivityKind::ChecklistEdited,
            ActivityKind::ChecklistChecked,
            ActivityKind::ChecklistUnchecked,
            ActivityKind::ChecklistDeleted,
            ActivityKind::AttachmentAdded,
            ActivityKind::AttachmentRemoved,
            ActivityKind::RevisionRestored,
            ActivityKind::NoteAdded,
            ActivityKind::NoteDeleted,
            ActivityKind::ReviewRequested,
            ActivityKind::Approved,
            ActivityKind::SentBack,
        ];
        ALL.iter().copied().find(|k| k.key() == key)
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
        }
    }
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
        self.get_issue(issue)?;
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

    /// Non-user activities on an issue since it was last marked reviewed,
    /// newest first.
    pub fn unreviewed_activities(&self, issue: IssueId) -> Result<Vec<Activity>> {
        self.get_issue(issue)?;
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
}
