//! Domain types.

use std::fmt;
use std::ops::Range;
use std::str::FromStr;

use anyhow::Result;
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::attachment::Attachment;
use crate::checklist::{ChecklistItem, checklist_progress};
use crate::db::Db;

/// Prefix of human-facing issue ids (`SOL-12`).
pub const ISSUE_PREFIX: &str = "SOL";

/// Identifies an issue. Displayed and serialized as `SOL-<n>`; parsed from
/// `SOL-<n>` (case-insensitive) or a bare `<n>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IssueId(pub i64);

impl fmt::Display for IssueId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{ISSUE_PREFIX}-{}", self.0)
    }
}

impl FromStr for IssueId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let digits = s
            .get(..ISSUE_PREFIX.len() + 1)
            .filter(|p| p.eq_ignore_ascii_case(&format!("{ISSUE_PREFIX}-")))
            .map_or(s, |p| &s[p.len()..]);
        digits
            .parse::<i64>()
            .ok()
            .filter(|n| *n > 0)
            .map(IssueId)
            .ok_or_else(|| format!("invalid issue id {s:?} (expected {ISSUE_PREFIX}-<n> or <n>)"))
    }
}

impl Serialize for IssueId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for IssueId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

impl ToSql for IssueId {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        self.0.to_sql()
    }
}

impl FromSql for IssueId {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        i64::column_result(value).map(IssueId)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Issue {
    pub id: IssueId,
    pub title: String,
    /// Plain text, possibly several paragraphs; empty when there is none.
    /// Stored as written (see [`normalize_description`]) so that it can be
    /// read as Markdown later.
    pub description: String,
    pub status: Status,
    /// Unix timestamp (seconds).
    pub created_at: i64,
    /// Archived issues are left out of listings, views and counts.
    pub archived: bool,
    /// Label paths (`acme/reverb`), sorted. Only the labels given to the
    /// issue; their ancestors are implied, not listed.
    pub labels: Vec<String>,
    /// Names of the projects the issue is in, sorted.
    pub projects: Vec<String>,
    /// Checklist items, in order; empty when there is no checklist.
    pub checklist: Vec<ChecklistItem>,
    /// Attached files, in the order they were added.
    pub attachments: Vec<Attachment>,
}

impl Issue {
    /// Done and total checklist items, or `None` without a checklist.
    pub fn checklist_progress(&self) -> Option<(usize, usize)> {
        checklist_progress(&self.checklist)
    }
}

/// Arguments of [`Db::create_issue`](crate::Db::create_issue).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewIssue {
    pub title: String,
    pub description: String,
    pub status: Status,
    /// Label paths; missing labels are created.
    pub labels: Vec<String>,
    /// Project names; the projects must exist. Their default labels are added.
    pub projects: Vec<String>,
    /// Checklist item texts, in order.
    pub checklist: Vec<String>,
}

impl NewIssue {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            description: String::new(),
            status: Status::Todo,
            labels: Vec::new(),
            projects: Vec::new(),
            checklist: Vec::new(),
        }
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    pub fn status(mut self, status: Status) -> Self {
        self.status = status;
        self
    }

    pub fn labels(mut self, labels: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.labels = labels.into_iter().map(Into::into).collect();
        self
    }

    pub fn projects(mut self, projects: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.projects = projects.into_iter().map(Into::into).collect();
        self
    }

    pub fn checklist(mut self, items: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.checklist = items.into_iter().map(Into::into).collect();
        self
    }
}

/// A change to apply to issues with
/// [`Db::update_issues`](crate::Db::update_issues).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueChange {
    SetStatus(Status),
    /// Archive (`true`) or unarchive.
    SetArchived(bool),
    /// Label paths; missing labels are created.
    AddLabels(Vec<String>),
    /// Exact labels (not sublabels); unknown labels are an error.
    RemoveLabels(Vec<String>),
    /// Project names; adds the projects' default labels.
    AddToProjects(Vec<String>),
    /// Project names; issues keep their labels.
    RemoveFromProjects(Vec<String>),
}

/// Changes to the fields of one issue, for
/// [`Db::edit_issue`](crate::Db::edit_issue); `None` fields are left alone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IssueEdit {
    pub title: Option<String>,
    /// Empty to remove the description.
    pub description: Option<String>,
}

/// Trims a title and checks that it is not empty.
pub(crate) fn normalize_title(title: &str) -> anyhow::Result<&str> {
    let title = title.trim();
    anyhow::ensure!(!title.is_empty(), "issue title cannot be empty");
    Ok(title)
}

/// Mentions of issues in a text (`SOL-12`, any case), with their byte
/// ranges, in order. A mention is a whole word: `XSOL-1` and `SOL-1a` are
/// not mentions. Whether the issues exist is up to the caller.
pub fn issue_mentions(text: &str) -> Vec<(Range<usize>, IssueId)> {
    let is_word_char = |c: char| c.is_alphanumeric() || c == '_';
    let prefix = ISSUE_PREFIX.len() + 1;
    let mut mentions = Vec::new();
    for (start, _) in text.match_indices('-') {
        let Some(start) = start.checked_sub(ISSUE_PREFIX.len()) else {
            continue;
        };
        let Some(head) = text.get(start..start + prefix) else {
            continue;
        };
        if !head[..ISSUE_PREFIX.len()].eq_ignore_ascii_case(ISSUE_PREFIX)
            || text[..start].chars().next_back().is_some_and(is_word_char)
        {
            continue;
        }
        let rest = &text[start + prefix..];
        let digits = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        let end = start + prefix + digits;
        if digits == 0 || text[end..].chars().next().is_some_and(is_word_char) {
            continue;
        }
        if let Ok(id) = text[start..end].parse() {
            mentions.push((start..end, id));
        }
    }
    mentions
}

/// Web addresses in a text (`https://…` or `http://…`, any case), as byte
/// ranges, in order. An address starts a word and runs to the next space,
/// `<`, `>`, `"` or `` ` ``; punctuation at its end (`.`, `,`, `)` closing a
/// parenthesis opened before it…) is left out, so `(see https://x.org/a).`
/// gives `https://x.org/a`.
pub fn web_links(text: &str) -> Vec<Range<usize>> {
    const SCHEMES: [&str; 2] = ["https://", "http://"];
    let is_end = |c: char| c.is_whitespace() || c.is_control() || "<>\"`".contains(c);
    let mut links = Vec::new();
    let mut from = 0;
    while let Some(start) = text[from..].find(['h', 'H']).map(|i| from + i) {
        from = start + 1;
        let Some(scheme) = SCHEMES.iter().find(|s| {
            text.get(start..start + s.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(s))
        }) else {
            continue;
        };
        if text[..start]
            .chars()
            .next_back()
            .is_some_and(char::is_alphanumeric)
        {
            continue;
        }
        let rest = &text[start..];
        let mut link = &rest[..rest.find(is_end).unwrap_or(rest.len())];
        loop {
            let unbalanced = |open, close| {
                link.ends_with(close) && link.matches(open).count() < link.matches(close).count()
            };
            if link.ends_with(['.', ',', ';', ':', '!', '?', '\'', '*'])
                || unbalanced('(', ')')
                || unbalanced('[', ']')
            {
                link = &link[..link.len() - 1];
            } else {
                break;
            }
        }
        if link.len() > scheme.len() {
            links.push(start..start + link.len());
            from = start + link.len();
        }
    }
    links
}

/// Cleans up a description without changing its meaning as text or as
/// Markdown: line endings become `\n`, trailing whitespace on each line and
/// blank lines at both ends are dropped. Indentation is kept.
pub fn normalize_description(description: &str) -> String {
    let lines: Vec<&str> = description.lines().map(str::trim_end).collect();
    let first = lines.iter().position(|l| !l.is_empty());
    let last = lines.iter().rposition(|l| !l.is_empty());
    match (first, last) {
        (Some(first), Some(last)) => lines[first..=last].join("\n"),
        _ => String::new(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Backlog,
    Todo,
    InProgress,
    Done,
    Blocked,
    Cancelled,
}

impl Status {
    /// All statuses, in menu order.
    pub const ALL: [Status; 6] = [
        Status::Backlog,
        Status::Todo,
        Status::InProgress,
        Status::Blocked,
        Status::Done,
        Status::Cancelled,
    ];

    /// All statuses, in the order the issue list groups them: work under way
    /// first, finished work last.
    pub const LIST_ORDER: [Status; 6] = [
        Status::InProgress,
        Status::Todo,
        Status::Blocked,
        Status::Backlog,
        Status::Done,
        Status::Cancelled,
    ];

    /// Position of this status in [`Status::LIST_ORDER`].
    pub fn list_rank(self) -> usize {
        Self::LIST_ORDER
            .iter()
            .position(|&s| s == self)
            .unwrap_or(usize::MAX)
    }

    /// Stable identifier, used in the database, JSON and on the command line.
    pub fn key(self) -> &'static str {
        match self {
            Status::Backlog => "backlog",
            Status::Todo => "todo",
            Status::InProgress => "in_progress",
            Status::Done => "done",
            Status::Blocked => "blocked",
            Status::Cancelled => "cancelled",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            Status::Backlog => "Backlog",
            Status::Todo => "Todo",
            Status::InProgress => "In Progress",
            Status::Done => "Done",
            Status::Blocked => "Blocked",
            Status::Cancelled => "Cancelled",
        }
    }

    /// Whether the issue is finished (done or cancelled).
    pub fn is_closed(self) -> bool {
        matches!(self, Status::Done | Status::Cancelled)
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

/// Lenient parsing for humans and agents: `in_progress`, `in-progress`,
/// `In Progress` and `inprogress` are all accepted.
impl FromStr for Status {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let norm = |s: &str| {
            s.chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
                .to_lowercase()
        };
        let wanted = norm(s);
        Self::ALL
            .into_iter()
            .find(|st| norm(st.key()) == wanted)
            .ok_or_else(|| {
                let keys: Vec<_> = Self::ALL.iter().map(|s| s.key()).collect();
                format!(
                    "unknown status {s:?} (expected one of: {})",
                    keys.join(", ")
                )
            })
    }
}

impl ToSql for Status {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.key()))
    }
}

impl FromSql for Status {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let key = value.as_str()?;
        Status::from_key(key)
            .ok_or_else(|| FromSqlError::Other(format!("unknown status {key:?}").into()))
    }
}

/// Setting holding the collapsed status sections, as comma-separated keys.
const COLLAPSED_STATUSES: &str = "collapsed_statuses";

impl Db {
    /// The statuses whose section is collapsed (its issues hidden) in the
    /// GUI issue list, in [`Status::LIST_ORDER`]. Display only: listings and
    /// filters are unaffected.
    pub fn collapsed_statuses(&self) -> Result<Vec<Status>> {
        let setting = self.setting(COLLAPSED_STATUSES)?.unwrap_or_default();
        // Unknown keys are ignored rather than failing the whole GUI.
        let collapsed: Vec<Status> = setting.split(',').filter_map(|k| k.parse().ok()).collect();
        Ok(Status::LIST_ORDER
            .into_iter()
            .filter(|s| collapsed.contains(s))
            .collect())
    }

    /// Collapses or expands a status's section in the GUI issue list;
    /// returns the collapsed statuses.
    pub fn set_status_collapsed(&self, status: Status, collapsed: bool) -> Result<Vec<Status>> {
        let mut statuses = self.collapsed_statuses()?;
        statuses.retain(|&s| s != status);
        if collapsed {
            statuses.push(status);
        }
        self.set_collapsed_statuses(&statuses)
    }

    /// Sets which statuses' sections are collapsed in the GUI issue list;
    /// returns them.
    pub fn set_collapsed_statuses(&self, statuses: &[Status]) -> Result<Vec<Status>> {
        let keys: Vec<_> = Status::LIST_ORDER
            .into_iter()
            .filter(|s| statuses.contains(s))
            .map(Status::key)
            .collect();
        let value = keys.join(",");
        self.set_setting(COLLAPSED_STATUSES, (!value.is_empty()).then_some(&*value))?;
        self.collapsed_statuses()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_order_has_every_status_once() {
        let mut ranks: Vec<_> = Status::ALL.iter().map(|s| s.list_rank()).collect();
        ranks.sort();
        assert_eq!(ranks, (0..Status::ALL.len()).collect::<Vec<_>>());
        assert_eq!(Status::InProgress.list_rank(), 0);
    }

    #[test]
    fn issue_id_parsing() {
        assert_eq!("SOL-12".parse(), Ok(IssueId(12)));
        assert_eq!("sol-12".parse(), Ok(IssueId(12)));
        assert_eq!(" 12 ".parse(), Ok(IssueId(12)));
        assert!("SOL-".parse::<IssueId>().is_err());
        assert!("0".parse::<IssueId>().is_err());
        assert!("FOO-1".parse::<IssueId>().is_err());
        assert_eq!(IssueId(7).to_string(), "SOL-7");
    }

    #[test]
    fn mentions() {
        let text = "See SOL-12, (sol-3) and SOL-4.\nNot XSOL-5, SOL-6a, SOL-, SOL-0 or é-SOL-7_";
        let found: Vec<_> = issue_mentions(text)
            .into_iter()
            .map(|(range, id)| (&text[range], id.0))
            .collect();
        assert_eq!(found, [("SOL-12", 12), ("sol-3", 3), ("SOL-4", 4)]);
        assert_eq!(issue_mentions("é-SOL-8"), [(3..8, IssueId(8))]);
        assert!(issue_mentions("").is_empty());
    }

    #[test]
    fn web_links_in_text() {
        let found = |text: &str| -> Vec<String> {
            web_links(text)
                .into_iter()
                .map(|range| text[range].to_string())
                .collect()
        };
        assert_eq!(
            found("See https://example.org/a?b=1#c, HTTP://x.y and http://z.com."),
            ["https://example.org/a?b=1#c", "HTTP://x.y", "http://z.com"]
        );
        assert_eq!(
            found(
                "(see https://en.wikipedia.org/wiki/Rust_(language)) or [doc](https://x.org/doc)!"
            ),
            [
                "https://en.wikipedia.org/wiki/Rust_(language)",
                "https://x.org/doc"
            ]
        );
        assert_eq!(
            found("<https://x.org/é> \"https://y.org\"\n`https://z.org`"),
            ["https://x.org/é", "https://y.org", "https://z.org"]
        );
        assert!(found("xhttps://x.org https:// http://. ftp://x.org h").is_empty());
        assert_eq!(found("é https://x.org/SOL-12"), ["https://x.org/SOL-12"]);
    }

    #[test]
    fn description_normalization() {
        assert_eq!(normalize_description(""), "");
        assert_eq!(normalize_description(" \n\t\n"), "");
        assert_eq!(
            normalize_description("\r\n\n  indented  \r\n\r\n- item\t\n\n"),
            "  indented\n\n- item"
        );
    }

    #[test]
    fn status_keys_round_trip() {
        for s in Status::ALL {
            assert_eq!(Status::from_key(s.key()), Some(s));
            assert_eq!(s.key().parse(), Ok(s));
            assert_eq!(s.label().parse(), Ok(s));
            assert_eq!(serde_json::to_value(s).unwrap(), s.key());
        }
        assert_eq!("in-progress".parse(), Ok(Status::InProgress));
        assert!("nope".parse::<Status>().is_err());
    }

    #[test]
    fn collapsed_statuses() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.collapsed_statuses().unwrap().is_empty());
        db.set_status_collapsed(Status::Cancelled, true).unwrap();
        let collapsed = db.set_status_collapsed(Status::Done, true).unwrap();
        // In list order, whatever the order they were collapsed in.
        assert_eq!(collapsed, [Status::Done, Status::Cancelled]);
        assert_eq!(
            db.set_status_collapsed(Status::Done, true).unwrap(),
            collapsed
        );
        assert_eq!(
            db.set_status_collapsed(Status::Done, false).unwrap(),
            [Status::Cancelled]
        );
        assert!(db.set_collapsed_statuses(&[]).unwrap().is_empty());
    }
}
