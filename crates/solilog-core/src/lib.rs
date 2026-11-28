//! Solilog core: domain types, storage and every domain operation.
//!
//! Frontends (the GPUI app, the CLI) call into this crate and contain no
//! business logic of their own. This crate must never depend on GPUI.

mod activity;
mod attachment;
mod backup;
mod checklist;
mod cli_link;
mod context;
mod db;
mod decision;
mod dependency;
mod filter;
mod issue;
mod label;
mod link;
mod note;
mod order;
mod path_map;
mod project;
mod review;
mod revision;
mod search;
mod skill;
mod thinning;
mod view;

pub use activity::{Activity, ActivityId, ActivityKind, ActivityQuery, Actor};
pub use attachment::{
    Attachment, AttachmentId, AttachmentKind, attachments_dir_for, format_size,
    normalize_attachment_name,
};
pub use backup::{
    AUTO_BACKUP_INTERVAL, Backup, BackupDir, BackupDirChange, BackupReason, Manifest, Restored,
    format_utc,
};
pub use checklist::{
    ChecklistItem, ChecklistItemId, ChecklistProgress, checklist_progress, normalize_item_text,
};
pub use cli_link::{
    CLI_LINK_PATH, CliLinkState, NOT_A_LINK_EXIT, admin_script, app_bundle_binary,
    applescript_string, cli_link_state, install_cli_link,
};
pub use context::{IssueContext, IssueSummary, ProjectSummary};
pub use db::{Db, default_path};
pub use decision::{Decision, DecisionId, DecisionOwner, DecisionState, normalize_decision_text};
pub use filter::IssueFilter;
pub use issue::{
    CompactIssue, ISSUE_PREFIX, Issue, IssueChange, IssueEdit, IssueId, NewIssue, Status,
    issue_mentions, normalize_description, web_links,
};
pub use label::{
    LABEL_SEPARATOR, Label, LabelColor, label_matches, normalize_label_name, renamed_label,
};
pub use link::{Link, LinkId, LinkKind, NewLink, normalize_link};
pub use note::{Note, NoteId, NoteKind, normalize_note_text};
pub use order::Placement;
pub use path_map::{MappedPath, PathMapping, PathOwnerKind, normalize_mapped_path};
pub use project::{NewProject, Project, ProjectStatus, ProjectUpdate};
pub use review::{Check, Handoff, Receipt, Verdict, latest_receipt};
pub use revision::{Revision, RevisionField, RevisionId, RevisionReason, format_age};
pub use search::{SearchField, SearchHit, SearchMatch, SearchQuery};
pub use skill::{
    Agent, SKILL_NAME, SKILL_VERSION, SkillAction, SkillChange, SkillEnv, SkillLocation,
    SkillState, SkillStatus, install_skill, outdated_skills, skill_md, skill_status,
    uninstall_skill, update_outdated_skills,
};
pub use view::View;
