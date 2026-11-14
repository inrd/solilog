//! Views: named, saved issue filters ("acme open bugs").
//!
//! Views refer to labels and projects by name. Renaming a label or project
//! updates the views that use it; deleting one makes those views fail to
//! list until they are saved again.

use anyhow::{Context as _, Result};
use rusqlite::{OptionalExtension as _, params};
use serde::Serialize;

use crate::Db;
use crate::filter::IssueFilter;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct View {
    pub name: String,
    pub filter: IssueFilter,
}

fn normalize_view_name(name: &str) -> Result<String> {
    let name = name.trim();
    anyhow::ensure!(!name.is_empty(), "view name cannot be empty");
    anyhow::ensure!(
        !name.chars().any(char::is_control),
        "invalid view name {name:?}: control character"
    );
    Ok(name.to_owned())
}

fn view_from_row(row: &rusqlite::Row) -> rusqlite::Result<View> {
    let json: String = row.get(1)?;
    let filter = serde_json::from_str(&json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e))
    })?;
    Ok(View {
        name: row.get(0)?,
        filter,
    })
}

impl Db {
    /// Every view, by name.
    pub fn list_views(&self) -> Result<Vec<View>> {
        Ok(self
            .conn
            .prepare_cached("SELECT name, filter FROM views ORDER BY name COLLATE NOCASE")?
            .query_map([], view_from_row)?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn get_view(&self, name: &str) -> Result<View> {
        self.conn
            .query_row(
                "SELECT name, filter FROM views WHERE name = ?1 COLLATE NOCASE",
                [name.trim()],
                view_from_row,
            )
            .optional()?
            .with_context(|| format!("no view {:?}", name.trim()))
    }

    /// Creates a view, or replaces the filter of an existing one (whose name
    /// keeps its spelling). The filter must be valid. Returns the view.
    pub fn save_view(&self, name: &str, filter: &IssueFilter) -> Result<View> {
        let name = normalize_view_name(name)?;
        let filter = self.resolve_filter(filter)?;
        let json = serde_json::to_string(&filter)?;
        self.conn.execute(
            "INSERT INTO views (name, filter) VALUES (?1, ?2)
             ON CONFLICT (name) DO UPDATE SET filter = excluded.filter",
            params![name, json],
        )?;
        self.get_view(&name)
    }

    pub fn rename_view(&self, name: &str, new_name: &str) -> Result<View> {
        let view = self.get_view(name)?;
        let new_name = normalize_view_name(new_name)?;
        anyhow::ensure!(
            view.name.eq_ignore_ascii_case(&new_name) || self.get_view(&new_name).is_err(),
            "view {new_name:?} already exists"
        );
        self.conn.execute(
            "UPDATE views SET name = ?2 WHERE name = ?1",
            params![view.name, new_name],
        )?;
        self.get_view(&new_name)
    }

    /// Returns the deleted view.
    pub fn delete_view(&self, name: &str) -> Result<View> {
        let view = self.get_view(name)?;
        self.conn
            .execute("DELETE FROM views WHERE name = ?1", [&view.name])?;
        Ok(view)
    }

    /// Applies `rewrite` to every view's filter. Call inside the transaction
    /// that renames a label or project.
    pub(crate) fn rewrite_view_filters(&self, rewrite: impl Fn(&mut IssueFilter)) -> Result<()> {
        for mut view in self.list_views()? {
            rewrite(&mut view.filter);
            self.conn.execute(
                "UPDATE views SET filter = ?2 WHERE name = ?1",
                params![view.name, serde_json::to_string(&view.filter)?],
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NewIssue, NewProject, ProjectUpdate, Status};

    fn strings(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn save_list_rename_delete() {
        let db = Db::open_in_memory().unwrap();
        db.create_label("acme/reverb").unwrap();
        let filter = IssueFilter {
            labels: strings(&["ACME"]),
            open_only: true,
            ..Default::default()
        };
        let view = db.save_view(" Acme open ", &filter).unwrap();
        assert_eq!(view.name, "Acme open");
        assert_eq!(view.filter.labels, ["acme"], "stored with canonical names");

        let replaced = db
            .save_view(
                "acme OPEN",
                &IssueFilter {
                    statuses: vec![Status::Blocked],
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(replaced.name, "Acme open");
        assert_eq!(replaced.filter.statuses, [Status::Blocked]);
        assert_eq!(db.list_views().unwrap().len(), 1);

        let invalid = IssueFilter {
            labels: strings(&["nope"]),
            ..Default::default()
        };
        assert!(db.save_view("bad", &invalid).is_err());

        db.save_view("other", &IssueFilter::default()).unwrap();
        assert!(db.rename_view("other", "acme open").is_err());
        assert_eq!(db.rename_view("other", "Mine").unwrap().name, "Mine");
        db.delete_view("mine").unwrap();
        assert!(db.get_view("mine").is_err());
    }

    #[test]
    fn renames_propagate() {
        let db = Db::open_in_memory().unwrap();
        db.create_project(&NewProject::new("v1")).unwrap();
        db.create_issue(&NewIssue::new("a").labels(["acme/reverb"]).projects(["v1"]))
            .unwrap();
        let filter = IssueFilter {
            labels: strings(&["acme/reverb"]),
            without_labels: strings(&["acme"]),
            projects: strings(&["v1"]),
            ..Default::default()
        };
        db.save_view("v", &filter).unwrap();
        db.rename_label("acme", "globex/acme").unwrap();
        db.update_project(
            "v1",
            &ProjectUpdate {
                name: Some("v1.1".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let view = db.get_view("v").unwrap();
        assert_eq!(view.filter.labels, ["globex/acme/reverb"]);
        assert_eq!(view.filter.without_labels, ["globex/acme"]);
        assert_eq!(view.filter.projects, ["v1.1"]);
        assert!(db.list_issues(&view.filter).is_ok());

        db.delete_label("globex").unwrap();
        assert!(db.list_issues(&db.get_view("v").unwrap().filter).is_err());
    }
}
