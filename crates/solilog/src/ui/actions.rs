//! Every GUI command, in one place.
//!
//! An action's name is `namespace::Name` (e.g. `issue::SetStatus`); that is
//! what keymap files refer to. The command palette shows it as
//! "Issue: Set Status" and uses the doc comment as its description.
//!
//! Namespaces listed in [`PALETTE_HIDDEN_NAMESPACES`] are low-level UI
//! plumbing (cursor movement, list navigation) and are bindable but not
//! shown in the palette.

/// Namespaces whose actions never appear in the command palette.
pub const PALETTE_HIDDEN_NAMESPACES: &[&str] = &[
    "archive",
    "command_palette",
    "issue_list",
    "picker",
    "sidebar",
    "text_input",
    "zed",
];

pub mod workspace {
    use gpui::{Action, actions};
    use schemars::JsonSchema;
    use serde::Deserialize;

    /// Open a web address in the default browser. Clicking a link in a
    /// description does this.
    ///
    /// Keymap example: `"g h": ["workspace::OpenUrl", { "url": "https://example.org" }]`
    #[derive(Clone, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = workspace)]
    #[serde(deny_unknown_fields)]
    pub struct OpenUrl {
        pub url: String,
    }

    /// Switch the UI theme: the given `name` (id or display name), else one
    /// picked from the list of built-in and user themes.
    ///
    /// Keymap example: `"g t": ["workspace::SetTheme", { "name": "mmx" }]`
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = workspace)]
    #[serde(deny_unknown_fields)]
    pub struct SetTheme {
        #[serde(default)]
        pub name: Option<String>,
    }

    actions!(
        workspace,
        [
            /// Quit Solilog.
            Quit,
            /// Open your keymap file in the default editor (it is created if missing).
            OpenKeymap,
            /// Reload the default and user keymaps.
            ReloadKeymap,
            /// Open the user themes folder in the file manager (it is created if missing).
            OpenThemesDirectory,
            /// Show or hide the sidebar.
            ToggleSidebar,
            /// Move focus to the sidebar (showing it if hidden).
            FocusSidebar,
            /// Show the archive: archived projects and issues, which can be
            /// deleted or brought back from there.
            OpenArchive,
            /// Show the database file and its attachments in the file manager.
            OpenDatabaseDirectory,
            /// Put the `solilog` command on the PATH (a link in
            /// /usr/local/bin to this app, which may ask for an administrator
            /// password) and install or update the skill that tells coding
            /// agents how to use it. macOS only.
            InstallCommandLineTool,
        ]
    );
}

/// Backups of the database and its attachments (see `solilog backup --help`).
pub mod backup {
    use std::path::PathBuf;

    use gpui::{Action, actions};
    use schemars::JsonSchema;
    use serde::Deserialize;

    actions!(
        backup,
        [
            /// Back up the database and its attachments now. Backups are
            /// also made automatically after changes, at most once an hour.
            Create,
            /// Open the backup folder in the file manager.
            OpenDirectory,
            /// Make backups go to the default folder, next to the database,
            /// moving the existing ones there.
            UseDefaultDirectory,
        ]
    );

    /// Choose the folder backups go to (e.g. a synced folder): the given
    /// `path`, else one chosen in a dialog. Existing backups are moved there.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = backup)]
    #[serde(deny_unknown_fields)]
    pub struct ChooseDirectory {
        #[serde(default)]
        pub path: Option<PathBuf>,
    }

    /// Replace all data with a backup's: the given `name` (a file in the
    /// backup folder), else one picked from the list. The current data is
    /// backed up first. Asks for confirmation unless `confirmed` is true.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = backup)]
    #[serde(deny_unknown_fields)]
    pub struct Restore {
        #[serde(default)]
        pub name: Option<String>,
        #[serde(default)]
        pub confirmed: bool,
    }
}

/// The archive's list.
pub mod archive {
    use gpui::actions;

    actions!(
        archive,
        [
            SelectNext,
            SelectPrevious,
            /// Show the menu of the highlighted archived item (delete…).
            /// Right-clicking an item does this.
            ShowContextMenu,
            /// Leave the archive, back to the issues shown before.
            Close,
        ]
    );
}

/// Sidebar navigation.
pub mod sidebar {
    use gpui::actions;

    actions!(
        sidebar,
        [
            SelectNext,
            SelectPrevious,
            /// Open the highlighted sidebar row.
            Open,
            /// Show or hide done projects in the sidebar.
            ToggleClosedProjects,
            /// Move the highlighted project or label above the previous one
            /// (projects within their section, labels among their siblings).
            MoveUp,
            /// Move the highlighted project or label below the next one.
            MoveDown,
            /// Show the menu of the highlighted row (open, rename, delete…).
            /// Right-clicking a row does this.
            ShowContextMenu,
        ]
    );
}

/// Managing projects. Where `name` is optional, leaving it out acts on the
/// project being shown.
pub mod project {
    use gpui::{Action, actions};
    use schemars::JsonSchema;
    use serde::Deserialize;
    use solilog_core::ProjectStatus;

    actions!(
        project,
        [
            /// Put the cursor in the description of the project being
            /// shown, in the header above its issues. It is saved as you
            /// type; Escape goes back to the list.
            EditDescription,
        ]
    );

    /// Create a project (active, without labels) and show it. Without a
    /// `name`, asks for one. The + next to the sidebar's Projects heading
    /// does this.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = project)]
    #[serde(deny_unknown_fields)]
    pub struct Create {
        #[serde(default)]
        pub name: Option<String>,
    }

    /// Rename a project. Without a `new_name`, asks for one.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = project)]
    #[serde(deny_unknown_fields)]
    pub struct Rename {
        #[serde(default)]
        pub name: Option<String>,
        #[serde(default)]
        pub new_name: Option<String>,
    }

    /// Set a project's status. Done and archived projects are hidden from
    /// pickers; archived ones are only listed in the archive. Without a
    /// `status`, opens the status picker.
    ///
    /// Keymap example: `"secondary-d": ["project::SetStatus", { "status": "done" }]`
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = project)]
    #[serde(deny_unknown_fields)]
    pub struct SetStatus {
        #[serde(default)]
        pub name: Option<String>,
        #[serde(default)]
        pub status: Option<ProjectStatus>,
    }

    /// Add a default label to a project, creating the label if needed:
    /// issues get it when they join the project (issues already in it are
    /// left as they are). Without a `label`, opens the label picker.
    ///
    /// Keymap example: `"secondary-l": ["project::AddLabel", { "label": "acme/reverb" }]`
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = project)]
    #[serde(deny_unknown_fields)]
    pub struct AddLabel {
        #[serde(default)]
        pub name: Option<String>,
        #[serde(default)]
        pub label: Option<String>,
    }

    /// Remove a default label from a project; issues keep it. Without a
    /// `label`, opens a picker of the project's default labels.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = project)]
    #[serde(deny_unknown_fields)]
    pub struct RemoveLabel {
        #[serde(default)]
        pub name: Option<String>,
        #[serde(default)]
        pub label: Option<String>,
    }

    /// Delete a project; its issues stay, with their labels. Asks for
    /// confirmation unless `confirmed` is true.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = project)]
    #[serde(deny_unknown_fields)]
    pub struct Delete {
        #[serde(default)]
        pub name: Option<String>,
        #[serde(default)]
        pub confirmed: bool,
    }

    /// Move a project right before or right after another one (give exactly
    /// one of `before` and `after`). Dragging a project in the sidebar does this.
    ///
    /// Keymap example: `"secondary-t": ["project::Move", { "name": "Triage", "before": "Website" }]`
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = project)]
    #[serde(deny_unknown_fields)]
    pub struct Move {
        pub name: String,
        #[serde(default)]
        pub before: Option<String>,
        #[serde(default)]
        pub after: Option<String>,
    }
}

/// Managing labels. Where `name` is optional, leaving it out acts on the
/// label being shown.
pub mod label {
    use gpui::Action;
    use schemars::JsonSchema;
    use serde::Deserialize;

    /// Rename a label, with its sublabels; a new path moves it
    /// (`acme` → `globex/acme`). Without a `new_name`, asks for one.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = label)]
    #[serde(deny_unknown_fields)]
    pub struct Rename {
        #[serde(default)]
        pub name: Option<String>,
        #[serde(default)]
        pub new_name: Option<String>,
    }

    /// Set the color of a label's chips; sublabels without a color of their
    /// own are shown in it. Without a `color` (and without `reset`), opens
    /// the color picker; `reset` goes back to the default color.
    ///
    /// Keymap example: `"secondary-b": ["label::SetColor", { "color": "blue" }]`
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = label)]
    #[serde(deny_unknown_fields)]
    pub struct SetColor {
        #[serde(default)]
        pub name: Option<String>,
        #[serde(default)]
        pub color: Option<solilog_core::LabelColor>,
        #[serde(default)]
        pub reset: bool,
    }

    /// Collapse or expand a label's sublabels in the sidebar. Without a
    /// `name`, acts on the highlighted label when the sidebar has focus,
    /// else on the label being shown.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = label)]
    #[serde(deny_unknown_fields)]
    pub struct ToggleCollapsed {
        #[serde(default)]
        pub name: Option<String>,
    }

    /// Delete a label and its sublabels, removing them from every issue.
    /// Asks for confirmation unless `confirmed` is true.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = label)]
    #[serde(deny_unknown_fields)]
    pub struct Delete {
        #[serde(default)]
        pub name: Option<String>,
        #[serde(default)]
        pub confirmed: bool,
    }

    /// Move a label, with its sublabels, right before or right after a
    /// sibling (give exactly one of `before` and `after`). Dragging a label
    /// in the sidebar does this.
    ///
    /// Keymap example: `"secondary-t": ["label::Move", { "name": "acme/bug", "before": "acme/feature" }]`
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = label)]
    #[serde(deny_unknown_fields)]
    pub struct Move {
        pub name: String,
        #[serde(default)]
        pub before: Option<String>,
        #[serde(default)]
        pub after: Option<String>,
    }
}

pub mod command_palette {
    use gpui::actions;

    actions!(
        command_palette,
        [
            /// Show or hide the command palette.
            Toggle,
        ]
    );
}

pub mod issue {
    use gpui::{Action, actions};
    use schemars::JsonSchema;
    use serde::Deserialize;
    use solilog_core::{IssueId, Status};

    actions!(
        issue,
        [
            /// Focus the new-issue input.
            New,
            /// Archive the selected issues: they keep everything but are left
            /// out of listings, views and counts (see `workspace::OpenArchive`).
            Archive,
            /// Bring the selected archived issue back from the archive.
            Unarchive,
            /// Copy the ids of the selected issues (`SOL-12`). In a
            /// description, an issue id links to that issue.
            CopyId,
            /// Show the earlier text of the issue (titles, descriptions and
            /// checklist items, deleted items too) and restore one. The
            /// current text is saved first, so a restore can be undone.
            ShowHistory,
            /// Mark the selected issues' non-user changes as reviewed. Any
            /// later non-user write marks them unreviewed again.
            MarkReviewed,
            /// Approve the selected issues that are in review: they become
            /// done, with an approval in their notes, and are marked reviewed.
            Approve,
        ]
    );

    /// Send issues in review back to Todo, with a note saying what to
    /// change: the given `ids` (`SOL-12`), else the selected issues in
    /// review. Asks for the note unless `message` is given.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = issue)]
    #[serde(deny_unknown_fields)]
    pub struct SendBack {
        #[serde(default)]
        #[schemars(with = "Vec<String>")]
        pub ids: Vec<IssueId>,
        #[serde(default)]
        pub message: Option<String>,
    }

    /// Mark every unreviewed issue in the Unreviewed view as reviewed. Any
    /// later non-user write marks them unreviewed again. Asks for confirmation
    /// unless `confirmed` is true.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = issue)]
    #[serde(deny_unknown_fields)]
    pub struct MarkAllReviewed {
        #[serde(default)]
        pub confirmed: bool,
    }

    /// Delete issues for good: the given `id` (`SOL-12`) or `ids`, else the
    /// selected ones. Asks for confirmation unless `confirmed` is true.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = issue)]
    #[serde(deny_unknown_fields)]
    pub struct Delete {
        #[serde(default)]
        #[schemars(with = "Option<String>")]
        pub id: Option<IssueId>,
        #[serde(default)]
        #[schemars(with = "Vec<String>")]
        pub ids: Vec<IssueId>,
        #[serde(default)]
        pub confirmed: bool,
    }

    /// Edit an issue's fields: the given `id` (`SOL-12`), else the
    /// highlighted issue. Sets the `title` and/or `description` given (an
    /// empty description removes it); with neither, opens the issue, like
    /// `issue::Open`.
    ///
    /// Keymap example: `"e": ["issue::Edit", { "title": "Triage me" }]`
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = issue)]
    #[serde(deny_unknown_fields)]
    pub struct Edit {
        #[serde(default)]
        #[schemars(with = "Option<String>")]
        pub id: Option<IssueId>,
        #[serde(default)]
        pub title: Option<String>,
        #[serde(default)]
        pub description: Option<String>,
    }

    /// Open an issue in place of the list, to read it and edit its title and
    /// description: the given `id` (`SOL-12`), else the highlighted issue.
    /// Double-clicking an issue does this.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = issue)]
    #[serde(deny_unknown_fields)]
    pub struct Open {
        #[serde(default)]
        #[schemars(with = "Option<String>")]
        pub id: Option<IssueId>,
    }

    /// Set the status of the selected issues. Without a `status`, opens the status picker.
    ///
    /// Keymap example: `"d": ["issue::SetStatus", { "status": "done" }]`
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = issue)]
    #[serde(deny_unknown_fields)]
    pub struct SetStatus {
        #[serde(default)]
        pub status: Option<Status>,
    }

    /// Move an issue right before or right after another issue with the
    /// same status (give exactly one of `before` and `after`; ids like
    /// `SOL-12`). The list keeps this order within each status group.
    /// Dragging an issue does this.
    #[derive(Clone, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = issue)]
    #[serde(deny_unknown_fields)]
    pub struct Move {
        #[schemars(with = "String")]
        pub id: IssueId,
        #[serde(default)]
        #[schemars(with = "Option<String>")]
        pub before: Option<IssueId>,
        #[serde(default)]
        #[schemars(with = "Option<String>")]
        pub after: Option<IssueId>,
    }

    /// Add a label to the selected issues, creating it if needed. Without a
    /// `label`, opens the label picker.
    ///
    /// Keymap example: `"b": ["issue::AddLabel", { "label": "bug" }]`
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = issue)]
    #[serde(deny_unknown_fields)]
    pub struct AddLabel {
        #[serde(default)]
        pub label: Option<String>,
    }

    /// Remove a label from the selected issues. Without a `label`, opens a
    /// picker of their labels.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = issue)]
    #[serde(deny_unknown_fields)]
    pub struct RemoveLabel {
        #[serde(default)]
        pub label: Option<String>,
    }

    /// Add the selected issues to a project, giving them the project's default
    /// labels. Without a `project`, opens the project picker (which can also
    /// create one).
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = issue)]
    #[serde(deny_unknown_fields)]
    pub struct AddToProject {
        #[serde(default)]
        pub project: Option<String>,
    }

    /// Remove the selected issues from a project; they keep their labels.
    /// Without a `project`, opens a picker of their projects.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = issue)]
    #[serde(deny_unknown_fields)]
    pub struct RemoveFromProject {
        #[serde(default)]
        pub project: Option<String>,
    }

    /// Make the selected issues depend on another issue (`on`, like
    /// `SOL-7`): work to finish first. An issue is waiting while an issue it
    /// depends on is open, ready once none is; nothing changes its status by
    /// itself. Without `on`, opens an issue picker.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = issue)]
    #[serde(deny_unknown_fields)]
    pub struct AddDependency {
        #[serde(default)]
        #[schemars(with = "Option<String>")]
        pub on: Option<IssueId>,
    }

    /// Make the selected issues no longer depend on an issue (`on`, like
    /// `SOL-7`). Without `on`, opens a picker of their dependencies.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = issue)]
    #[serde(deny_unknown_fields)]
    pub struct RemoveDependency {
        #[serde(default)]
        #[schemars(with = "Option<String>")]
        pub on: Option<IssueId>,
    }
}

/// The open issue (see `issue::Open`). Issue commands act on it.
pub mod issue_detail {
    use gpui::actions;

    actions!(
        issue_detail,
        [
            /// Back to the list. Edits are saved as you type, and when you leave.
            Close,
            /// Put the cursor in the title.
            EditTitle,
            /// Put the cursor in the description.
            EditDescription,
            /// Leave the title or description field (saving it), staying on the issue.
            StopEditing,
        ]
    );
}

/// The checklist of an issue: one-line items, each checked or not. In the
/// open issue, item commands act on the item being edited unless given an `id`.
pub mod checklist {
    use gpui::{Action, actions};
    use schemars::JsonSchema;
    use serde::Deserialize;
    use solilog_core::ChecklistItemId;

    actions!(
        checklist,
        [
            /// Move the item being edited up the checklist.
            MoveItemUp,
            /// Move the item being edited down the checklist.
            MoveItemDown,
            /// Edit the next checklist item (or the new-item field).
            FocusNext,
            /// Edit the previous checklist item.
            FocusPrevious,
        ]
    );

    /// Add an item to the checklist of the open (else the highlighted)
    /// issue. With a `text`, adds it; without, opens the issue and puts the
    /// cursor in its new-item field.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = checklist)]
    #[serde(deny_unknown_fields)]
    pub struct AddItem {
        #[serde(default)]
        pub text: Option<String>,
    }

    /// Check or uncheck a checklist item: the given `id`, else the one
    /// being edited.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = checklist)]
    #[serde(deny_unknown_fields)]
    pub struct ToggleItem {
        #[serde(default)]
        #[schemars(with = "Option<i64>")]
        pub id: Option<ChecklistItemId>,
    }

    /// Move a checklist item right before or right after another item of
    /// the same checklist (give exactly one of `before` and `after`).
    /// Dragging an item by its handle does this.
    #[derive(Clone, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = checklist)]
    #[serde(deny_unknown_fields)]
    pub struct MoveItem {
        #[schemars(with = "i64")]
        pub id: ChecklistItemId,
        #[serde(default)]
        #[schemars(with = "Option<i64>")]
        pub before: Option<ChecklistItemId>,
        #[serde(default)]
        #[schemars(with = "Option<i64>")]
        pub after: Option<ChecklistItemId>,
    }

    /// Delete a checklist item: the given `id`, else the one being edited.
    /// Emptying an item's text and leaving it also deletes it.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = checklist)]
    #[serde(deny_unknown_fields)]
    pub struct DeleteItem {
        #[serde(default)]
        #[schemars(with = "Option<i64>")]
        pub id: Option<ChecklistItemId>,
    }
}

/// Notes: an issue's worklog, below its checklist. Timestamped entries,
/// each by the user or an agent, never edited: what was tried, decided, or
/// should be done next. Agents read them with `solilog issue context`.
pub mod note {
    use gpui::{Action, actions};
    use schemars::JsonSchema;
    use serde::Deserialize;
    use solilog_core::NoteId;

    actions!(
        note,
        [
            /// Add the note being written to the open issue's notes.
            AddDraft,
        ]
    );

    /// Add a note to the open (else the highlighted) issue. With a `text`,
    /// adds it; without, opens the issue and puts the cursor in its
    /// new-note field.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = note)]
    #[serde(deny_unknown_fields)]
    pub struct Add {
        #[serde(default)]
        pub text: Option<String>,
    }

    /// Delete a note of the open issue for good: the given `id`, else one
    /// picked from its notes. Asks for confirmation unless `confirmed` is true.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = note)]
    #[serde(deny_unknown_fields)]
    pub struct Delete {
        #[serde(default)]
        #[schemars(with = "Option<i64>")]
        pub id: Option<NoteId>,
        #[serde(default)]
        pub confirmed: bool,
    }
}

/// Files attached to the open issue. They are copied into the attachments
/// directory next to the database, one folder per issue.
pub mod attachment {
    use std::path::PathBuf;

    use gpui::{Action, actions};
    use schemars::JsonSchema;
    use serde::Deserialize;
    use solilog_core::AttachmentId;

    actions!(
        attachment,
        [
            /// Attach the files (copied in the file manager) or the image
            /// (e.g. a screenshot) on the clipboard to the open issue.
            Paste,
        ]
    );

    /// Attach files to the open issue: the given `paths`, else the files
    /// chosen in a file dialog. Dropping files on the open issue does this.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = attachment)]
    #[serde(deny_unknown_fields)]
    pub struct Add {
        #[serde(default)]
        pub paths: Vec<PathBuf>,
    }

    /// Show an image attachment in the app: the given `id`, else the open
    /// issue's first image. The preview goes through the issue's other
    /// images (`image_preview::Next`, `Previous`). Clicking an image
    /// attachment does this.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = attachment)]
    #[serde(deny_unknown_fields)]
    pub struct Preview {
        #[serde(default)]
        #[schemars(with = "Option<i64>")]
        pub id: Option<AttachmentId>,
    }

    /// Show an attachment's file in the file manager (the Finder on macOS): the
    /// given `id`, else the image being previewed, else one picked among
    /// the open issue's attachments. Clicking a file attachment does this.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = attachment)]
    #[serde(deny_unknown_fields)]
    pub struct Reveal {
        #[serde(default)]
        #[schemars(with = "Option<i64>")]
        pub id: Option<AttachmentId>,
    }

    /// Delete an attachment and its file: the given `id`, else one picked
    /// among the open issue's attachments. Asks for confirmation unless
    /// `confirmed` is true.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = attachment)]
    #[serde(deny_unknown_fields)]
    pub struct Delete {
        #[serde(default)]
        #[schemars(with = "Option<i64>")]
        pub id: Option<AttachmentId>,
        #[serde(default)]
        pub confirmed: bool,
    }
}

/// The image preview (see `attachment::Preview`).
pub mod image_preview {
    use gpui::actions;

    actions!(
        image_preview,
        [
            /// Show the issue's next image (after the last, the first).
            Next,
            /// Show the issue's previous image.
            Previous,
            /// Close the preview, back to the issue.
            Close,
        ]
    );
}

pub mod view {
    use gpui::{Action, actions};
    use schemars::JsonSchema;
    use serde::Deserialize;

    actions!(
        view,
        [
            /// Show all issues (leave the current view).
            ShowAll,
            /// Show issues with unreviewed non-user changes.
            ShowUnreviewed,
            /// Save the current filter as a new view, or replace an existing one.
            SaveAs,
            /// Discard unsaved changes to the filter of the current view,
            /// project or label.
            Revert,
        ]
    );

    /// Show the issues of a project. Without a `name`, opens the project picker.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = view)]
    #[serde(deny_unknown_fields)]
    pub struct OpenProject {
        #[serde(default)]
        pub name: Option<String>,
    }

    /// Show the issues with a label (or one of its sublabels). Without a
    /// `name`, opens the label picker.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = view)]
    #[serde(deny_unknown_fields)]
    pub struct OpenLabel {
        #[serde(default)]
        pub name: Option<String>,
    }

    /// Save the current filter. With a `name`, saves it as that view (created
    /// or replaced); without one, updates the view being shown, or asks for a
    /// name.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = view)]
    #[serde(deny_unknown_fields)]
    pub struct Save {
        #[serde(default)]
        pub name: Option<String>,
    }

    /// Rename a view; without a `name`, the view being shown. Without a
    /// `new_name`, asks for one.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = view)]
    #[serde(deny_unknown_fields)]
    pub struct Rename {
        #[serde(default)]
        pub name: Option<String>,
        #[serde(default)]
        pub new_name: Option<String>,
    }

    /// Delete a view; without a `name`, the view being shown (then all
    /// issues are shown). Asks for confirmation unless `confirmed` is true.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = view)]
    #[serde(deny_unknown_fields)]
    pub struct Delete {
        #[serde(default)]
        pub name: Option<String>,
        #[serde(default)]
        pub confirmed: bool,
    }

    /// Show the issues of a saved view. Without a `name`, opens the view picker.
    /// Views are created with `solilog view save`.
    ///
    /// Keymap example: `"g b": ["view::Open", { "name": "Acme bugs" }]`
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = view)]
    #[serde(deny_unknown_fields)]
    pub struct Open {
        #[serde(default)]
        pub name: Option<String>,
    }
}

/// Editing the filter of the issue list. Without an argument, each opens a
/// picker.
pub mod filter {
    use gpui::{Action, actions};
    use schemars::JsonSchema;
    use serde::Deserialize;
    use solilog_core::Status;

    actions!(
        filter,
        [
            /// Show only open issues, or all of them again.
            ToggleOpenOnly,
            /// Show only issues in no project, or drop that criterion.
            ToggleNoProject,
            /// Show only issues that can be started (open, and every issue
            /// they depend on is done or cancelled), or drop that criterion.
            ToggleReady,
            /// Show only open issues that depend on an issue still open, or
            /// drop that criterion.
            ToggleWaiting,
            /// Show only issues with unreviewed non-user changes, or drop
            /// that criterion.
            ToggleUnreviewed,
            /// Remove every filter criterion.
            Clear,
            /// Show issues of every status: drops the status and "open only"
            /// criteria, keeping the rest of the filter.
            AllStatuses,
        ]
    );

    /// Require a label (or one of its sublabels), or drop that requirement.
    /// All required labels must match.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = filter)]
    #[serde(deny_unknown_fields)]
    pub struct ToggleLabel {
        #[serde(default)]
        pub label: Option<String>,
    }

    /// Hide issues with a label (or one of its sublabels), or show them again.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = filter)]
    #[serde(deny_unknown_fields)]
    pub struct ExcludeLabel {
        #[serde(default)]
        pub label: Option<String>,
    }

    /// Add or remove a project; issues in any of the chosen projects match.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = filter)]
    #[serde(deny_unknown_fields)]
    pub struct ToggleProject {
        #[serde(default)]
        pub project: Option<String>,
    }

    /// Add or remove a status; issues with any of the chosen statuses match.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = filter)]
    #[serde(deny_unknown_fields)]
    pub struct ToggleStatus {
        #[serde(default)]
        pub status: Option<Status>,
    }
}

/// The issue list's status sections.
pub mod status {
    use gpui::{Action, actions};
    use schemars::JsonSchema;
    use serde::Deserialize;
    use solilog_core::Status;

    /// Collapse or expand a status's section in the issue list: a collapsed
    /// section only shows its header and issue count. Without a `status`,
    /// asks which section, starting from the highlighted issue's. Clicking
    /// a section header does this.
    ///
    /// Keymap example: `"z d": ["status::ToggleCollapsed", { "status": "done" }]`
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = status)]
    #[serde(deny_unknown_fields)]
    pub struct ToggleCollapsed {
        #[serde(default)]
        pub status: Option<Status>,
    }

    actions!(
        status,
        [
            /// Expand every collapsed section of the issue list.
            ExpandAll,
        ]
    );
}

pub mod issue_list {
    use gpui::actions;

    actions!(
        issue_list,
        [
            /// Move focus to the issue list.
            Focus,
            SelectNext,
            SelectPrevious,
            SelectFirst,
            SelectLast,
            /// Show the menu of the selected issues (set status, labels,
            /// archive…). Right-clicking an issue does this.
            ShowContextMenu,
            /// Add the highlighted issue to the selection, or take it out.
            /// Issue commands act on every selected issue, or on the
            /// highlighted one when none is selected. Cmd-clicking an issue
            /// (ctrl-clicking off macOS) does this.
            ToggleSelected,
            /// Select every issue in the list.
            SelectAll,
            /// Deselect every issue.
            ClearSelection,
            /// Move the highlight down, selecting the issues between where
            /// the move started and the highlight. Shift-clicking an issue
            /// selects up to it the same way.
            ExtendSelectionDown,
            /// Move the highlight up, selecting like `ExtendSelectionDown`.
            ExtendSelectionUp,
            /// Move the highlighted issue above the previous one with the
            /// same status (the order is saved).
            MoveUp,
            /// Move the highlighted issue below the next one with the same
            /// status.
            MoveDown,
        ]
    );
}

/// Shared by every list-with-a-highlight popup (command palette, status picker).
pub mod picker {
    use gpui::actions;

    actions!(picker, [SelectNext, SelectPrevious, Confirm, Dismiss]);
}

/// "issue::SetStatus" -> "Issue: Set Status".
pub fn humanize_action_name(name: &str) -> String {
    let (ns, action) = name.split_once("::").unwrap_or(("", name));
    let ns = ns.split('_').map(capitalize).collect::<Vec<_>>().join(" ");
    let mut words = String::new();
    for (i, c) in action.char_indices() {
        if i > 0 && c.is_uppercase() {
            words.push(' ');
        }
        words.push(c);
    }
    if ns.is_empty() {
        words
    } else {
        format!("{ns}: {words}")
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

pub fn is_palette_visible(name: &str) -> bool {
    let ns = name.split_once("::").map_or("", |(ns, _)| ns);
    !PALETTE_HIDDEN_NAMESPACES.contains(&ns)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn humanize() {
        assert_eq!(
            humanize_action_name("issue::SetStatus"),
            "Issue: Set Status"
        );
        assert_eq!(
            humanize_action_name("command_palette::Toggle"),
            "Command Palette: Toggle"
        );
    }
}
