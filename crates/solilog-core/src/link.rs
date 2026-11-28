//! Links: where an issue's work lives outside Solilog. The branch it is
//! on, its pull request, other pages (an upstream bug, a design, docs).
//!
//! Apart from attachments (files copied in) and the description (the
//! spec): a short list of pointers, found at a glance and in `issue
//! context`. Commits and changed files aren't links: a receipt records them
//! when the work is handed off (`review.rs`).
//!
//! A link is unique per issue (kind and target): adding it again changes
//! nothing. Links are added and removed, never edited. Deleting an issue
//! deletes its links; archiving keeps them.

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use anyhow::{Context as _, Result};
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use rusqlite::{OptionalExtension as _, Row, params};
use serde::{Deserialize, Serialize};

use crate::Db;
use crate::activity::{ActivityKind, Actor};
use crate::issue::{Issue, IssueChange, IssueId};

/// Identifies a link, across all issues. Displayed as a plain number;
/// parsed from `12` or `#12`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LinkId(pub i64);

impl fmt::Display for LinkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for LinkId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        s.strip_prefix('#')
            .unwrap_or(s)
            .parse::<i64>()
            .ok()
            .filter(|n| *n > 0)
            .map(LinkId)
            .ok_or_else(|| format!("invalid link id {s:?} (expected a number)"))
    }
}

impl ToSql for LinkId {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        self.0.to_sql()
    }
}

impl FromSql for LinkId {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        i64::column_result(value).map(LinkId)
    }
}

/// What a link points to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    /// A git branch, by name (`fix/reverb-tail`).
    Branch,
    /// A pull or merge request, by URL.
    Pr,
    /// Any other page, by URL.
    Url,
}

impl LinkKind {
    pub const ALL: [LinkKind; 3] = [LinkKind::Branch, LinkKind::Pr, LinkKind::Url];

    /// Stable identifier, used in the database, JSON and the CLI.
    pub fn key(self) -> &'static str {
        match self {
            Self::Branch => "branch",
            Self::Pr => "pr",
            Self::Url => "url",
        }
    }

    /// For display.
    pub fn label(self) -> &'static str {
        match self {
            Self::Branch => "Branch",
            Self::Pr => "Pull request",
            Self::Url => "Link",
        }
    }

    /// The kind a target is most likely: a URL to a pull or merge request
    /// is a `pr`, any other URL a `url`, anything else a `branch`.
    pub fn infer(target: &str) -> Self {
        let target = target.trim();
        if !is_url(target) {
            Self::Branch
        } else if is_pr_url(target) {
            Self::Pr
        } else {
            Self::Url
        }
    }
}

impl fmt::Display for LinkKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

impl FromStr for LinkKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        Self::ALL
            .into_iter()
            .find(|k| k.key().eq_ignore_ascii_case(s))
            .ok_or_else(|| format!("unknown link kind {s:?} (expected branch, pr or url)"))
    }
}

impl ToSql for LinkKind {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(self.key().into())
    }
}

impl FromSql for LinkKind {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        value
            .as_str()?
            .parse()
            .map_err(|e: String| FromSqlError::Other(e.into()))
    }
}

/// A link on an issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Link {
    pub id: LinkId,
    pub issue_id: IssueId,
    pub kind: LinkKind,
    /// The branch name or the URL.
    pub target: String,
    /// What it is, in a few words; `None` when not given.
    pub title: Option<String>,
    /// Who added it.
    pub author: Actor,
    /// Unix timestamp (seconds).
    pub created_at: i64,
}

impl Link {
    /// The text to show for it: its title, or its target.
    pub fn display_text(&self) -> &str {
        self.title.as_deref().unwrap_or(&self.target)
    }

    /// `true` when the target is a URL to open (not a branch).
    pub fn is_url(&self) -> bool {
        self.kind != LinkKind::Branch
    }
}

/// A link to add, for [`IssueChange::AddLinks`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewLink {
    pub target: String,
    /// `None` infers it from the target ([`LinkKind::infer`]).
    pub kind: Option<LinkKind>,
    /// Empty or `None` for none.
    pub title: Option<String>,
}

impl NewLink {
    pub fn new(target: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            ..Default::default()
        }
    }
}

/// `scheme://rest`, without whitespace: a URL to open. Branch names can't
/// hold `:`, so a target is never both.
fn is_url(s: &str) -> bool {
    let Some((scheme, rest)) = s.split_once("://") else {
        return false;
    };
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
        && !rest.is_empty()
        && !s.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// An http(s) URL to a pull or merge request on the usual forges: GitHub
/// (`/pull/12`), GitLab (`/merge_requests/12`), Gitea and Forgejo
/// (`/pulls/12`), Bitbucket (`/pull-requests/12`).
fn is_pr_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let Some(rest) = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
    else {
        return false;
    };
    let path = rest.split(['?', '#']).next().unwrap_or_default();
    let segments: Vec<&str> = path.split('/').collect();
    segments.windows(2).any(|w| {
        matches!(w[0], "pull" | "pulls" | "merge_requests" | "pull-requests")
            && !w[1].is_empty()
            && w[1].chars().all(|c| c.is_ascii_digit())
    })
}

/// A valid git branch name (`git check-ref-format --branch`, roughly).
fn is_branch_name(s: &str) -> bool {
    !s.is_empty()
        && s != "@"
        && !s.starts_with(['-', '/', '.'])
        && !s.ends_with(['/', '.'])
        && !s.ends_with(".lock")
        && !s.contains("..")
        && !s.contains("//")
        && !s.contains("@{")
        && !s.contains("/.")
        && !s
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(c))
}

/// Cleans up a link to add and checks it: the kind (inferred when not
/// given), the trimmed target, and the title (one line; `None` when
/// empty). A `pr` or `url` needs a URL, a `branch` a valid branch name.
pub fn normalize_link(new: &NewLink) -> Result<(LinkKind, String, Option<String>)> {
    let target = new.target.trim();
    anyhow::ensure!(
        !target.is_empty(),
        "a link needs a target (a branch or a URL)"
    );
    let kind = new.kind.unwrap_or_else(|| LinkKind::infer(target));
    match kind {
        LinkKind::Branch => anyhow::ensure!(
            is_branch_name(target),
            "{target:?} is neither a URL nor a valid branch name"
        ),
        LinkKind::Pr | LinkKind::Url => anyhow::ensure!(
            is_url(target),
            "{target:?} is not a URL (a {kind} link needs one, like https://…)"
        ),
    }
    let title = new
        .title
        .as_deref()
        .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|t| !t.is_empty());
    Ok((kind, target.to_owned(), title))
}

const LINK_COLUMNS: &str = "id, issue_id, kind, target, title, author, created_at";

fn link_from_row(row: &Row) -> rusqlite::Result<Link> {
    Ok(Link {
        id: row.get(0)?,
        issue_id: row.get(1)?,
        kind: row.get(2)?,
        target: row.get(3)?,
        title: row.get(4)?,
        author: row.get(5)?,
        created_at: row.get(6)?,
    })
}

impl Db {
    /// Adds a link to an issue (nothing changes if it has it already) and
    /// returns the issue.
    pub fn add_link(&self, issue: IssueId, link: &NewLink) -> Result<Issue> {
        self.update_issue(issue, &IssueChange::AddLinks(vec![link.clone()]))
    }

    /// Removes a link and returns the issue it was on.
    pub fn remove_link(&self, id: LinkId) -> Result<Issue> {
        let link = self.get_link(id)?;
        self.update_issue(link.issue_id, &IssueChange::RemoveLinks(vec![id]))
    }

    /// One link.
    pub fn get_link(&self, id: LinkId) -> Result<Link> {
        self.conn
            .query_row(
                &format!("SELECT {LINK_COLUMNS} FROM links WHERE id = ?1"),
                [id],
                link_from_row,
            )
            .optional()?
            .with_context(|| format!("no link {id}"))
    }

    /// Adds links to an issue, skipping those it has (same kind and
    /// target). Returns the targets actually added. Call inside a
    /// transaction.
    pub(crate) fn insert_links(&self, issue: IssueId, links: &[NewLink]) -> Result<Vec<String>> {
        self.issue_state(issue)?;
        let mut added = Vec::new();
        for link in links {
            let (kind, target, title) = normalize_link(link)?;
            let inserted = self.conn.execute(
                "INSERT OR IGNORE INTO links (issue_id, kind, target, title, author)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![issue, kind, target, title, self.actor],
            )?;
            if inserted > 0 {
                added.push(target);
            }
        }
        Ok(added)
    }

    /// Removes links from an issue; each must be on it. Returns their
    /// targets. Call inside a transaction.
    pub(crate) fn delete_links(&self, issue: IssueId, ids: &[LinkId]) -> Result<Vec<String>> {
        self.issue_state(issue)?;
        let mut removed = Vec::new();
        for &id in ids {
            let link = self.get_link(id)?;
            anyhow::ensure!(
                link.issue_id == issue,
                "link {id} is on {}, not {issue}",
                link.issue_id
            );
            self.conn.execute("DELETE FROM links WHERE id = ?1", [id])?;
            removed.push(link.target);
        }
        Ok(removed)
    }

    /// The links of every issue (or only of `only`), in the order added.
    pub(crate) fn links_per_issue(
        &self,
        only: Option<IssueId>,
    ) -> Result<HashMap<IssueId, Vec<Link>>> {
        let mut map: HashMap<IssueId, Vec<Link>> = HashMap::new();
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT {LINK_COLUMNS} FROM links WHERE ?1 IS NULL OR issue_id = ?1
             ORDER BY issue_id, id"
        ))?;
        for link in stmt.query_map([only], link_from_row)? {
            let link = link?;
            map.entry(link.issue_id).or_default().push(link);
        }
        Ok(map)
    }

    /// Records the activity of links added or removed.
    pub(crate) fn record_links_activity(
        &self,
        issue: IssueId,
        kind: ActivityKind,
        targets: &[String],
    ) -> Result<()> {
        if targets.is_empty() {
            return Ok(());
        }
        self.record_activity(issue, kind, &targets.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IssueEdit, NewIssue};

    #[test]
    fn kinds_are_inferred() {
        let infer = LinkKind::infer;
        assert_eq!(infer("fix/reverb-tail"), LinkKind::Branch);
        assert_eq!(infer(" main "), LinkKind::Branch);
        assert_eq!(infer("https://github.com/o/r/pull/12"), LinkKind::Pr);
        assert_eq!(
            infer("https://github.com/o/r/pull/12/files#diff"),
            LinkKind::Pr
        );
        assert_eq!(
            infer("https://gitlab.com/g/p/-/merge_requests/3?tab=notes"),
            LinkKind::Pr
        );
        assert_eq!(infer("http://git.example.test/o/r/pulls/7"), LinkKind::Pr);
        assert_eq!(
            infer("https://bitbucket.org/o/r/pull-requests/9"),
            LinkKind::Pr
        );
        assert_eq!(infer("https://github.com/o/r/pulls"), LinkKind::Url);
        assert_eq!(infer("https://github.com/o/r/issues/4"), LinkKind::Url);
        assert_eq!(infer("obsidian://open?vault=x"), LinkKind::Url);
        assert_eq!("PR".parse(), Ok(LinkKind::Pr));
        assert!("commit".parse::<LinkKind>().is_err());
    }

    #[test]
    fn links_are_checked() {
        let link = |target: &str, kind: Option<LinkKind>, title: Option<&str>| NewLink {
            target: target.into(),
            kind,
            title: title.map(Into::into),
        };
        assert_eq!(
            normalize_link(&link(" feat/x ", None, Some("  The   branch \n"))).unwrap(),
            (LinkKind::Branch, "feat/x".into(), Some("The branch".into()))
        );
        assert_eq!(
            normalize_link(&link(
                "https://example.test/a",
                Some(LinkKind::Pr),
                Some(" ")
            ))
            .unwrap(),
            (LinkKind::Pr, "https://example.test/a".into(), None)
        );
        for bad in [
            "",
            "two words",
            "a..b",
            "-x",
            "x.lock",
            "x/",
            "a:b",
            "https://",
        ] {
            assert!(normalize_link(&link(bad, None, None)).is_err(), "{bad:?}");
        }
        assert!(normalize_link(&link("main", Some(LinkKind::Url), None)).is_err());
        assert!(normalize_link(&link("https://x.test", Some(LinkKind::Branch), None)).is_err());
    }

    #[test]
    fn add_list_remove() {
        let mut db = Db::open_in_memory().unwrap();
        let a = db.create_issue(&NewIssue::new("a")).unwrap().id;
        let b = db.create_issue(&NewIssue::new("b")).unwrap().id;
        let agent = Actor::parse("claude:x").unwrap();
        db.set_actor(agent.clone());

        let issue = db
            .add_link(
                a,
                &NewLink {
                    title: Some("Fix".into()),
                    ..NewLink::new("fix/tail")
                },
            )
            .unwrap();
        let issue = db
            .update_issue(
                issue.id,
                &IssueChange::AddLinks(vec![
                    NewLink::new("https://github.com/o/r/pull/3"),
                    // Already there: nothing changes, not even the title.
                    NewLink {
                        title: Some("Other".into()),
                        ..NewLink::new("fix/tail")
                    },
                ]),
            )
            .unwrap();
        assert_eq!(issue.links.len(), 2);
        let (branch, pr) = (&issue.links[0], &issue.links[1]);
        assert_eq!(branch.kind, LinkKind::Branch);
        assert_eq!(branch.display_text(), "Fix");
        assert!(!branch.is_url());
        assert_eq!(branch.author, agent);
        assert_eq!(pr.kind, LinkKind::Pr);
        assert_eq!(pr.display_text(), "https://github.com/o/r/pull/3");
        assert_eq!(
            db.list_issues(&Default::default()).unwrap()[1].links,
            issue.links
        );
        let json = serde_json::to_value(branch).unwrap();
        assert_eq!(json["kind"], "branch");
        assert_eq!(json["target"], "fix/tail");
        assert_eq!(json["issue_id"], "SOL-1");

        // Activities, and an agent's links need review.
        let pending = db.unreviewed_activities(a).unwrap();
        assert_eq!(pending[0].kind, ActivityKind::LinkAdded);
        assert_eq!(pending[0].detail, "https://github.com/o/r/pull/3");
        assert_eq!(pending[1].detail, "fix/tail");

        // A link is removed from its own issue only; all or nothing.
        let edit = IssueEdit {
            title: Some("renamed".into()),
            changes: vec![IssueChange::RemoveLinks(vec![branch.id])],
            ..Default::default()
        };
        assert!(db.edit_issue(b, &edit).is_err());
        assert_eq!(db.get_issue(b).unwrap().title, "b");
        assert!(db.add_link(a, &NewLink::new("not a branch")).is_err());
        assert_eq!(db.get_issue(a).unwrap().links.len(), 2);

        let issue = db.remove_link(branch.id).unwrap();
        assert_eq!(issue.links, std::slice::from_ref(pr));
        assert!(db.remove_link(branch.id).is_err());
        let activities = db.issue_activities(a).unwrap();
        assert_eq!(activities[0].kind, ActivityKind::LinkRemoved);
        assert_eq!(activities[0].detail, "fix/tail");

        // The same link can be on several issues; deleting one keeps the other's.
        db.update_issues(
            &[a, b],
            &IssueChange::AddLinks(vec![NewLink::new("https://example.test/spec")]),
        )
        .unwrap();
        assert_eq!(db.get_issue(b).unwrap().links.len(), 1);
        db.set_archived(a, true).unwrap();
        assert_eq!(db.get_issue(a).unwrap().links.len(), 2);
        db.delete_issue(a).unwrap();
        assert!(db.get_link(pr.id).is_err());
        assert_eq!(db.get_issue(b).unwrap().links.len(), 1);
    }
}
