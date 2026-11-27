//! Edits made elsewhere while the user types. When the CLI (an agent)
//! changes the open issue's title or description while that field has
//! unsaved edits, the field keeps the user's text and the next save
//! replaces the other edit. The core keeps the replaced text as a revision
//! (text someone else wrote always gets one); this tells the user, in a
//! panel above the description, until they open the history or close the
//! issue.

use gpui::{AnyElement, FocusHandle, FontWeight, IntoElement, SharedString, div, prelude::*, px};
use solilog_core::{Actor, Issue};

use super::IssueDetail;
use super::review::button;
use crate::ui::actions::issue::ShowHistory;
use crate::ui::theme;

/// The issue text fields edited in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TextField {
    Title,
    Description,
}

impl TextField {
    fn label(self) -> &'static str {
        match self {
            TextField::Title => "title",
            TextField::Description => "description",
        }
    }

    fn text(self, issue: &Issue) -> &str {
        match self {
            TextField::Title => &issue.title,
            TextField::Description => &issue.description,
        }
    }

    fn author(self, issue: &Issue) -> &Actor {
        match self {
            TextField::Title => &issue.title_by,
            TextField::Description => &issue.description_by,
        }
    }
}

/// An edit made elsewhere to a field with unsaved edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct EditElsewhere {
    pub field: TextField,
    pub by: Actor,
}

/// What the detail knows about edits made elsewhere.
#[derive(Default)]
pub(super) struct Overwrites {
    /// Fields changed elsewhere while they had unsaved edits: the next save
    /// of the field replaces that edit.
    pending: Vec<EditElsewhere>,
    /// Edits made elsewhere that a save replaced, shown to the user.
    shown: Vec<EditElsewhere>,
}

impl Overwrites {
    /// After a reload: `saved` is the issue as the detail last had it,
    /// `issue` as it now is, `unsaved` whether `field` has unsaved edits.
    pub fn refresh(&mut self, field: TextField, unsaved: bool, saved: &Issue, issue: &Issue) {
        if !unsaved {
            self.pending.retain(|e| e.field != field);
        } else if field.text(saved) != field.text(issue) {
            self.pending.retain(|e| e.field != field);
            self.pending.push(EditElsewhere {
                field,
                by: field.author(issue).clone(),
            });
        }
    }

    /// After `field` was saved: an edit made elsewhere that it replaced
    /// is shown.
    pub fn saved(&mut self, field: TextField) {
        if let Some(ix) = self.pending.iter().position(|e| e.field == field) {
            let edit = self.pending.remove(ix);
            self.shown.retain(|e| e.field != field);
            self.shown.push(edit);
        }
    }

    /// The user saw them (opened the history).
    pub fn dismiss(&mut self) {
        self.shown.clear();
    }

    #[cfg(test)]
    fn shown(&self) -> &[EditElsewhere] {
        &self.shown
    }
}

/// One line per replaced edit, e.g. "While you were typing, claude:docs
/// changed the description. Your text replaced theirs, which is in the
/// history."
fn message(edit: &EditElsewhere) -> String {
    let who = if edit.by.is_user() {
        "you changed".to_owned()
    } else {
        format!("{} changed", edit.by)
    };
    let field = edit.field.label();
    format!(
        "While you were typing, {who} the {field} elsewhere. Your text replaced that edit, \
         which is kept in the history."
    )
}

/// Above the description: the edits made elsewhere that the user's text
/// replaced, with a button to the history (to restore one).
pub(super) fn render_overwrites(
    detail: &IssueDetail,
    focus: &FocusHandle,
    t: &theme::Theme,
) -> Option<AnyElement> {
    let shown = &detail.overwrites.shown;
    if shown.is_empty() {
        return None;
    }
    let header = div()
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.colors.error)
                .child("Edited elsewhere"),
        )
        .child(div().flex_1())
        .child(button(
            "overwrite-history",
            "Show history",
            false,
            focus,
            t,
            Box::new(ShowHistory),
        ));
    Some(
        div()
            .relative()
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2()
            .rounded(t.radius_sm())
            .bg(t.colors.surface)
            .border_l_2()
            .border_color(t.colors.error)
            .child(header)
            .children(shown.iter().map(|edit| {
                div()
                    .text_size(px(12.))
                    .text_color(t.colors.text)
                    .child(SharedString::from(message(edit)))
            }))
            .children(t.raised_bevel_thin())
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use solilog_core::NewIssue;

    fn issue(title: &str, by: &Actor) -> Issue {
        let db = solilog_core::Db::open_in_memory()
            .unwrap()
            .with_actor(by.clone());
        db.create_issue(&NewIssue::new(title)).unwrap()
    }

    #[test]
    fn edits_elsewhere_are_shown_once_saved_over() {
        let agent = Actor::parse("claude:test").unwrap();
        let saved = issue("old", &Actor::user());
        let changed = issue("agent's", &agent);
        let mut overwrites = Overwrites::default();

        // No unsaved edits: the field shows the new text, nothing to tell.
        overwrites.refresh(TextField::Title, false, &saved, &changed);
        overwrites.saved(TextField::Title);
        assert!(overwrites.shown().is_empty());

        // Unsaved edits: saving replaces the agent's text, which is shown.
        overwrites.refresh(TextField::Title, true, &saved, &changed);
        overwrites.saved(TextField::Description);
        assert!(overwrites.shown().is_empty(), "another field was saved");
        overwrites.saved(TextField::Title);
        assert_eq!(
            overwrites.shown(),
            [EditElsewhere {
                field: TextField::Title,
                by: agent.clone()
            }]
        );
        // Shown once: later saves of the field add nothing.
        overwrites.saved(TextField::Title);
        assert_eq!(overwrites.shown().len(), 1);
        overwrites.dismiss();
        assert!(overwrites.shown().is_empty());

        // The user's edits were dropped (or now match) before a save: no
        // overwrite.
        overwrites.refresh(TextField::Title, true, &saved, &changed);
        overwrites.refresh(TextField::Title, false, &changed, &changed);
        overwrites.saved(TextField::Title);
        assert!(overwrites.shown().is_empty());

        // Nothing changed elsewhere: unsaved edits are just unsaved.
        overwrites.refresh(TextField::Title, true, &saved, &saved);
        overwrites.saved(TextField::Title);
        assert!(overwrites.shown().is_empty());
    }

    #[test]
    fn messages() {
        let agent = EditElsewhere {
            field: TextField::Description,
            by: Actor::parse("claude:docs").unwrap(),
        };
        assert!(
            message(&agent)
                .starts_with("While you were typing, claude:docs changed the description")
        );
        let user = EditElsewhere {
            field: TextField::Title,
            by: Actor::user(),
        };
        assert!(
            message(&user).starts_with("While you were typing, you changed the title elsewhere")
        );
    }
}
