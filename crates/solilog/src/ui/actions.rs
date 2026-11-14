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
