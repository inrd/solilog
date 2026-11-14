//! Issue filters.

use crate::issue::{Issue, Status};
use crate::label::label_matches;

/// Which issues [`Db::list_issues`](crate::Db::list_issues) returns. The
/// default matches everything; every non-empty criterion must hold.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IssueFilter {
    /// Only these statuses (any status if empty).
    pub statuses: Vec<Status>,
    /// Exclude done and cancelled issues.
    pub open_only: bool,
    /// The issue has every one of these labels, or a sublabel of each.
    pub labels: Vec<String>,
    /// The issue has none of these labels, nor any of their sublabels.
    pub without_labels: Vec<String>,
}

impl IssueFilter {
    /// Assumes label names have been resolved to their stored spelling
    /// (`Db::list_issues` does this); comparison is case-insensitive anyway.
    pub fn matches(&self, issue: &Issue) -> bool {
        let has = |filter: &String| issue.labels.iter().any(|l| label_matches(l, filter));
        (self.statuses.is_empty() || self.statuses.contains(&issue.status))
            && !(self.open_only && issue.status.is_closed())
            && self.labels.iter().all(has)
            && !self.without_labels.iter().any(has)
    }
}
