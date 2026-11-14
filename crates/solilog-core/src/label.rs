//! Labels: free-form, hierarchical tags.
//!
//! A label's name is a `/`-separated path (`acme/reverb`). Every ancestor of
//! a label exists as a label of its own (`acme`), and is created on demand.
//! Filtering by a label also matches its sublabels: an issue labelled
//! `acme/reverb` matches the filter `acme`. Issues only carry the labels they
//! were given, never the implied ancestors.
//!
//! Names are case-insensitive (ASCII) and keep the case they were created with.
//!
//! Labels are listed as a tree, siblings in a user-defined order (rearranged
//! with [`Db::move_label`]). New labels go last among their siblings.

use anyhow::{Context as _, Result};
use rusqlite::{OptionalExtension as _, params};
use serde::Serialize;

use crate::Db;
use crate::order::{Ordered, Placement};

pub const LABEL_SEPARATOR: char = '/';

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Label {
    /// Full path, e.g. `acme/reverb`.
    pub name: String,
    /// Open and closed issues carrying this label or one of its sublabels.
    pub issue_count: i64,
}

impl Label {
    /// Parent path (`acme` for `acme/reverb`), `None` for a top-level label.
    pub fn parent(&self) -> Option<&str> {
        self.name.rsplit_once(LABEL_SEPARATOR).map(|(p, _)| p)
    }

    /// Nesting level: 0 for top-level labels.
    pub fn depth(&self) -> usize {
        self.name.matches(LABEL_SEPARATOR).count()
    }
}

/// Trims each path segment and rejects empty ones: `" acme / reverb "` →
/// `"acme/reverb"`.
pub fn normalize_label_name(name: &str) -> Result<String> {
    let segments: Vec<&str> = name.split(LABEL_SEPARATOR).map(str::trim).collect();
    anyhow::ensure!(
        segments.iter().all(|s| !s.is_empty()),
        "invalid label name {name:?}: empty segment"
    );
    anyhow::ensure!(
        !name.chars().any(char::is_control),
        "invalid label name {name:?}: control character"
    );
    Ok(segments.join("/"))
}

/// Whether label `name` is `filter` or one of its sublabels (case-insensitive).
pub fn label_matches(name: &str, filter: &str) -> bool {
    let (name, filter) = (name.as_bytes(), filter.as_bytes());
    name.len() >= filter.len()
        && name[..filter.len()].eq_ignore_ascii_case(filter)
        && (name.len() == filter.len() || name[filter.len()] == LABEL_SEPARATOR as u8)
}

/// Label `name` after renaming `old` to `new`, if it is `old` or one of its
/// sublabels: `renamed_label("acme/reverb", "acme", "globex/acme")` is
/// `Some("globex/acme/reverb")`.
pub fn renamed_label(name: &str, old: &str, new: &str) -> Option<String> {
    // `label_matches` compares bytes, so `old.len()` is a char boundary.
    label_matches(name, old).then(|| format!("{new}{}", &name[old.len()..]))
}

impl Db {
    /// Every label, as a tree: each label is followed by its sublabels, and
    /// siblings are in the user-defined order.
    pub fn list_labels(&self) -> Result<Vec<Label>> {
        let rows: Vec<(String, (i64, i64))> = self
            .conn
            .prepare_cached("SELECT name, position, id FROM labels")?
            .query_map([], |row| Ok((row.get(0)?, (row.get(1)?, row.get(2)?))))?
            .collect::<rusqlite::Result<_>>()?;
        // Sort by the (position, id) of every ancestor, then of the label:
        // parents come before their children, siblings by position.
        let rank: std::collections::HashMap<String, (i64, i64)> = rows
            .iter()
            .map(|(name, rank)| (name.to_ascii_lowercase(), *rank))
            .collect();
        let path_rank = |name: &str| -> Vec<(i64, i64)> {
            let lower = name.to_ascii_lowercase();
            lower
                .match_indices(LABEL_SEPARATOR)
                .map(|(ix, _)| &lower[..ix])
                .chain([lower.as_str()])
                .map(|path| rank.get(path).copied().unwrap_or_default())
                .collect()
        };
        let mut names: Vec<String> = rows.iter().map(|(name, _)| name.clone()).collect();
        names.sort_by_cached_key(|name| path_rank(name));
        let assigned: Vec<(i64, String)> = self
            .conn
            .prepare_cached(
                "SELECT il.issue_id, l.name FROM issue_labels il JOIN labels l ON l.id = il.label_id",
            )?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(names
            .into_iter()
            .map(|name| {
                let mut issues: Vec<i64> = assigned
                    .iter()
                    .filter(|(_, l)| label_matches(l, &name))
                    .map(|(id, _)| *id)
                    .collect();
                issues.sort_unstable();
                issues.dedup();
                Label {
                    issue_count: issues.len() as i64,
                    name,
                }
            })
            .collect())
    }

    pub fn get_label(&self, name: &str) -> Result<Label> {
        let name = self.label_name(name)?;
        Ok(self
            .list_labels()?
            .into_iter()
            .find(|l| l.name == name)
            .expect("label exists"))
    }

    /// Creates a label (and any missing ancestors). Fails if it already exists.
    pub fn create_label(&self, name: &str) -> Result<Label> {
        let name = normalize_label_name(name)?;
        anyhow::ensure!(
            self.find_label(&name)?.is_none(),
            "label {name:?} already exists"
        );
        let tx = self.conn.unchecked_transaction()?;
        self.ensure_label(&name)?;
        tx.commit()?;
        self.get_label(&name)
    }

    /// Renames a label, moving its sublabels along (`acme` → `globex/acme`
    /// also turns `acme/reverb` into `globex/acme/reverb`). Issues keep their
    /// labels. Missing ancestors of the new name are created.
    pub fn rename_label(&self, name: &str, new_name: &str) -> Result<Label> {
        let old = self.label_name(name)?;
        let new = normalize_label_name(new_name)?;
        let case_only = old.eq_ignore_ascii_case(&new);
        anyhow::ensure!(
            case_only || self.find_label(&new)?.is_none(),
            "label {new:?} already exists"
        );
        anyhow::ensure!(
            case_only || !label_matches(&new, &old),
            "cannot move label {old:?} into itself"
        );
        let tx = self.conn.unchecked_transaction()?;
        // Reuse the stored spelling of existing ancestors.
        let new = match new.rsplit_once(LABEL_SEPARATOR) {
            Some((parent, leaf)) => {
                let parent_id = self.ensure_label(parent)?;
                format!("{}/{leaf}", self.label_name_by_id(parent_id)?)
            }
            _ => new,
        };
        // A label that changes parent goes last among its new siblings.
        let parent = |name: &str| {
            name.rsplit_once(LABEL_SEPARATOR)
                .map(|(p, _)| p.to_ascii_lowercase())
        };
        if parent(&old) != parent(&new) {
            self.conn.execute(
                "UPDATE labels SET position = ?2 WHERE name = ?1",
                params![old, self.next_position(Ordered::Labels)?],
            )?;
        }
        let subtree: Vec<(i64, String)> = self
            .conn
            .prepare("SELECT id, name FROM labels")?
            .query_map([], |row| Ok((row.get(0)?, row.get::<_, String>(1)?)))?
            .filter(|r| r.as_ref().map_or(true, |(_, n)| label_matches(n, &old)))
            .collect::<rusqlite::Result<_>>()?;
        for (id, name) in subtree {
            let renamed = renamed_label(&name, &old, &new).unwrap_or(name);
            self.conn.execute(
                "UPDATE labels SET name = ?2 WHERE id = ?1",
                params![id, renamed],
            )?;
        }
        self.rewrite_view_filters(|filter| filter.rename_label(&old, &new))?;
        tx.commit()?;
        self.get_label(&new)
    }

    /// Moves a label, with its sublabels, before or after a sibling (a label
    /// with the same parent). Returns the moved label.
    pub fn move_label(&self, name: &str, placement: &Placement) -> Result<Label> {
        let name = self.label_name(name)?;
        let anchor = self.label_name(placement.anchor())?;
        let parent = |name: &str| {
            name.rsplit_once(LABEL_SEPARATOR)
                .map(|(p, _)| p.to_ascii_lowercase())
        };
        anyhow::ensure!(
            parent(&name) == parent(&anchor),
            "cannot move label {name:?} next to {anchor:?}: only labels with the same parent can be reordered"
        );
        let tx = self.conn.unchecked_transaction()?;
        let siblings: Vec<(i64, String)> = self
            .conn
            .prepare_cached("SELECT id, name FROM labels ORDER BY position, id")?
            .query_map([], |row| Ok((row.get(0)?, row.get::<_, String>(1)?)))?
            .filter(|r| r.as_ref().map_or(true, |(_, n)| parent(n) == parent(&name)))
            .collect::<rusqlite::Result<_>>()?;
        let id_of = |label: &str| {
            siblings
                .iter()
                .find(|(_, n)| n == label)
                .map(|(id, _)| *id)
                .context("label vanished while moving")
        };
        let (id, anchor) = (id_of(&name)?, id_of(&anchor)?);
        let order = siblings.iter().map(|(id, _)| *id).collect();
        self.place(Ordered::Labels, order, id, anchor, placement)?;
        tx.commit()?;
        self.get_label(&name)
    }

    /// Deletes a label and its sublabels, removing them from every issue.
    /// Returns the deleted labels.
    pub fn delete_label(&self, name: &str) -> Result<Vec<Label>> {
        let name = self.label_name(name)?;
        let deleted: Vec<Label> = self
            .list_labels()?
            .into_iter()
            .filter(|l| label_matches(&l.name, &name))
            .collect();
        let tx = self.conn.unchecked_transaction()?;
        for label in &deleted {
            self.conn
                .execute("DELETE FROM labels WHERE name = ?1", [&label.name])?;
        }
        tx.commit()?;
        Ok(deleted)
    }

    /// Adds labels to an issue, creating labels that don't exist yet.
    /// Labels the issue already has are left alone. Returns the issue.
    pub fn add_labels(&self, id: crate::IssueId, labels: &[String]) -> Result<crate::Issue> {
        self.get_issue(id)?;
        let tx = self.conn.unchecked_transaction()?;
        for label in labels {
            let label_id = self.ensure_label(&normalize_label_name(label)?)?;
            self.conn.execute(
                "INSERT OR IGNORE INTO issue_labels (issue_id, label_id) VALUES (?1, ?2)",
                params![id, label_id],
            )?;
        }
        tx.commit()?;
        self.get_issue(id)
    }

    /// Removes labels from an issue (exact labels, not sublabels). Labels the
    /// issue doesn't have are ignored; unknown labels are an error.
    pub fn remove_labels(&self, id: crate::IssueId, labels: &[String]) -> Result<crate::Issue> {
        self.get_issue(id)?;
        let tx = self.conn.unchecked_transaction()?;
        for label in labels {
            let label_id = self.label_id(label)?;
            self.conn.execute(
                "DELETE FROM issue_labels WHERE issue_id = ?1 AND label_id = ?2",
                params![id, label_id],
            )?;
        }
        tx.commit()?;
        self.get_issue(id)
    }

    // ---- helpers ------------------------------------------------------------

    fn find_label(&self, name: &str) -> Result<Option<(i64, String)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, name FROM labels WHERE name = ?1 COLLATE NOCASE",
                [name],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }

    pub(crate) fn label_id(&self, name: &str) -> Result<i64> {
        let name = normalize_label_name(name)?;
        Ok(self
            .find_label(&name)?
            .with_context(|| format!("no label {name:?}"))?
            .0)
    }

    /// The stored spelling of an existing label.
    pub(crate) fn label_name(&self, name: &str) -> Result<String> {
        let name = normalize_label_name(name)?;
        Ok(self
            .find_label(&name)?
            .with_context(|| format!("no label {name:?}"))?
            .1)
    }

    fn label_name_by_id(&self, id: i64) -> Result<String> {
        Ok(self
            .conn
            .query_row("SELECT name FROM labels WHERE id = ?1", [id], |row| {
                row.get(0)
            })?)
    }

    /// Returns the id of a normalized label path, creating it and its missing
    /// ancestors. Existing ancestors keep their spelling. Call inside a
    /// transaction.
    pub(crate) fn ensure_label(&self, name: &str) -> Result<i64> {
        let mut path = String::new();
        let mut id = 0;
        for segment in name.split(LABEL_SEPARATOR) {
            if !path.is_empty() {
                path.push(LABEL_SEPARATOR);
            }
            path.push_str(segment);
            (id, path) = match self.find_label(&path)? {
                Some(found) => found,
                None => {
                    self.conn.execute(
                        "INSERT INTO labels (name, position) VALUES (?1, ?2)",
                        params![path, self.next_position(Ordered::Labels)?],
                    )?;
                    (self.conn.last_insert_rowid(), path)
                }
            };
        }
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IssueFilter, NewIssue};

    fn names(db: &Db) -> Vec<String> {
        db.list_labels()
            .unwrap()
            .into_iter()
            .map(|l| l.name)
            .collect()
    }

    fn strings(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn normalization_and_matching() {
        assert_eq!(normalize_label_name(" a / b ").unwrap(), "a/b");
        assert!(normalize_label_name("a//b").is_err());
        assert!(normalize_label_name("/a").is_err());
        assert!(normalize_label_name("").is_err());
        assert!(label_matches("acme/reverb", "acme"));
        assert!(label_matches("Acme/Reverb", "acme/reverb"));
        assert!(label_matches("acme", "acme"));
        assert!(!label_matches("acmeish", "acme"));
        assert!(!label_matches("acme", "acme/reverb"));
    }

    #[test]
    fn create_makes_ancestors_and_keeps_spelling() {
        let db = Db::open_in_memory().unwrap();
        db.create_label("Acme").unwrap();
        let label = db.create_label("acme/reverb").unwrap();
        assert_eq!(label.name, "Acme/reverb");
        assert_eq!(label.parent(), Some("Acme"));
        assert_eq!(label.depth(), 1);
        assert!(db.create_label("ACME").is_err());
        db.create_label("x/y/z").unwrap();
        assert_eq!(names(&db), ["Acme", "Acme/reverb", "x", "x/y", "x/y/z"]);
    }

    #[test]
    fn order() {
        let db = Db::open_in_memory().unwrap();
        for name in ["b", "a/z", "a/y", "c"] {
            db.create_label(name).unwrap();
        }
        // Creation order, children after their parent.
        assert_eq!(names(&db), ["b", "a", "a/z", "a/y", "c"]);
        let before = |l: &str| Placement::Before(l.into());
        let after = |l: &str| Placement::After(l.into());

        // A label moves with its sublabels.
        assert_eq!(db.move_label("A", &before("b")).unwrap().name, "a");
        assert_eq!(names(&db), ["a", "a/z", "a/y", "b", "c"]);
        db.move_label("a", &after("c")).unwrap();
        assert_eq!(names(&db), ["b", "c", "a", "a/z", "a/y"]);
        db.move_label("a/z", &after("a/y")).unwrap();
        assert_eq!(names(&db), ["b", "c", "a", "a/y", "a/z"]);
        db.move_label("c", &before("c")).unwrap();
        assert_eq!(names(&db), ["b", "c", "a", "a/y", "a/z"]);
        assert!(db.move_label("a/y", &before("b")).is_err(), "not siblings");
        assert!(db.move_label("a", &before("nope")).is_err());

        // New sublabels go last among their siblings.
        db.create_label("a/x").unwrap();
        assert_eq!(names(&db), ["b", "c", "a", "a/y", "a/z", "a/x"]);
        // So do labels moved under a new parent; keeping the parent keeps the place.
        db.rename_label("a/y", "b/y").unwrap();
        db.rename_label("a/z", "a/Z").unwrap();
        assert_eq!(names(&db), ["b", "b/y", "c", "a", "a/Z", "a/x"]);
        db.rename_label("c", "top").unwrap();
        assert_eq!(names(&db), ["b", "b/y", "top", "a", "a/Z", "a/x"]);
    }

    #[test]
    fn issue_labels_and_counts() {
        let db = Db::open_in_memory().unwrap();
        let a = db.create_issue(&NewIssue::new("a")).unwrap();
        let b = db.create_issue(&NewIssue::new("b")).unwrap();
        let a = db
            .add_labels(a.id, &strings(&["bug", "acme/reverb", "bug"]))
            .unwrap();
        assert_eq!(a.labels, ["acme/reverb", "bug"]);
        db.add_labels(b.id, &strings(&["acme"])).unwrap();
        let counts: Vec<_> = db
            .list_labels()
            .unwrap()
            .into_iter()
            .map(|l| (l.name, l.issue_count))
            .collect();
        assert_eq!(
            counts,
            [
                ("bug".to_string(), 1),
                ("acme".to_string(), 2),
                ("acme/reverb".to_string(), 1)
            ]
        );
        let a = db.remove_labels(a.id, &strings(&["BUG"])).unwrap();
        assert_eq!(a.labels, ["acme/reverb"]);
        assert!(db.remove_labels(a.id, &strings(&["nope"])).is_err());
        assert!(db.add_labels(crate::IssueId(99), &strings(&["x"])).is_err());
        let c = db
            .create_issue(&NewIssue::new("c").labels(["new/label"]))
            .unwrap();
        assert_eq!(c.labels, ["new/label"]);
        assert!(names(&db).contains(&"new".to_string()));
        assert!(
            db.create_issue(&NewIssue::new("d").labels(["a//b"]))
                .is_err()
        );
    }

    #[test]
    fn filter_implies_ancestors() {
        let db = Db::open_in_memory().unwrap();
        let a = db.create_issue(&NewIssue::new("a")).unwrap();
        let b = db.create_issue(&NewIssue::new("b")).unwrap();
        db.create_issue(&NewIssue::new("c")).unwrap();
        db.add_labels(a.id, &strings(&["acme/reverb", "bug"]))
            .unwrap();
        db.add_labels(b.id, &strings(&["globex/delay", "bug"]))
            .unwrap();
        let titles = |filter: IssueFilter| -> Vec<String> {
            db.list_issues(&filter)
                .unwrap()
                .into_iter()
                .map(|i| i.title)
                .collect()
        };
        let acme = IssueFilter {
            labels: strings(&["ACME"]),
            ..Default::default()
        };
        assert_eq!(titles(acme), ["a"]);
        let bugs = IssueFilter {
            labels: strings(&["bug"]),
            ..Default::default()
        };
        assert_eq!(titles(bugs), ["b", "a"]);
        let not_acme = IssueFilter {
            without_labels: strings(&["acme"]),
            ..Default::default()
        };
        assert_eq!(titles(not_acme), ["c", "b"]);
        let unknown = IssueFilter {
            labels: strings(&["nope"]),
            ..Default::default()
        };
        assert!(db.list_issues(&unknown).is_err());
    }

    #[test]
    fn rename_moves_subtree() {
        let db = Db::open_in_memory().unwrap();
        let a = db.create_issue(&NewIssue::new("a")).unwrap();
        db.add_labels(a.id, &strings(&["acme/reverb", "bug"]))
            .unwrap();
        db.create_label("Globex").unwrap();
        let renamed = db.rename_label("acme", "globex/acme").unwrap();
        assert_eq!(renamed.name, "Globex/acme");
        assert_eq!(
            names(&db),
            ["bug", "Globex", "Globex/acme", "Globex/acme/reverb"]
        );
        assert_eq!(
            db.get_issue(a.id).unwrap().labels,
            ["bug", "Globex/acme/reverb"]
        );
        assert!(db.rename_label("bug", "globex").is_err());
        assert!(db.rename_label("globex", "globex/x").is_err());
        assert!(db.rename_label("nope", "x").is_err());
        assert_eq!(db.rename_label("bug", "Bug").unwrap().name, "Bug");
    }

    #[test]
    fn delete_removes_subtree() {
        let db = Db::open_in_memory().unwrap();
        let a = db.create_issue(&NewIssue::new("a")).unwrap();
        db.add_labels(a.id, &strings(&["acme/reverb", "bug"]))
            .unwrap();
        let deleted = db.delete_label("acme").unwrap();
        assert_eq!(deleted.len(), 2);
        assert_eq!(names(&db), ["bug"]);
        assert_eq!(db.get_issue(a.id).unwrap().labels, ["bug"]);
        assert!(db.delete_label("acme").is_err());
    }
}
