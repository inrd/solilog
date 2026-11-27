//! Single-call context bundling for an issue and its immediate neighborhood.

use std::collections::HashSet;

use anyhow::Result;
use rusqlite::OptionalExtension as _;
use serde::Serialize;

use crate::Db;
use crate::decision::{Decision, DecisionOwner};
use crate::issue::{Issue, IssueId, Status, issue_mentions};
use crate::note::Note;
use crate::project::ProjectStatus;

const SUMMARY_MAX_CHARS: usize = 160;

/// Compact representation of a project in [`IssueContext`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectSummary {
    pub name: String,
    pub status: ProjectStatus,
    pub description: String,
}

/// Compact representation of a related issue in [`IssueContext`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueSummary {
    pub id: IssueId,
    pub title: String,
    pub status: Status,
    pub summary: String,
}

/// An issue together with its projects, dependencies, the issues it
/// mentions and those that mention it, notes and the decisions that apply to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueContext {
    pub issue: Issue,
    pub projects: Vec<ProjectSummary>,
    pub depends_on: Vec<IssueSummary>,
    pub referenced_issues: Vec<IssueSummary>,
    /// The other issues that mention this one (`SOL-n`) in their
    /// description, checklist or notes (see `Db::mentioned_by`).
    pub mentioned_by: Vec<IssueSummary>,
    /// The issue's worklog, oldest first.
    pub notes: Vec<Note>,
    /// The decisions in force that apply to the issue: its own, its
    /// projects', its labels' and their ancestors' (most specific first;
    /// see `Db::inherited_decisions`).
    pub decisions: Vec<Decision>,
}

fn brief_summary(description: &str) -> String {
    let text = description.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= SUMMARY_MAX_CHARS {
        return text;
    }
    let cut: String = text
        .chars()
        .take(SUMMARY_MAX_CHARS.saturating_sub(1))
        .collect();
    format!("{}…", cut.trim_end())
}

impl Db {
    /// Returns an issue together with its projects, `depends_on` issues, and
    /// any existing `SOL-n` issues referenced in its description, the
    /// issues that mention it, its notes and the decisions that apply to it.
    pub fn issue_context(&self, id: IssueId) -> Result<IssueContext> {
        let issue = self.get_issue(id)?;

        let mut projects = Vec::with_capacity(issue.projects.len());
        for name in &issue.projects {
            let project = self.get_project(name)?;
            projects.push(ProjectSummary {
                name: project.name,
                status: project.status,
                description: project.description,
            });
        }

        let mut depends_on = Vec::with_capacity(issue.depends_on.len());
        for &dep_id in &issue.depends_on {
            if let Some(summary) = self.issue_summary(dep_id)? {
                depends_on.push(summary);
            }
        }

        let mut seen = HashSet::from([issue.id]);
        let mut referenced_issues = Vec::new();
        for (_, mentioned_id) in issue_mentions(&issue.description) {
            if seen.insert(mentioned_id)
                && let Some(summary) = self.issue_summary(mentioned_id)?
            {
                referenced_issues.push(summary);
            }
        }

        let mut mentioned_by = Vec::new();
        for other in self.mentioned_by(id)? {
            if let Some(summary) = self.issue_summary(other)? {
                mentioned_by.push(summary);
            }
        }

        let notes = self.issue_notes(id)?;
        let decisions = self.inherited_decisions(&DecisionOwner::issue(id))?;
        Ok(IssueContext {
            issue,
            projects,
            depends_on,
            referenced_issues,
            mentioned_by,
            notes,
            decisions,
        })
    }

    /// The other issues (archived ones too) that mention `id` as `SOL-n`
    /// in their description, a checklist item or a note, by id.
    pub fn mentioned_by(&self, id: IssueId) -> Result<Vec<IssueId>> {
        // `LIKE` narrows it down (it ignores ASCII case, like mentions);
        // `issue_mentions` then rules out `SOL-120` for `SOL-12` and the like.
        let mut stmt = self.conn.prepare_cached(
            "SELECT issue_id, text FROM (
                 SELECT id AS issue_id, description AS text FROM issues
                 UNION ALL SELECT issue_id, text FROM checklist_items
                 UNION ALL SELECT issue_id, text FROM notes
             )
             WHERE issue_id != ?1 AND text LIKE '%SOL-' || ?1 || '%'
             ORDER BY issue_id",
        )?;
        let rows = stmt.query_map([id], |row| {
            Ok((row.get::<_, IssueId>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut ids: Vec<IssueId> = Vec::new();
        for row in rows {
            let (issue, text) = row?;
            if ids.last() != Some(&issue) && issue_mentions(&text).iter().any(|&(_, m)| m == id) {
                ids.push(issue);
            }
        }
        Ok(ids)
    }

    fn issue_summary(&self, id: IssueId) -> Result<Option<IssueSummary>> {
        self.conn
            .query_row(
                "SELECT id, title, status, description FROM issues WHERE id = ?1",
                [id],
                |row| {
                    let description: String = row.get(3)?;
                    Ok(IssueSummary {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        status: row.get(2)?,
                        summary: brief_summary(&description),
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NewIssue, NewProject};

    #[test]
    fn context_bundles_issue_projects_deps_and_mentions() {
        let db = Db::open_in_memory().unwrap();
        db.create_project(&NewProject {
            name: "alpha".into(),
            status: ProjectStatus::Active,
            description: "Core engine work".into(),
            labels: vec![],
        })
        .unwrap();

        let dep = db
            .create_issue(
                &NewIssue::new("Dependency task")
                    .description("First line.\n\nSecond line.")
                    .status(Status::InProgress),
            )
            .unwrap();
        let ref_issue = db
            .create_issue(
                &NewIssue::new("Referenced task")
                    .description("Some background details")
                    .status(Status::Done),
            )
            .unwrap();
        let main = db
            .create_issue(
                &NewIssue::new("Main task")
                    .description(format!(
                        "See {} and {} again, plus SOL-9999 and self.",
                        ref_issue.id, ref_issue.id
                    ))
                    .projects(["alpha"]),
            )
            .unwrap();
        // Add self-reference to description as well.
        let main = db
            .edit_issue(
                main.id,
                &crate::IssueEdit {
                    description: Some(format!(
                        "See {} and {} again, plus SOL-9999 and {}.",
                        ref_issue.id, ref_issue.id, main.id
                    )),
                    ..Default::default()
                },
            )
            .unwrap();
        db.update_issues(
            &[main.id],
            &crate::IssueChange::AddDependencies(vec![dep.id]),
        )
        .unwrap();

        let ctx = db.issue_context(main.id).unwrap();
        assert_eq!(ctx.issue.id, main.id);
        assert_eq!(ctx.projects.len(), 1);
        assert_eq!(ctx.projects[0].name, "alpha");
        assert_eq!(ctx.projects[0].status, ProjectStatus::Active);
        assert_eq!(ctx.projects[0].description, "Core engine work");

        assert_eq!(ctx.depends_on.len(), 1);
        assert_eq!(ctx.depends_on[0].id, dep.id);
        assert_eq!(ctx.depends_on[0].title, "Dependency task");
        assert_eq!(ctx.depends_on[0].status, Status::InProgress);
        assert_eq!(ctx.depends_on[0].summary, "First line. Second line.");

        assert_eq!(ctx.referenced_issues.len(), 1);
        assert_eq!(ctx.referenced_issues[0].id, ref_issue.id);
        assert_eq!(ctx.referenced_issues[0].title, "Referenced task");
        assert_eq!(ctx.referenced_issues[0].status, Status::Done);
        assert_eq!(ctx.referenced_issues[0].summary, "Some background details");

        assert!(ctx.notes.is_empty());
        let note = db.add_note(main.id, "Started on it.").unwrap();
        assert_eq!(db.issue_context(main.id).unwrap().notes, [note]);

        assert!(ctx.decisions.is_empty());
        let decision = db
            .record_decision(&DecisionOwner::project("alpha"), "No unsafe code.")
            .unwrap();
        assert_eq!(db.issue_context(main.id).unwrap().decisions, [decision]);
    }

    #[test]
    fn mentioned_by_finds_descriptions_checklists_and_notes() {
        let db = Db::open_in_memory().unwrap();
        let target = db.create_issue(&NewIssue::new("Target")).unwrap();
        let t = target.id;
        let by_description = db
            .create_issue(&NewIssue::new("A").description(format!("After {t}, and {t} again.")))
            .unwrap();
        let by_checklist = db
            .create_issue(&NewIssue::new("B").checklist([format!("wait for sol-{}", t.0)]))
            .unwrap();
        let by_note = db.create_issue(&NewIssue::new("C")).unwrap();
        db.add_note(by_note.id, &format!("Handed off to {t}."))
            .unwrap();
        db.update_issues(&[by_note.id], &crate::IssueChange::SetArchived(true))
            .unwrap();
        // Not mentions of `t`: a longer id, and the issue itself.
        db.create_issue(&NewIssue::new("D").description(format!("{t}0 and XSOL-{}", t.0)))
            .unwrap();
        db.edit_issue(
            t,
            &crate::IssueEdit {
                description: Some(format!("This is {t}.")),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(
            db.mentioned_by(t).unwrap(),
            [by_description.id, by_checklist.id, by_note.id]
        );
        let ctx = db.issue_context(t).unwrap();
        let ids: Vec<_> = ctx.mentioned_by.iter().map(|i| i.id).collect();
        assert_eq!(ids, [by_description.id, by_checklist.id, by_note.id]);
        assert_eq!(ctx.mentioned_by[0].title, "A");
        assert!(db.mentioned_by(by_description.id).unwrap().is_empty());
    }

    #[test]
    fn brief_summary_truncates_long_text() {
        let long = "word ".repeat(60);
        let summary = brief_summary(&long);
        assert!(summary.chars().count() <= SUMMARY_MAX_CHARS);
        assert!(summary.ends_with('…'));
    }

    #[test]
    fn context_missing_issue_errors() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.issue_context(IssueId(42)).is_err());
    }
}
