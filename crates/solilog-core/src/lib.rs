//! Solilog core: domain types, storage and every domain operation.
//!
//! Frontends (the GPUI app, the CLI) call into this crate and contain no
//! business logic of their own. This crate must never depend on GPUI.

mod db;
mod filter;
mod issue;
mod label;
mod order;
mod project;
mod view;

pub use db::{Db, default_path};
pub use filter::IssueFilter;
pub use issue::{ISSUE_PREFIX, Issue, IssueChange, IssueId, NewIssue, Status};
pub use label::{LABEL_SEPARATOR, Label, label_matches, normalize_label_name, renamed_label};
pub use order::Placement;
pub use project::{NewProject, Project, ProjectStatus, ProjectUpdate};
pub use view::View;
