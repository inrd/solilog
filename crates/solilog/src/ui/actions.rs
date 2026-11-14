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
    "command_palette",
    "issue_list",
    "picker",
    "sidebar",
    "text_input",
    "zed",
];

pub mod workspace {
    use gpui::actions;

    actions!(
        workspace,
        [
            /// Quit Solilog.
            Quit,
            /// Open your keymap file in the default editor (it is created if missing).
            OpenKeymap,
            /// Reload the default and user keymaps.
            ReloadKeymap,
            /// Show or hide the sidebar.
            ToggleSidebar,
            /// Move focus to the sidebar (showing it if hidden).
            FocusSidebar,
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
            /// Show or hide done and archived projects in the sidebar.
            ToggleClosedProjects,
        ]
    );
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
    use solilog_core::Status;

    actions!(
        issue,
        [
            /// Focus the new-issue input.
            New,
        ]
    );

    /// Set the status of the selected issue. Without a `status`, opens the status picker.
    ///
    /// Keymap example: `"d": ["issue::SetStatus", { "status": "done" }]`
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = issue)]
    #[serde(deny_unknown_fields)]
    pub struct SetStatus {
        #[serde(default)]
        pub status: Option<Status>,
    }

    /// Add a label to the selected issue, creating it if needed. Without a
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

    /// Remove a label from the selected issue. Without a `label`, opens a
    /// picker of the issue's labels.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = issue)]
    #[serde(deny_unknown_fields)]
    pub struct RemoveLabel {
        #[serde(default)]
        pub label: Option<String>,
    }

    /// Add the selected issue to a project, giving it the project's default
    /// labels. Without a `project`, opens the project picker (which can also
    /// create one).
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = issue)]
    #[serde(deny_unknown_fields)]
    pub struct AddToProject {
        #[serde(default)]
        pub project: Option<String>,
    }

    /// Remove the selected issue from a project; it keeps its labels. Without
    /// a `project`, opens a picker of the issue's projects.
    #[derive(Clone, Default, PartialEq, Debug, Deserialize, JsonSchema, Action)]
    #[action(namespace = issue)]
    #[serde(deny_unknown_fields)]
    pub struct RemoveFromProject {
        #[serde(default)]
        pub project: Option<String>,
    }
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
            /// Remove every filter criterion.
            Clear,
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
