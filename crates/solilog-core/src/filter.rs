//! Issue filters.

use std::fmt;

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

/// One-line summary: `open, status: todo|blocked, label: acme & bug, not: x,
/// project: a|b`, or `all issues`.
impl fmt::Display for IssueFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if self.open_only {
            parts.push("open".to_string());
        }
        let statuses = self.statuses.iter().map(|s| s.key().to_string()).collect();
        for (name, values, sep) in [
            ("status", statuses, "|"),
            ("label", self.labels.clone(), " & "),
            ("not", self.without_labels.clone(), ", "),
            ("project", self.projects.clone(), "|"),
        ] {
            if !values.is_empty() {
                parts.push(format!("{name}: {}", values.join(sep)));
            }
        }
        if self.no_project {
            parts.push("no project".to_string());
        }
        if parts.is_empty() {
            f.write_str("all issues")
        } else {
            f.write_str(&parts.join(", "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary() {
        assert_eq!(IssueFilter::default().to_string(), "all issues");
        let filter = IssueFilter {
            open_only: true,
            statuses: vec![Status::Todo, Status::Blocked],
            labels: vec!["acme".into(), "bug".into()],
            without_labels: vec!["x".into()],
            no_project: true,
            ..Default::default()
        };
        assert_eq!(
            filter.to_string(),
            "open, status: todo|blocked, label: acme & bug, not: x, no project"
        );
    }
}
