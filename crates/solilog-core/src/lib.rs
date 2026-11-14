//! Solilog core: domain types, storage and every domain operation.
//!
//! Frontends (the GPUI app, the CLI) call into this crate and contain no
//! business logic of their own. This crate must never depend on GPUI.

mod db;
mod issue;

pub use db::{Db, IssueFilter, default_path};
pub use issue::{ISSUE_PREFIX, Issue, IssueId, Status};
