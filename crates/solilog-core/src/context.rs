//! Single-call context bundling for an issue and its immediate neighborhood.

use std::collections::HashSet;

use anyhow::Result;
use rusqlite::OptionalExtension as _;
use serde::Serialize;

use crate::Db;
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

/// An issue together with its projects, dependencies, referenced issues and
/// notes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueContext {
    pub issue: Issue,
    pub projects: Vec<ProjectSummary>,
    pub depends_on: Vec<IssueSummary>,
    pub referenced_issues: Vec<IssueSummary>,
    /// The issue's worklog, oldest first.
    pub notes: Vec<Note>,
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
    /// any existing `SOL-n` issues referenced in its description, and its
    /// notes.
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

        let notes = self.issue_notes(id)?;
        Ok(IssueContext {
            issue,
            projects,
            depends_on,
            referenced_issues,
            notes,
        })
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
