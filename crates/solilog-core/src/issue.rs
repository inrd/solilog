//! Domain types.

use std::fmt;
use std::str::FromStr;

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

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
}

/// Arguments of [`Db::create_issue`](crate::Db::create_issue).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewIssue {
    pub title: String,
    pub status: Status,
    /// Label paths; missing labels are created.
    pub labels: Vec<String>,
    /// Project names; the projects must exist. Their default labels are added.
    pub projects: Vec<String>,
}

impl NewIssue {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            status: Status::Todo,
            labels: Vec::new(),
            projects: Vec::new(),
        }
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
