//! Solilog core: domain types, storage and every domain operation.
//!
//! Frontends (the GPUI app, the CLI) call into this crate and contain no
//! business logic of their own. This crate must never depend on GPUI.

mod attachment;
mod backup;
mod checklist;
mod db;
mod filter;
mod issue;
mod label;
mod order;
mod project;
mod view;

pub use attachment::{
    Attachment, AttachmentId, AttachmentKind, attachments_dir_for, format_size,
    normalize_attachment_name,
};
pub use backup::{
    AUTO_BACKUP_INTERVAL, Backup, BackupDir, BackupDirChange, BackupReason, Manifest, Restored,
    format_utc,
};
pub use checklist::{ChecklistItem, ChecklistItemId, checklist_progress, normalize_item_text};
pub use db::{Db, default_path};
pub use filter::IssueFilter;
pub use issue::{
    ISSUE_PREFIX, Issue, IssueChange, IssueEdit, IssueId, NewIssue, Status, normalize_description,
};
pub use label::{
    LABEL_SEPARATOR, Label, LabelColor, label_matches, normalize_label_name, renamed_label,
};
pub use order::Placement;
pub use project::{NewProject, Project, ProjectStatus, ProjectUpdate};
pub use view::View;
