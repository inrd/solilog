//! Projects: bodies of work with a goal and a lifecycle (e.g. "reverb v1.1").
//!
//! An issue can be in any number of projects, or none. A project can have
//! default labels: issues get them when they join the project, and keep them
//! when they leave it. Projects are identified by name (case-insensitive).
//!
//! Projects have a user-defined order (rearranged with [`Db::move_project`]).
//! New projects go last; listings show open projects before closed ones, each
//! in that order.

use std::fmt;
use std::str::FromStr;

use anyhow::{Context as _, Result};
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use rusqlite::{OptionalExtension as _, Row, params};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Db;
use crate::issue::{Issue, IssueId};
use crate::label::normalize_label_name;
use crate::order::{Ordered, Placement};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProjectStatus {
    Planned,
    Active,
    Done,
    Archived,
}

impl ProjectStatus {
    /// All statuses, in display order.
    pub const ALL: [ProjectStatus; 4] = [
        ProjectStatus::Active,
        ProjectStatus::Planned,
        ProjectStatus::Done,
        ProjectStatus::Archived,
    ];

    /// Stable identifier, used in the database, JSON and on the command line.
    pub fn key(self) -> &'static str {
        match self {
            ProjectStatus::Planned => "planned",
            ProjectStatus::Active => "active",
            ProjectStatus::Done => "done",
            ProjectStatus::Archived => "archived",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ProjectStatus::Planned => "Planned",
            ProjectStatus::Active => "Active",
            ProjectStatus::Done => "Done",
            ProjectStatus::Archived => "Archived",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.key() == key)
    }

    /// Done and archived projects are hidden from pickers and default listings.
    pub fn is_closed(self) -> bool {
        matches!(self, ProjectStatus::Done | ProjectStatus::Archived)
    }
}

impl fmt::Display for ProjectStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

impl FromStr for ProjectStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_key(&s.trim().to_lowercase()).ok_or_else(|| {
            let keys: Vec<_> = Self::ALL.iter().map(|s| s.key()).collect();
            format!(
                "unknown project status {s:?} (expected one of: {})",
                keys.join(", ")
            )
        })
    }
}

impl ToSql for ProjectStatus {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.key()))
    }
}

impl FromSql for ProjectStatus {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let key = value.as_str()?;
        ProjectStatus::from_key(key)
            .ok_or_else(|| FromSqlError::Other(format!("unknown project status {key:?}").into()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Project {
    pub name: String,
    pub status: ProjectStatus,
    pub description: String,
    /// Default labels, given to issues when they join the project.
    pub labels: Vec<String>,
    pub issue_count: i64,
    /// Done or cancelled issues.
    pub closed_issue_count: i64,
    /// Unix timestamp (seconds).
    pub created_at: i64,
}

/// Arguments of [`Db::create_project`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewProject {
    pub name: String,
    pub status: ProjectStatus,
    pub description: String,
    /// Default labels; missing labels are created.
    pub labels: Vec<String>,
}

impl NewProject {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: ProjectStatus::Active,
            description: String::new(),
            labels: Vec::new(),
        }
    }
}

/// Changes for [`Db::update_project`]; `None` fields are left alone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectUpdate {
    pub name: Option<String>,
    pub status: Option<ProjectStatus>,
    pub description: Option<String>,
}

fn normalize_project_name(name: &str) -> Result<String> {
    let name = name.trim();
    anyhow::ensure!(!name.is_empty(), "project name cannot be empty");
    anyhow::ensure!(
        !name.chars().any(char::is_control),
        "invalid project name {name:?}: control character"
    );
    Ok(name.to_owned())
}

const PROJECT_COLUMNS: &str = "p.id, p.name, p.status, p.description, p.created_at,
    (SELECT count(*) FROM issue_projects ip WHERE ip.project_id = p.id),
    (SELECT count(*) FROM issue_projects ip JOIN issues i ON i.id = ip.issue_id
        WHERE ip.project_id = p.id AND i.status IN ('done', 'cancelled'))";

fn project_from_row(row: &Row) -> rusqlite::Result<(i64, Project)> {
    Ok((
        row.get(0)?,
        Project {
            name: row.get(1)?,
            status: row.get(2)?,
            description: row.get(3)?,
            created_at: row.get(4)?,
            issue_count: row.get(5)?,
            closed_issue_count: row.get(6)?,
            labels: Vec::new(),
        },
    ))
}

impl Db {
    /// Open projects, then (if `include_closed`) done and archived ones, each
    /// in the user-defined order.
    pub fn list_projects(&self, include_closed: bool) -> Result<Vec<Project>> {
        let rows: Vec<(i64, Project)> = self
            .conn
            .prepare_cached(&format!(
                "SELECT {PROJECT_COLUMNS} FROM projects p
                 ORDER BY p.position, p.id"
            ))?
            .query_map([], project_from_row)?
            .collect::<rusqlite::Result<_>>()?;
        let mut projects = Vec::new();
        for (id, mut project) in rows {
            if include_closed || !project.status.is_closed() {
                project.labels = self.project_labels(id)?;
                projects.push(project);
            }
        }
        projects.sort_by_key(|p| p.status.is_closed());
        Ok(projects)
    }

    pub fn get_project(&self, name: &str) -> Result<Project> {
        let id = self.project_id(name)?;
        let (_, mut project) = self.conn.query_row(
            &format!("SELECT {PROJECT_COLUMNS} FROM projects p WHERE p.id = ?1"),
            [id],
            project_from_row,
        )?;
        project.labels = self.project_labels(id)?;
        Ok(project)
    }

    pub fn create_project(&self, new: &NewProject) -> Result<Project> {
        let name = normalize_project_name(&new.name)?;
        anyhow::ensure!(
            self.find_project(&name)?.is_none(),
            "project {name:?} already exists"
        );
        let labels = new
            .labels
            .iter()
            .map(|l| normalize_label_name(l))
            .collect::<Result<Vec<_>>>()?;
        let tx = self.conn.unchecked_transaction()?;
        self.conn.execute(
            "INSERT INTO projects (name, status, description, position)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                name,
                new.status,
                new.description.trim(),
                self.next_position(Ordered::Projects)?
            ],
        )?;
        let id = self.conn.last_insert_rowid();
        for label in &labels {
            let label_id = self.ensure_label(label)?;
            self.conn.execute(
                "INSERT OR IGNORE INTO project_labels (project_id, label_id) VALUES (?1, ?2)",
                params![id, label_id],
            )?;
        }
        tx.commit()?;
        self.get_project(&name)
    }

    /// Returns the updated project.
    pub fn update_project(&self, name: &str, update: &ProjectUpdate) -> Result<Project> {
        let id = self.project_id(name)?;
        let mut name = self.project_name_by_id(id)?;
        let tx = self.conn.unchecked_transaction()?;
        if let Some(new_name) = &update.name {
            let new_name = normalize_project_name(new_name)?;
            if let Some((other, _)) = self.find_project(&new_name)?
                && other != id
            {
                anyhow::bail!("project {new_name:?} already exists");
            }
            self.conn.execute(
                "UPDATE projects SET name = ?2 WHERE id = ?1",
                params![id, new_name],
            )?;
            self.rewrite_view_filters(|filter| filter.rename_project(&name, &new_name))?;
            name = new_name;
        }
        if let Some(status) = update.status {
            self.conn.execute(
                "UPDATE projects SET status = ?2 WHERE id = ?1",
                params![id, status],
            )?;
        }
        if let Some(description) = &update.description {
            self.conn.execute(
                "UPDATE projects SET description = ?2 WHERE id = ?1",
                params![id, description.trim()],
            )?;
        }
        tx.commit()?;
        self.get_project(&name)
    }

    /// Moves a project before or after another one in the user-defined order.
    /// Returns the moved project.
    pub fn move_project(&self, name: &str, placement: &Placement) -> Result<Project> {
        let id = self.project_id(name)?;
        let anchor = self.project_id(placement.anchor())?;
        let tx = self.conn.unchecked_transaction()?;
        let order: Vec<i64> = self
            .conn
            .prepare_cached("SELECT id FROM projects ORDER BY position, id")?
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        self.place(Ordered::Projects, order, id, anchor, placement)?;
        tx.commit()?;
        self.get_project(&self.project_name_by_id(id)?)
    }

    /// Deletes a project. Its issues stay, with their labels. Returns the
    /// project as it was.
    pub fn delete_project(&self, name: &str) -> Result<Project> {
        let project = self.get_project(name)?;
        self.conn
            .execute("DELETE FROM projects WHERE name = ?1", [&project.name])?;
        Ok(project)
    }

    /// Adds default labels to a project (creating missing labels). Issues
    /// already in the project are not changed. Returns the project.
    pub fn add_project_labels(&self, name: &str, labels: &[String]) -> Result<Project> {
        let id = self.project_id(name)?;
        let tx = self.conn.unchecked_transaction()?;
        for label in labels {
            let label_id = self.ensure_label(&normalize_label_name(label)?)?;
            self.conn.execute(
                "INSERT OR IGNORE INTO project_labels (project_id, label_id) VALUES (?1, ?2)",
                params![id, label_id],
            )?;
        }
        tx.commit()?;
        self.get_project(name)
    }

    /// Removes default labels from a project. Issues keep their labels.
    pub fn remove_project_labels(&self, name: &str, labels: &[String]) -> Result<Project> {
        let id = self.project_id(name)?;
        let tx = self.conn.unchecked_transaction()?;
        for label in labels {
            let label_id = self.label_id(label)?;
            self.conn.execute(
                "DELETE FROM project_labels WHERE project_id = ?1 AND label_id = ?2",
                params![id, label_id],
            )?;
        }
        tx.commit()?;
        self.get_project(name)
    }

    /// Adds an issue to projects, giving it their default labels. Returns the issue.
    pub fn add_to_projects(&self, id: IssueId, projects: &[String]) -> Result<Issue> {
        self.get_issue(id)?;
        let tx = self.conn.unchecked_transaction()?;
        self.join_projects(id, projects)?;
        tx.commit()?;
        self.get_issue(id)
    }

    /// Removes an issue from projects. It keeps its labels. Returns the issue.
    pub fn remove_from_projects(&self, id: IssueId, projects: &[String]) -> Result<Issue> {
        self.get_issue(id)?;
        let tx = self.conn.unchecked_transaction()?;
        for project in projects {
            let project_id = self.project_id(project)?;
            self.conn.execute(
                "DELETE FROM issue_projects WHERE issue_id = ?1 AND project_id = ?2",
                params![id, project_id],
            )?;
        }
        tx.commit()?;
        self.get_issue(id)
    }

    // ---- helpers ------------------------------------------------------------

    /// Adds the issue to the projects and applies their default labels. Call
    /// inside a transaction.
    pub(crate) fn join_projects(&self, id: IssueId, projects: &[String]) -> Result<()> {
        for project in projects {
            let project_id = self.project_id(project)?;
            self.conn.execute(
                "INSERT OR IGNORE INTO issue_projects (issue_id, project_id) VALUES (?1, ?2)",
                params![id, project_id],
            )?;
            self.conn.execute(
                "INSERT OR IGNORE INTO issue_labels (issue_id, label_id)
                 SELECT ?1, label_id FROM project_labels WHERE project_id = ?2",
                params![id, project_id],
            )?;
        }
        Ok(())
    }

    fn project_labels(&self, id: i64) -> Result<Vec<String>> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT l.name FROM project_labels pl JOIN labels l ON l.id = pl.label_id
                 WHERE pl.project_id = ?1 ORDER BY l.name COLLATE NOCASE",
            )?
            .query_map([id], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }

    fn find_project(&self, name: &str) -> Result<Option<(i64, String)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, name FROM projects WHERE name = ?1 COLLATE NOCASE",
                [name.trim()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }

    fn project_id(&self, name: &str) -> Result<i64> {
        Ok(self
            .find_project(name)?
            .with_context(|| format!("no project {:?}", name.trim()))?
            .0)
    }

    /// The stored spelling of an existing project's name.
    pub(crate) fn project_name(&self, name: &str) -> Result<String> {
        Ok(self
            .find_project(name)?
            .with_context(|| format!("no project {:?}", name.trim()))?
            .1)
    }

    fn project_name_by_id(&self, id: i64) -> Result<String> {
        Ok(self
            .conn
            .query_row("SELECT name FROM projects WHERE id = ?1", [id], |row| {
                row.get(0)
            })?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IssueFilter, NewIssue, Status};

    fn strings(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| s.to_string()).collect()
    }

    fn project(db: &Db, name: &str, labels: &[&str]) -> Project {
        db.create_project(&NewProject {
            labels: strings(labels),
            ..NewProject::new(name)
        })
        .unwrap()
    }

    #[test]
    fn create_update_delete() {
        let db = Db::open_in_memory().unwrap();
        let p = db
            .create_project(&NewProject {
                description: " goal ".into(),
                labels: strings(&["acme/reverb"]),
                ..NewProject::new(" Reverb v1.1 ")
            })
            .unwrap();
        assert_eq!(p.name, "Reverb v1.1");
        assert_eq!(p.status, ProjectStatus::Active);
        assert_eq!(p.description, "goal");
        assert_eq!(p.labels, ["acme/reverb"]);
        assert!(db.create_project(&NewProject::new("reverb V1.1")).is_err());

        let p = db
            .update_project(
                "reverb v1.1",
                &ProjectUpdate {
                    name: Some("Reverb 1.1".into()),
                    status: Some(ProjectStatus::Done),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(
            (p.name.as_str(), p.status),
            ("Reverb 1.1", ProjectStatus::Done)
        );
        assert!(db.list_projects(false).unwrap().is_empty());
        assert_eq!(db.list_projects(true).unwrap().len(), 1);

        let p = db
            .remove_project_labels("Reverb 1.1", &strings(&["acme/reverb"]))
            .unwrap();
        assert!(p.labels.is_empty());
        db.delete_project("reverb 1.1").unwrap();
        assert!(db.get_project("reverb 1.1").is_err());
    }

    #[test]
    fn listing_order() {
        let db = Db::open_in_memory().unwrap();
        let mk = |name: &str, status| {
            db.create_project(&NewProject {
                status,
                ..NewProject::new(name)
            })
            .unwrap();
        };
        mk("b", ProjectStatus::Planned);
        mk("d", ProjectStatus::Archived);
        mk("c", ProjectStatus::Active);
        mk("a", ProjectStatus::Active);
        let names = |all| -> Vec<String> {
            db.list_projects(all)
                .unwrap()
                .into_iter()
                .map(|p| p.name)
                .collect()
        };
        // Creation order, open projects first.
        assert_eq!(names(true), ["b", "c", "a", "d"]);
        assert_eq!(names(false), ["b", "c", "a"]);

        let before = |p: &str| Placement::Before(p.into());
        let after = |p: &str| Placement::After(p.into());
        assert_eq!(db.move_project("A", &before("b")).unwrap().name, "a");
        assert_eq!(names(true), ["a", "b", "c", "d"]);
        db.move_project("a", &after("c")).unwrap();
        assert_eq!(names(true), ["b", "c", "a", "d"]);
        db.move_project("b", &after("a")).unwrap();
        assert_eq!(names(true), ["c", "a", "b", "d"]);
        db.move_project("b", &after("b")).unwrap();
        assert_eq!(names(true), ["c", "a", "b", "d"]);
        assert!(db.move_project("b", &after("nope")).is_err());
        assert!(db.move_project("nope", &after("b")).is_err());

        // Closing a project keeps its place among the closed ones.
        db.move_project("d", &before("c")).unwrap();
        assert_eq!(names(true), ["c", "a", "b", "d"]);
        db.update_project(
            "a",
            &ProjectUpdate {
                status: Some(ProjectStatus::Done),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(names(true), ["c", "b", "d", "a"]);
        mk("e", ProjectStatus::Active);
        assert_eq!(names(true), ["c", "b", "e", "d", "a"]);
    }

    #[test]
    fn membership_and_default_labels() {
        let db = Db::open_in_memory().unwrap();
        project(&db, "v1", &["acme/reverb"]);
        project(&db, "shared", &["bug"]);
        let a = db
            .create_issue(&NewIssue {
                projects: strings(&["V1"]),
                ..NewIssue::new("a")
            })
            .unwrap();
        assert_eq!(a.projects, ["v1"]);
        assert_eq!(a.labels, ["acme/reverb"]);

        let a = db.add_to_projects(a.id, &strings(&["shared"])).unwrap();
        assert_eq!(a.projects, ["shared", "v1"]);
        assert_eq!(a.labels, ["acme/reverb", "bug"]);
        let a = db.remove_from_projects(a.id, &strings(&["v1"])).unwrap();
        assert_eq!(a.projects, ["shared"]);
        assert_eq!(a.labels, ["acme/reverb", "bug"], "labels are kept");
        assert!(db.add_to_projects(a.id, &strings(&["nope"])).is_err());

        db.set_status(a.id, Status::Done).unwrap();
        let shared = db.get_project("shared").unwrap();
        assert_eq!((shared.issue_count, shared.closed_issue_count), (1, 1));

        db.delete_project("shared").unwrap();
        assert!(db.get_issue(a.id).unwrap().projects.is_empty());
    }

    #[test]
    fn filters() {
        let db = Db::open_in_memory().unwrap();
        project(&db, "v1", &[]);
        project(&db, "v2", &[]);
        let new = |title: &str, projects: &[&str]| {
            db.create_issue(&NewIssue {
                projects: strings(projects),
                ..NewIssue::new(title)
            })
            .unwrap();
        };
        new("a", &["v1"]);
        new("b", &["v1", "v2"]);
        new("c", &[]);
        let titles = |filter: IssueFilter| -> Vec<String> {
            db.list_issues(&filter)
                .unwrap()
                .into_iter()
                .map(|i| i.title)
                .collect()
        };
        let any = |p: &[&str]| IssueFilter {
            projects: strings(p),
            ..Default::default()
        };
        assert_eq!(titles(any(&["v2"])), ["b"]);
        assert_eq!(titles(any(&["v1", "v2"])), ["b", "a"]);
        let none = IssueFilter {
            no_project: true,
            ..Default::default()
        };
        assert_eq!(titles(none), ["c"]);
        assert!(db.list_issues(&any(&["nope"])).is_err());
        assert!(
            db.list_issues(&IssueFilter {
                no_project: true,
                ..any(&["v1"])
            })
            .is_err()
        );
    }
}
