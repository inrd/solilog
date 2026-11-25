//! The notes of the open issue, below its checklist: its worklog, oldest
//! first, each with its author and age, then a field to write a new one.
//!
//! Key context: `NoteDraft` around the new-note field. Notes are never
//! edited. The draft is added with `note::AddDraft` (cmd/ctrl-Enter), and
//! when the issue closes, so nothing typed is lost; not when the field just
//! loses focus, since a note can't be changed once added.

use gpui::{
    Action, ClickEvent, Context, Entity, FocusHandle, Focusable, FontWeight, IntoElement,
    ScrollHandle, Window, div, prelude::*, px,
};
use solilog_core::{Note, NoteId, NoteKind, Verdict, format_age};

use super::super::Workspace;
use super::IssueDetail;
use super::review::receipt_details;
use crate::ui::actions::note::{Add, AddDraft, Delete};
use crate::ui::picker::PickerItem;
use crate::ui::text_input::TextInput;
use crate::ui::theme;

pub const DRAFT_CONTEXT: &str = "NoteDraft";

/// How much of a note pickers show.
const PREVIEW_CHARS: usize = 60;

pub(super) struct Notes {
    /// The issue's notes as last read, oldest first.
    notes: Vec<Note>,
    draft: Entity<TextInput>,
}

impl Notes {
    pub(super) fn new(notes: Vec<Note>, scroll: ScrollHandle, cx: &mut Context<Workspace>) -> Self {
        let draft = cx.new(|cx| {
            TextInput::new(
                "Add a note: what you tried, decided, or what comes next…",
                cx,
            )
            .multiline()
            .autoscroll(scroll)
        });
        Self { notes, draft }
    }

    pub(super) fn set_notes(&mut self, notes: Vec<Note>) {
        self.notes = notes;
    }

    /// The issue's notes as last read, oldest first.
    pub(super) fn all(&self) -> &[Note] {
        &self.notes
    }

    fn draft_focus(&self, cx: &gpui::App) -> FocusHandle {
        self.draft.focus_handle(cx)
    }
}

/// A note on one line, shortened.
fn preview(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= PREVIEW_CHARS {
        return line;
    }
    let cut: String = line.chars().take(PREVIEW_CHARS - 1).collect();
    format!("{}…", cut.trim_end())
}

impl Workspace {
    /// Adds the note being written to the open issue, if any. Returns
    /// whether one was added.
    pub(super) fn add_drafted_note(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(detail) = &self.detail else {
            return false;
        };
        let text = detail.notes.draft.read(cx).text().to_owned();
        if text.trim().is_empty() {
            return false;
        }
        let id = detail.issue().id;
        if self.report(self.db.add_note(id, &text), cx).is_none() {
            return false;
        }
        if let Some(detail) = &self.detail {
            detail.notes.draft.update(cx, |input, cx| input.clear(cx));
        }
        self.reload(cx);
        true
    }

    // ---- actions ------------------------------------------------------------

    pub(in crate::ui::workspace) fn on_add_note(
        &mut self,
        action: &Add,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(issue) = self.selected_issue(cx) else {
            return;
        };
        if let Some(text) = &action.text {
            if self.report(self.db.add_note(issue.id, text), cx).is_some() {
                self.reload(cx);
            }
            return;
        }
        if self.detail.as_ref().map(|d| d.issue().id) != Some(issue.id) {
            self.close_modal(window, cx);
            self.open_issue(issue.id, window, cx);
        }
        if let Some(detail) = &self.detail {
            let input = detail.notes.draft.clone();
            window.focus(&detail.notes.draft_focus(cx));
            input.update(cx, |input, cx| input.move_to_end(cx));
        }
    }

    fn on_add_draft_note(&mut self, _: &AddDraft, _: &mut Window, cx: &mut Context<Self>) {
        self.add_drafted_note(cx);
    }

    pub(super) fn on_delete_note(
        &mut self,
        action: &Delete,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(detail) = &self.detail else {
            return;
        };
        let notes = &detail.notes.notes;
        let Some(id) = action.id else {
            if notes.is_empty() {
                self.error = Some(format!("{} has no notes", detail.issue().id));
                cx.notify();
                return;
            }
            let entries = notes
                .iter()
                .rev()
                .map(|n| {
                    let item = PickerItem::new(preview(&n.text)).hint(Some(format!(
                        "{}, {}",
                        n.author,
                        format_age(n.created_at)
                    )));
                    let delete = Delete {
                        id: Some(n.id),
                        confirmed: false,
                    };
                    (item, Box::new(delete) as Box<dyn Action>)
                })
                .collect();
            return self.open_action_picker("Delete note…", entries, 0, window, cx);
        };
        if !action.confirmed {
            let Some(note) = self.report(self.db.get_note(id), cx) else {
                return;
            };
            let item =
                PickerItem::new(format!("Delete “{}”", preview(&note.text))).hint(Some("for good"));
            let delete = Delete {
                id: Some(id),
                confirmed: true,
            };
            return self.open_action_picker(
                "Delete this note?",
                vec![(item, Box::new(delete))],
                0,
                window,
                cx,
            );
        }
        if self.report(self.db.delete_note(id), cx).is_some() {
            self.reload(cx);
        }
    }

    // ---- rendering ------------------------------------------------------------

    pub(super) fn render_notes(
        &self,
        detail: &IssueDetail,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = theme::current(cx);
        let pane_focus = detail.focus.clone();
        let notes = &detail.notes.notes;
        let header = div()
            .flex()
            .items_center()
            .gap_3()
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Notes"))
            .when(!notes.is_empty(), |s| {
                s.child(
                    div()
                        .text_size(px(12.))
                        .text_color(t.colors.text_muted)
                        .child(notes.len().to_string()),
                )
            });

        let rows = notes.iter().map(|note| {
            let id: NoteId = note.id;
            let delete = div()
                .id(("note-delete", id.0 as u64))
                .flex_none()
                .px_1()
                .rounded(t.radius_sm())
                .cursor_pointer()
                .invisible()
                .group_hover("note", |s| s.visible())
                .text_color(t.colors.text_muted)
                .hover(|s| s.bg(t.colors.highlight).text_color(t.colors.highlight_text))
                .child("×")
                .on_click({
                    let focus = pane_focus.clone();
                    move |_: &ClickEvent, window, cx| {
                        window.focus(&focus);
                        let action = Delete {
                            id: Some(id),
                            confirmed: false,
                        };
                        window.dispatch_action(action.boxed_clone(), cx);
                    }
                });
            let author = if note.author.is_user() {
                "You".to_owned()
            } else {
                note.author.to_string()
            };
            // Receipts and reviews say so, in the review panel's colors.
            let (tag, tag_color) = match &note.kind {
                NoteKind::Note => (None, t.colors.text_muted),
                NoteKind::Receipt(_) => (Some("handed off for review"), t.colors.review),
                NoteKind::Review {
                    verdict: Verdict::Approved,
                } => (Some("approved"), t.colors.review),
                NoteKind::Review {
                    verdict: Verdict::ChangesRequested,
                } => (Some("sent back"), t.colors.error),
            };
            let details = match &note.kind {
                NoteKind::Receipt(receipt) => Some(receipt_details(receipt, &t)),
                _ => None,
            };
            div()
                .id(("note", id.0 as u64))
                .group("note")
                .flex()
                .flex_col()
                .pl_3()
                .pr_1()
                .py_1()
                .border_l_2()
                .border_color(if tag.is_some() {
                    tag_color
                } else if note.author.is_agent() {
                    t.colors.accent
                } else {
                    t.colors.border
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(12.))
                        .line_height(px(20.))
                        .text_color(t.colors.text_muted)
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(t.colors.text)
                                .child(author),
                        )
                        .children(tag.map(|label| div().text_color(tag_color).child(label)))
                        .child(format_age(note.created_at))
                        .child(div().flex_1())
                        .child(delete),
                )
                .when(!note.text.is_empty(), |s| {
                    s.child(div().line_height(px(22.)).child(note.text.clone()))
                })
                .children(details)
        });

        let draft = div()
            .key_context(DRAFT_CONTEXT)
            .on_action(cx.listener(Self::on_add_draft_note))
            .relative()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .min_h(px(52.))
            .px_2()
            .py_1()
            .bg(t.colors.input)
            .when(!t.is_bevel(), |s| {
                s.rounded(t.radius_sm())
                    .border_1()
                    .border_color(t.colors.border)
            })
            .line_height(px(22.))
            .child(detail.notes.draft.clone())
            .children(t.sunken_bevel());

        div()
            .mt_4()
            .mb_6()
            .flex()
            .flex_col()
            .gap_2()
            .child(header)
            .children(rows)
            .child(draft)
    }
}
