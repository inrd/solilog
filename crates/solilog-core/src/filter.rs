//! Issue filters.

use serde::{Deserialize, Serialize};

use crate::issue::{Issue, Status};
use crate::label::label_matches;

/// Which issues [`Db::list_issues`](crate::Db::list_issues) returns. The
/// default matches everything; every non-empty criterion must hold.
///
/// Serialized as stored in views; missing fields take their default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct IssueFilter {
    /// Only these statuses (any status if empty).
    pub statuses: Vec<Status>,
    /// Exclude done and cancelled issues.
    pub open_only: bool,
    /// The issue has every one of these labels, or a sublabel of each.
    pub labels: Vec<String>,
    /// The issue has none of these labels, nor any of their sublabels.
    pub without_labels: Vec<String>,
    /// The issue is in at least one of these projects.
    pub projects: Vec<String>,
    /// The issue is in no project. Incompatible with `projects`.
    pub no_project: bool,
}

impl IssueFilter {
    /// Assumes names have been resolved to their stored spelling
    /// (`Db::list_issues` does this); comparison is case-insensitive anyway.
    pub fn matches(&self, issue: &Issue) -> bool {
        let has = |filter: &String| issue.labels.iter().any(|l| label_matches(l, filter));
        (self.statuses.is_empty() || self.statuses.contains(&issue.status))
            && !(self.open_only && issue.status.is_closed())
            && self.labels.iter().all(has)
            && !self.without_labels.iter().any(has)
            && (self.projects.is_empty()
                || self
                    .projects
                    .iter()
                    .any(|p| issue.projects.iter().any(|ip| ip.eq_ignore_ascii_case(p))))
            && (!self.no_project || issue.projects.is_empty())
    }
}
