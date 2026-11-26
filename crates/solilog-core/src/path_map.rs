//! Directory paths mapped to labels and projects: where a codebase lives on
//! disk, so an agent working in a directory can find the label or project
//! its issues belong to (and the other way round).
//!
//! A label or project can have several paths (a checkout and its worktrees);
//! a path belongs to one label or one project only. A directory belongs to
//! the mapping of the deepest mapped path containing it, so a sublabel's
//! codebase can sit inside its parent's, and directories below a mapped path
//! belong to it.
//!
//! Paths under the home directory are stored relative to it (`~/code/reverb`)
//! so a database restored on another machine with the same layout keeps
//! working. Paths that don't exist (not cloned on this machine, a drive not
//! plugged in) are kept: they just match nothing.

use std::path::{Component, Path, PathBuf};

use anyhow::Result;
use rusqlite::{OptionalExtension as _, params};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{Db, IssueFilter, NewIssue};

/// A directory mapped to a label or project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MappedPath {
    /// As stored: `~/…` under the home directory, absolute otherwise.
    pub path: String,
    /// The path on this machine, absolute.
    pub absolute: PathBuf,
    /// Whether the directory exists on this machine.
    pub exists: bool,
}

/// A stored path, with whether it exists on this machine.
pub(crate) fn mapped_path(path: String) -> MappedPath {
    let absolute = expand_home(&path, dirs::home_dir().as_deref());
    MappedPath {
        exists: absolute.is_dir(),
        absolute,
        path,
    }
}

/// What a path is mapped to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PathOwnerKind {
    Label,
    Project,
}

impl PathOwnerKind {
    pub fn key(self) -> &'static str {
        match self {
            PathOwnerKind::Label => "label",
            PathOwnerKind::Project => "project",
        }
    }
}

/// A mapped path with its label or project: the result of looking a
/// directory up ([`Db::path_owner`]), or a row of [`Db::list_paths`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PathMapping {
    /// `label` or `project`.
    pub kind: PathOwnerKind,
    /// The label's full name or the project's name.
    pub name: String,
    /// The mapped path, as stored (`~/…` under the home directory).
    pub path: String,
    /// The mapped path on this machine, absolute.
    pub absolute: PathBuf,
    /// Whether the directory exists on this machine.
    pub exists: bool,
}

impl PathMapping {
    /// Narrows a filter to this label's or project's issues.
    pub fn scope_filter(&self, filter: &mut IssueFilter) {
        let list = match self.kind {
            PathOwnerKind::Label => &mut filter.labels,
            PathOwnerKind::Project => &mut filter.projects,
        };
        if !list.iter().any(|n| n.eq_ignore_ascii_case(&self.name)) {
            list.push(self.name.clone());
        }
    }

    /// Gives a new issue this label, or puts it in this project.
    pub fn scope_new_issue(&self, new: &mut NewIssue) {
        let list = match self.kind {
            PathOwnerKind::Label => &mut new.labels,
            PathOwnerKind::Project => &mut new.projects,
        };
        if !list.iter().any(|n| n.eq_ignore_ascii_case(&self.name)) {
            list.push(self.name.clone());
        }
    }
}

/// Removes `.` and `..` components and trailing separators, without touching
/// the file system.
fn clean(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// The real path when it exists (symlinks resolved, as the shell's working
/// directory has them), else the cleaned-up path.
fn resolve(path: &Path) -> PathBuf {
    let path = clean(path);
    std::fs::canonicalize(&path).unwrap_or(path)
}

/// `~` or `~/…` expanded with `home`; other paths as given.
fn expand_home(path: &str, home: Option<&Path>) -> PathBuf {
    match (path.strip_prefix('~'), home) {
        (Some(""), Some(home)) => home.to_path_buf(),
        (Some(rest), Some(home)) if rest.starts_with('/') || rest.starts_with('\\') => {
            home.join(&rest[1..])
        }
        _ => PathBuf::from(path),
    }
}

/// The form a directory path is stored in: absolute, cleaned up, symlinks
/// resolved when it exists, and `~/…` when under `home`. Relative paths are
/// refused: the CLI makes them absolute first.
pub fn normalize_mapped_path(path: &str, home: Option<&Path>) -> Result<String> {
    let trimmed = path.trim();
    anyhow::ensure!(!trimmed.is_empty(), "path cannot be empty");
    anyhow::ensure!(
        !trimmed.chars().any(char::is_control),
        "invalid path {trimmed:?}: control character"
    );
    let expanded = expand_home(trimmed, home);
    anyhow::ensure!(
        expanded.is_absolute(),
        "path {trimmed:?} must be absolute (or start with ~/)"
    );
    let resolved = resolve(&expanded);
    if let Some(home) = home {
        for home in [resolve(home), clean(home)] {
            if let Ok(rest) = resolved.strip_prefix(&home) {
                let rest = rest.to_string_lossy();
                return Ok(if rest.is_empty() {
                    "~".into()
                } else {
                    format!("~/{}", rest.replace('\\', "/"))
                });
            }
        }
    }
    resolved
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("path {trimmed:?} is not valid UTF-8"))
}

impl Db {
    /// Maps a directory to a label (see [`PathMapping`]). Fails if the path
    /// already belongs to another label or a project. Returns the label.
    pub fn add_label_path(&self, name: &str, path: &str) -> Result<crate::Label> {
        self.add_label_path_in(name, path, dirs::home_dir().as_deref())
    }

    /// Removes a directory from a label's paths. Returns the label.
    pub fn remove_label_path(&self, name: &str, path: &str) -> Result<crate::Label> {
        self.remove_label_path_in(name, path, dirs::home_dir().as_deref())
    }

    /// Maps a directory to a project (see [`PathMapping`]). Fails if the path
    /// already belongs to another project or a label. Returns the project.
    pub fn add_project_path(&self, name: &str, path: &str) -> Result<crate::Project> {
        self.add_project_path_in(name, path, dirs::home_dir().as_deref())
    }

    /// Removes a directory from a project's paths. Returns the project.
    pub fn remove_project_path(&self, name: &str, path: &str) -> Result<crate::Project> {
        self.remove_project_path_in(name, path, dirs::home_dir().as_deref())
    }

    /// The label or project a directory belongs to: the one with the deepest
    /// mapped path containing it. `None` when no mapped path contains it.
    pub fn path_owner(&self, dir: &Path) -> Result<Option<PathMapping>> {
        self.path_owner_in(dir, dirs::home_dir().as_deref())
    }

    /// Like [`Db::path_owner`], but no match is an error that says how to
    /// map the directory, and lists mapped paths missing on this machine
    /// (after a restore on another machine, say).
    pub fn require_path_owner(&self, dir: &Path) -> Result<PathMapping> {
        self.require_path_owner_in(dir, dirs::home_dir().as_deref())
    }

    /// Every mapped path: labels' (in label order), then projects' (in
    /// project order), each owner's paths by path.
    pub fn list_paths(&self) -> Result<Vec<PathMapping>> {
        self.list_paths_in(dirs::home_dir().as_deref())
    }

    // ---- with an explicit home directory (for tests) -------------------------

    pub(crate) fn add_label_path_in(
        &self,
        name: &str,
        path: &str,
        home: Option<&Path>,
    ) -> Result<crate::Label> {
        let id = self.label_id(name)?;
        self.add_path(PathOwnerKind::Label, id, path, home)?;
        self.get_label(name)
    }

    pub(crate) fn remove_label_path_in(
        &self,
        name: &str,
        path: &str,
        home: Option<&Path>,
    ) -> Result<crate::Label> {
        let id = self.label_id(name)?;
        self.remove_path(PathOwnerKind::Label, id, name, path, home)?;
        self.get_label(name)
    }

    pub(crate) fn add_project_path_in(
        &self,
        name: &str,
        path: &str,
        home: Option<&Path>,
    ) -> Result<crate::Project> {
        let id = self.project_id(name)?;
        self.add_path(PathOwnerKind::Project, id, path, home)?;
        self.get_project(name)
    }

    pub(crate) fn remove_project_path_in(
        &self,
        name: &str,
        path: &str,
        home: Option<&Path>,
    ) -> Result<crate::Project> {
        let id = self.project_id(name)?;
        self.remove_path(PathOwnerKind::Project, id, name, path, home)?;
        self.get_project(name)
    }

    pub(crate) fn path_owner_in(
        &self,
        dir: &Path,
        home: Option<&Path>,
    ) -> Result<Option<PathMapping>> {
        let dir = resolve(dir);
        Ok(self
            .list_paths_in(home)?
            .into_iter()
            .filter(|m| dir.starts_with(&m.absolute) || dir.starts_with(resolve(&m.absolute)))
            .max_by_key(|m| m.absolute.components().count()))
    }

    pub(crate) fn require_path_owner_in(
        &self,
        dir: &Path,
        home: Option<&Path>,
    ) -> Result<PathMapping> {
        if let Some(found) = self.path_owner_in(dir, home)? {
            return Ok(found);
        }
        let missing: Vec<String> = self
            .list_paths_in(home)?
            .into_iter()
            .filter(|m| !m.exists)
            .map(|m| format!("{} {:?} → {} (missing)", m.kind.key(), m.name, m.path))
            .collect();
        let mut message = format!(
            "no label or project is mapped to {}; map it with `solilog label add-path <label> <dir>` or `solilog project add-path <project> <dir>`",
            dir.display()
        );
        if !missing.is_empty() {
            message.push_str(&format!(
                "\nmapped paths missing on this machine:\n  {}",
                missing.join("\n  ")
            ));
        }
        anyhow::bail!(message)
    }

    pub(crate) fn list_paths_in(&self, home: Option<&Path>) -> Result<Vec<PathMapping>> {
        let rows: Vec<(String, Option<String>, Option<String>)> = self
            .conn
            .prepare_cached(
                "SELECT mp.path, l.name, p.name FROM mapped_paths mp
                 LEFT JOIN labels l ON l.id = mp.label_id
                 LEFT JOIN projects p ON p.id = mp.project_id
                 ORDER BY mp.path",
            )?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let mapping = |path: String, kind, name: String| {
            let absolute = expand_home(&path, home);
            PathMapping {
                kind,
                name,
                exists: absolute.is_dir(),
                absolute,
                path,
            }
        };
        let mut out = Vec::new();
        for label in self.list_labels()? {
            for (path, l, _) in &rows {
                if l.as_deref() == Some(label.name.as_str()) {
                    out.push(mapping(
                        path.clone(),
                        PathOwnerKind::Label,
                        label.name.clone(),
                    ));
                }
            }
        }
        for project in self.list_projects(true)? {
            for (path, _, p) in &rows {
                if p.as_deref() == Some(project.name.as_str()) {
                    out.push(mapping(
                        path.clone(),
                        PathOwnerKind::Project,
                        project.name.clone(),
                    ));
                }
            }
        }
        Ok(out)
    }

    // ---- helpers ------------------------------------------------------------

    /// A label's or project's paths, by path.
    pub(crate) fn owner_paths(&self, kind: PathOwnerKind, id: i64) -> Result<Vec<MappedPath>> {
        let column = owner_column(kind);
        Ok(self
            .conn
            .prepare_cached(&format!(
                "SELECT path FROM mapped_paths WHERE {column} = ?1 ORDER BY path"
            ))?
            .query_map([id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .map(mapped_path)
            .collect())
    }

    fn add_path(
        &self,
        kind: PathOwnerKind,
        id: i64,
        path: &str,
        home: Option<&Path>,
    ) -> Result<()> {
        let path = normalize_mapped_path(path, home)?;
        let owner: Option<(bool, i64, String)> = self
            .conn
            .query_row(
                "SELECT mp.label_id IS NOT NULL, coalesce(mp.label_id, mp.project_id),
                        coalesce(l.name, p.name)
                 FROM mapped_paths mp
                 LEFT JOIN labels l ON l.id = mp.label_id
                 LEFT JOIN projects p ON p.id = mp.project_id
                 WHERE mp.path = ?1",
                [&path],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        if let Some((is_label, owner_id, owner_name)) = owner {
            let owner_kind = if is_label {
                PathOwnerKind::Label
            } else {
                PathOwnerKind::Project
            };
            if owner_kind == kind && owner_id == id {
                return Ok(());
            }
            anyhow::bail!(
                "{path} is already mapped to {} {owner_name:?}",
                owner_kind.key()
            );
        }
        self.conn.execute(
            &format!(
                "INSERT INTO mapped_paths (path, {}) VALUES (?1, ?2)",
                owner_column(kind)
            ),
            params![path, id],
        )?;
        Ok(())
    }

    /// Removes a path given as stored (`~/…`) or in any form that normalizes
    /// to it.
    fn remove_path(
        &self,
        kind: PathOwnerKind,
        id: i64,
        name: &str,
        path: &str,
        home: Option<&Path>,
    ) -> Result<()> {
        let normalized = normalize_mapped_path(path, home)?;
        let removed = self.conn.execute(
            &format!(
                "DELETE FROM mapped_paths WHERE {} = ?1 AND path IN (?2, ?3)",
                owner_column(kind)
            ),
            params![id, normalized, path.trim()],
        )?;
        anyhow::ensure!(
            removed > 0,
            "{} {:?} has no path {normalized}",
            kind.key(),
            name.trim()
        );
        Ok(())
    }
}

fn owner_column(kind: PathOwnerKind) -> &'static str {
    match kind {
        PathOwnerKind::Label => "label_id",
        PathOwnerKind::Project => "project_id",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NewProject;
    use std::fs;

    /// A fresh temporary directory, removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("solilog-paths-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(fs::canonicalize(dir).unwrap())
        }

        fn mkdir(&self, rel: &str) -> PathBuf {
            let dir = self.0.join(rel);
            fs::create_dir_all(&dir).unwrap();
            dir
        }

        fn str(&self, rel: &str) -> String {
            self.0.join(rel).to_str().unwrap().to_owned()
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn normalization() {
        let home = Path::new("/home/someone");
        let n = |p: &str| normalize_mapped_path(p, Some(home)).unwrap();
        assert_eq!(n("/home/someone/code/reverb/"), "~/code/reverb");
        assert_eq!(n("~/code/./x/../reverb"), "~/code/reverb");
        assert_eq!(n("~"), "~");
        assert_eq!(n("/home/someone"), "~");
        assert_eq!(n("/home/someoneelse/x"), "/home/someoneelse/x");
        assert_eq!(n("/srv/code"), "/srv/code");
        assert_eq!(normalize_mapped_path("/srv/x", None).unwrap(), "/srv/x");
        assert!(normalize_mapped_path("code/reverb", Some(home)).is_err());
        assert!(normalize_mapped_path("  ", Some(home)).is_err());
        assert!(
            normalize_mapped_path("~/code", None).is_err(),
            "no home to expand"
        );
    }

    #[test]
    fn resolve_deepest_and_one_owner() {
        let tmp = TempDir::new("resolve");
        let home = tmp.mkdir("home");
        let home = Some(home.as_path());
        tmp.mkdir("home/code/acme/reverb/src");
        tmp.mkdir("home/code/acme/delay");
        let db = Db::open_in_memory().unwrap();
        db.create_label("acme/reverb").unwrap();
        db.create_project(&NewProject::new("Delay v2")).unwrap();

        let label = db
            .add_label_path_in("acme", &tmp.str("home/code/acme"), home)
            .unwrap();
        let paths: Vec<_> = label.paths.iter().map(|p| p.path.as_str()).collect();
        assert_eq!(paths, ["~/code/acme"]);
        db.add_label_path_in("ACME/reverb", "~/code/acme/reverb", home)
            .unwrap();
        db.add_project_path_in("delay V2", &tmp.str("home/code/acme/delay/"), home)
            .unwrap();
        // Adding the same path again to the same owner does nothing.
        db.add_label_path_in("acme", "~/code/acme", home).unwrap();
        // A path has one owner.
        let err = db
            .add_project_path_in("Delay v2", "~/code/acme", home)
            .unwrap_err();
        assert!(err.to_string().contains("label \"acme\""), "{err}");
        let err = db
            .add_label_path_in("acme", "~/code/acme/delay", home)
            .unwrap_err();
        assert!(err.to_string().contains("project \"Delay v2\""), "{err}");

        let owner = |rel: &str| {
            db.path_owner_in(&tmp.0.join(rel), home)
                .unwrap()
                .map(|m| (m.kind, m.name, m.path))
        };
        let label = |name: &str, path: &str| Some((PathOwnerKind::Label, name.into(), path.into()));
        assert_eq!(owner("home/code/acme"), label("acme", "~/code/acme"));
        assert_eq!(
            owner("home/code/acme/reverb/src"),
            label("acme/reverb", "~/code/acme/reverb")
        );
        assert_eq!(
            owner("home/code/acme/delay"),
            Some((
                PathOwnerKind::Project,
                "Delay v2".into(),
                "~/code/acme/delay".into()
            ))
        );
        assert_eq!(owner("home/code"), None);
        assert_eq!(owner("home/code/acmeish"), None);
        assert!(db.require_path_owner_in(&tmp.0.join("home"), home).is_err());

        // Renames carry paths along; deletions drop them.
        db.rename_label("acme", "globex/acme").unwrap();
        assert_eq!(
            owner("home/code/acme/reverb"),
            label("globex/acme/reverb", "~/code/acme/reverb")
        );
        db.delete_project("delay v2").unwrap();
        assert_eq!(
            owner("home/code/acme/delay"),
            label("globex/acme", "~/code/acme")
        );

        let removed = db
            .remove_label_path_in(
                "globex/acme/reverb",
                &tmp.str("home/code/acme/reverb"),
                home,
            )
            .unwrap();
        assert!(removed.paths.is_empty());
        assert!(
            db.remove_label_path_in("globex/acme/reverb", "~/code/acme/reverb", home)
                .is_err()
        );
        assert_eq!(
            owner("home/code/acme/reverb"),
            label("globex/acme", "~/code/acme")
        );
        db.delete_label("globex").unwrap();
        assert!(db.list_paths_in(home).unwrap().is_empty());
    }

    #[test]
    fn missing_paths_are_kept_and_reported() {
        let tmp = TempDir::new("missing");
        let db = Db::open_in_memory().unwrap();
        db.create_label("reverb").unwrap();
        db.add_label_path_in("reverb", &tmp.str("gone"), None)
            .unwrap();
        let paths = db.list_paths_in(None).unwrap();
        assert_eq!(paths.len(), 1);
        assert!(!paths[0].exists);
        let err = db.require_path_owner_in(&tmp.0, None).unwrap_err();
        assert!(err.to_string().contains("(missing)"), "{err}");
        // Created later, it matches.
        tmp.mkdir("gone/src");
        let found = db
            .require_path_owner_in(&tmp.0.join("gone/src"), None)
            .unwrap();
        assert_eq!(found.name, "reverb");
        assert!(found.exists);
        // A missing path is removed by its stored form.
        fs::remove_dir_all(tmp.0.join("gone")).unwrap();
        db.remove_label_path_in("reverb", &tmp.str("gone"), None)
            .unwrap();
    }

    #[test]
    fn scoping() {
        let mapping = |kind, name: &str| PathMapping {
            kind,
            name: name.into(),
            path: "~/x".into(),
            absolute: "/x".into(),
            exists: true,
        };
        let mut filter = IssueFilter::default();
        mapping(PathOwnerKind::Label, "acme").scope_filter(&mut filter);
        mapping(PathOwnerKind::Label, "ACME").scope_filter(&mut filter);
        mapping(PathOwnerKind::Project, "v1").scope_filter(&mut filter);
        assert_eq!(
            (filter.labels, filter.projects),
            (vec!["acme".into()], vec!["v1".into()])
        );
        let mut new = NewIssue::new("x");
        mapping(PathOwnerKind::Label, "acme").scope_new_issue(&mut new);
        mapping(PathOwnerKind::Project, "v1").scope_new_issue(&mut new);
        assert_eq!(
            (new.labels, new.projects),
            (vec!["acme".into()], vec!["v1".into()])
        );
    }
}
