//! Dependencies between issues: an issue depends on others, which must be
//! done before it can start. Informational only: nothing changes an issue's
//! status by itself.

use std::collections::HashMap;

use anyhow::Result;
use rusqlite::{OptionalExtension as _, params};

use crate::Db;
use crate::issue::{ISSUE_PREFIX, Issue, IssueChange, IssueId, Status};

/// One issue's dependencies, as loaded from the database.
#[derive(Default)]
struct Links {
    depends_on: Vec<IssueId>,
    dependents: Vec<IssueId>,
    waiting_on: Vec<IssueId>,
}

impl Db {
    /// Makes an issue depend on others (to be done first) and returns it.
    /// Existing dependencies are kept; unknown issues, an issue depending
    /// on itself and cycles are errors.
    pub fn add_dependencies(&self, id: IssueId, on: &[IssueId]) -> Result<Issue> {
        self.update_issue(id, &IssueChange::AddDependencies(on.to_vec()))
    }

    /// Removes dependencies of an issue and returns it. Dependencies it
    /// doesn't have are ignored; unknown issues are errors.
    pub fn remove_dependencies(&self, id: IssueId, on: &[IssueId]) -> Result<Issue> {
        self.update_issue(id, &IssueChange::RemoveDependencies(on.to_vec()))
    }

    /// Fills the dependency fields of `issues`. With `only`, loads just the
    /// links of that issue (the others are left empty).
    pub(crate) fn fill_dependencies(
        &self,
        issues: &mut [Issue],
        only: Option<IssueId>,
    ) -> Result<()> {
        let mut links: HashMap<IssueId, Links> = HashMap::new();
        let mut stmt = self.conn.prepare_cached(
            "SELECT d.issue_id, d.depends_on_id, i.status
             FROM issue_dependencies d JOIN issues i ON i.id = d.depends_on_id
             WHERE ?1 IS NULL OR d.issue_id = ?1 OR d.depends_on_id = ?1
             ORDER BY d.issue_id, d.depends_on_id",
        )?;
        let rows = stmt.query_map([only], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get::<_, Status>(2)?))
        })?;
        for row in rows {
            let (issue, on, status): (IssueId, IssueId, Status) = row?;
            let entry = links.entry(issue).or_default();
            entry.depends_on.push(on);
            if !status.is_closed() {
                entry.waiting_on.push(on);
            }
            links.entry(on).or_default().dependents.push(issue);
        }
        for issue in issues {
            if let Some(mut l) = links.remove(&issue.id) {
                l.dependents.sort();
                issue.depends_on = l.depends_on;
                issue.dependents = l.dependents;
                issue.waiting_on = l.waiting_on;
            }
        }
        Ok(())
    }

    /// Makes `issue` depend on each of `on`. Returns the dependencies
    /// actually added. Call inside a transaction. Existing dependencies are
    /// left as they are; unknown issues, an issue depending on itself and
    /// cycles are errors.
    pub(crate) fn insert_dependencies(
        &self,
        issue: IssueId,
        on: &[IssueId],
    ) -> Result<Vec<IssueId>> {
        self.get_issue(issue)?;
        let mut added = Vec::new();
        for &on in on {
            self.get_issue(on)?;
            anyhow::ensure!(on != issue, "{issue} can't depend on itself");
            if let Some(path) = self.dependency_path(on, issue)? {
                anyhow::bail!(
                    "{issue} can't depend on {on}: that would make a cycle ({issue} → {path})"
                );
            }
            let inserted = self.conn.execute(
                "INSERT OR IGNORE INTO issue_dependencies (issue_id, depends_on_id) VALUES (?1, ?2)",
                params![issue, on],
            )?;
            if inserted > 0 {
                added.push(on);
            }
        }
        Ok(added)
    }

    /// Removes `issue`'s dependencies on each of `on`; missing ones are
    /// ignored, unknown issues are errors. Returns the dependencies actually
    /// removed. Call inside a transaction.
    pub(crate) fn delete_dependencies(
        &self,
        issue: IssueId,
        on: &[IssueId],
    ) -> Result<Vec<IssueId>> {
        self.get_issue(issue)?;
        let mut removed = Vec::new();
        for &on in on {
            self.get_issue(on)?;
            let deleted = self.conn.execute(
                "DELETE FROM issue_dependencies WHERE issue_id = ?1 AND depends_on_id = ?2",
                params![issue, on],
            )?;
            if deleted > 0 {
                removed.push(on);
            }
        }
        Ok(removed)
    }

    /// The issues that can't become a dependency of every one of `ids`:
    /// `ids` themselves and the issues that depend on one of them, directly
    /// or through other issues (that would make a cycle). Sorted.
    pub fn cannot_depend_on(&self, ids: &[IssueId]) -> Result<Vec<IssueId>> {
        let mut stmt = self.conn.prepare_cached(
            "WITH RECURSIVE reach (id) AS (
                 SELECT value FROM json_each(?1)
                 UNION
                 SELECT d.issue_id FROM issue_dependencies d JOIN reach r ON d.depends_on_id = r.id
             )
             SELECT id FROM reach ORDER BY id",
        )?;
        let ids = serde_json::to_string(&ids.iter().map(|id| id.0).collect::<Vec<_>>())?;
        let found = stmt
            .query_map([ids], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(found)
    }

    /// How `from` depends on `to`, directly or through other issues
    /// (`SOL-1 → SOL-2 → SOL-3`), or `None` if it doesn't.
    fn dependency_path(&self, from: IssueId, to: IssueId) -> Result<Option<String>> {
        // The dependencies never form a cycle, so the walk ends.
        Ok(self
            .conn
            .query_row(
                "WITH RECURSIVE reach (id, path) AS (
                     SELECT ?1, ?3 || ?1
                     UNION ALL
                     SELECT d.depends_on_id, r.path || ' → ' || ?3 || d.depends_on_id
                     FROM issue_dependencies d JOIN reach r ON d.issue_id = r.id
                 )
                 SELECT path FROM reach WHERE id = ?2 LIMIT 1",
                params![from, to, format!("{ISSUE_PREFIX}-")],
                |row| row.get(0),
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use crate::{Db, IssueChange, IssueFilter, IssueId, NewIssue, Status};

    fn ids(db: &Db, filter: &IssueFilter) -> Vec<IssueId> {
        let mut ids: Vec<_> = db
            .list_issues(filter)
            .unwrap()
            .iter()
            .map(|i| i.id)
            .collect();
        ids.sort();
        ids
    }

    #[test]
    fn add_and_remove() {
        let db = Db::open_in_memory().unwrap();
        let id = |title: &str| db.create_issue(&NewIssue::new(title)).unwrap().id;
        let (a, b, c) = (id("a"), id("b"), id("c"));

        let updated = db
            .update_issue(a, &IssueChange::AddDependencies(vec![c, b, b]))
            .unwrap();
        assert_eq!(updated.depends_on, [b, c]);
        assert_eq!(updated.waiting_on, [b, c]);
        assert!(updated.dependents.is_empty());
        assert_eq!(db.get_issue(b).unwrap().dependents, [a]);
        // Adding it again is a no-op.
        db.update_issue(a, &IssueChange::AddDependencies(vec![b]))
            .unwrap();

        // Closed dependencies don't make the issue wait.
        db.set_status(b, Status::Done).unwrap();
        assert_eq!(db.get_issue(a).unwrap().waiting_on, [c]);
        db.set_status(c, Status::Cancelled).unwrap();
        assert!(db.get_issue(a).unwrap().waiting_on.is_empty());

        // Listing fills the same fields.
        let listed = db.list_issues(&IssueFilter::default()).unwrap();
        let listed_b = listed.iter().find(|i| i.id == b).unwrap();
        assert_eq!(listed_b.dependents, [a]);
        assert_eq!(
            listed.iter().find(|i| i.id == a).unwrap().depends_on,
            [b, c]
        );

        let updated = db
            .update_issue(a, &IssueChange::RemoveDependencies(vec![b, a]))
            .unwrap();
        assert_eq!(updated.depends_on, [c]);
        assert!(db.get_issue(b).unwrap().dependents.is_empty());
        assert!(
            db.update_issue(a, &IssueChange::RemoveDependencies(vec![IssueId(999)]))
                .is_err()
        );
    }

    #[test]
    fn refused() {
        let db = Db::open_in_memory().unwrap();
        let id = |title: &str| db.create_issue(&NewIssue::new(title)).unwrap().id;
        let (a, b, c) = (id("a"), id("b"), id("c"));
        let add = |issue, on: &[IssueId]| {
            db.update_issue(issue, &IssueChange::AddDependencies(on.to_vec()))
        };
        assert!(add(a, &[a]).is_err());
        assert!(add(a, &[IssueId(999)]).is_err());
        add(a, &[b]).unwrap();
        add(b, &[c]).unwrap();
        // Direct and indirect cycles.
        assert!(add(b, &[a]).is_err());
        let err = add(c, &[a]).unwrap_err().to_string();
        assert!(err.contains("SOL-3 → SOL-1 → SOL-2 → SOL-3"), "{err}");

        // All or nothing, across issues and dependencies.
        let d = id("d");
        assert!(
            db.update_issues(&[d, c], &IssueChange::AddDependencies(vec![a]))
                .is_err()
        );
        assert!(db.get_issue(d).unwrap().depends_on.is_empty());
        assert!(add(d, &[b, d]).is_err());
        assert!(db.get_issue(d).unwrap().depends_on.is_empty());

        // What a picker should leave out (a → b → c): the issues and those
        // depending on them, directly or not.
        assert_eq!(db.cannot_depend_on(&[c]).unwrap(), [a, b, c]);
        assert_eq!(db.cannot_depend_on(&[b, d]).unwrap(), [a, b, d]);
        assert_eq!(db.cannot_depend_on(&[a]).unwrap(), [a]);
        assert!(db.cannot_depend_on(&[]).unwrap().is_empty());
    }

    #[test]
    fn delete_and_archive() {
        let db = Db::open_in_memory().unwrap();
        let id = |title: &str| db.create_issue(&NewIssue::new(title)).unwrap().id;
        let (a, b, c) = (id("a"), id("b"), id("c"));
        db.update_issue(b, &IssueChange::AddDependencies(vec![a]))
            .unwrap();
        db.update_issue(c, &IssueChange::AddDependencies(vec![b]))
            .unwrap();

        // An archived open dependency still counts.
        db.set_archived(a, true).unwrap();
        assert_eq!(db.get_issue(b).unwrap().waiting_on, [a]);

        // Deleting an issue drops its links both ways.
        db.delete_issue(b).unwrap();
        assert!(db.get_issue(a).unwrap().dependents.is_empty());
        assert!(db.get_issue(c).unwrap().depends_on.is_empty());
    }

    #[test]
    fn ready_and_waiting() {
        let db = Db::open_in_memory().unwrap();
        let id = |title: &str| db.create_issue(&NewIssue::new(title)).unwrap().id;
        let (a, b, c) = (id("a"), id("b"), id("c"));
        let closed = db
            .create_issue(&NewIssue::new("closed").status(Status::Done))
            .unwrap()
            .id;
        db.update_issues(&[b, closed], &IssueChange::AddDependencies(vec![a]))
            .unwrap();
        let ready = IssueFilter {
            ready: true,
            ..Default::default()
        };
        let waiting = IssueFilter {
            waiting: true,
            ..Default::default()
        };
        assert_eq!(ids(&db, &ready), [a, c]);
        assert_eq!(ids(&db, &waiting), [b]);
        db.set_status(a, Status::Done).unwrap();
        assert_eq!(ids(&db, &ready), [b, c]);
        assert!(ids(&db, &waiting).is_empty());

        let both = IssueFilter {
            ready: true,
            waiting: true,
            ..Default::default()
        };
        assert!(db.list_issues(&both).is_err());
    }
}
