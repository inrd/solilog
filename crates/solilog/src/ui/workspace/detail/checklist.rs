//! The checklist of the open issue: one single-line input per item, each with
//! a checkbox, then a field to add items.
//!
//! Key contexts: `Checklist` around it all, `ChecklistItem` around each item.
//! Items can be dragged by the handle on their left to reorder them.
//! Enter in an item goes to the next one; Enter in the new-item field adds
//! the item. Item text is saved like the title and description; an item
//! whose text is emptied is deleted when it loses focus. A draft left in the
//! new-item field is added when it loses focus.
//!
//! Rows follow `IssueDetail::issue.checklist`: they are matched to items by
//! id when the detail renders (see [`Workspace::sync_checklist`]), so an item
//! keeps its input, and unsaved edits, through reloads and moves.

use std::collections::HashMap;

use gpui::{
    Action, ClickEvent, Context, CursorStyle, Entity, FocusHandle, Focusable, FontWeight,
    IntoElement, SharedString, Subscription, Window, div, prelude::*, px, relative, rgb,
};
use solilog_core::{ChecklistItem, ChecklistItemId, Placement};

use super::super::Workspace;
use super::IssueDetail;
use crate::ui::actions::checklist::{
    AddItem, DeleteItem, FocusNext, FocusPrevious, MoveItem, MoveItemDown, MoveItemUp, ToggleItem,
};
use crate::ui::text_input::{Changed, Submit, TextInput};
use crate::ui::theme;

pub const CONTEXT: &str = "Checklist";
pub const ITEM_CONTEXT: &str = "ChecklistItem";

pub(super) struct Checklist {
    /// One per item, in checklist order (as of the last render).
    rows: Vec<ItemRow>,
    new_item: Entity<TextInput>,
    _subscriptions: Vec<Subscription>,
}

struct ItemRow {
    /// The item as last shown.
    item: ChecklistItem,
    input: Entity<TextInput>,
    _subscriptions: Vec<Subscription>,
}

/// Whether an item's input holds something other than `saved`.
fn item_changed(text: &str, saved: &str) -> bool {
    text.trim() != saved
}

/// An item being dragged by its handle; also renders what follows the mouse.
#[derive(Clone)]
struct DraggedItem {
    id: ChecklistItemId,
    text: SharedString,
    /// Index in the checklist, to tell moving up from moving down.
    index: usize,
}

impl Render for DraggedItem {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .max_w(px(480.))
            .px_2()
            .rounded_sm()
            .bg(rgb(theme::HIGHLIGHT))
            .border_1()
            .border_color(rgb(theme::BORDER))
            .line_height(px(24.))
            .text_color(rgb(theme::TEXT))
            .truncate()
            .child(self.text.clone())
    }
}

/// A write to make when saving the checklist.
enum Write {
    Edit(ChecklistItemId, String),
    Delete(ChecklistItemId),
    Add(String),
}

impl Checklist {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let new_item = cx.new(|cx| TextInput::new("Add an item…", cx));
        let subscriptions = vec![
            cx.subscribe_in(&new_item, window, |this, _, _: &Submit, _, cx| {
                this.add_drafted_item(cx)
            }),
            cx.on_blur(&new_item.focus_handle(cx), window, |_, window, cx| {
                cx.defer_in(window, |this, _, cx| this.save_detail(true, cx))
            }),
        ];
        Self {
            rows: Vec::new(),
            new_item,
            _subscriptions: subscriptions,
        }
    }

    pub(super) fn new_item_focus(&self, cx: &gpui::App) -> FocusHandle {
        self.new_item.focus_handle(cx)
    }

    /// The input of row `ix`, or the new-item field for `rows.len()`.
    fn field(&self, ix: usize) -> Option<&Entity<TextInput>> {
        match self.rows.get(ix) {
            Some(row) => Some(&row.input),
            None => (ix == self.rows.len()).then_some(&self.new_item),
        }
    }

    /// The field being edited: an item's index, or `rows.len()` for the
    /// new-item field.
    fn focused_field(&self, window: &Window, cx: &gpui::App) -> Option<usize> {
        (0..=self.rows.len()).find(|&ix| {
            self.field(ix)
                .is_some_and(|input| input.focus_handle(cx).is_focused(window))
        })
    }

    /// The id of the item being edited.
    fn focused_item(&self, window: &Window, cx: &gpui::App) -> Option<ChecklistItemId> {
        let ix = self.focused_field(window, cx)?;
        self.rows.get(ix).map(|row| row.item.id)
    }
}

fn item_row(item: ChecklistItem, window: &mut Window, cx: &mut Context<Workspace>) -> ItemRow {
    let id = item.id;
    let input = cx.new(|cx| {
        let mut input = TextInput::new("Item", cx);
        input.reset_text(item.text.clone(), cx);
        input
    });
    let subscriptions = vec![
        cx.subscribe_in(&input, window, |this, _, _: &Changed, window, cx| {
            this.schedule_autosave(window, cx)
        }),
        // Enter: on to the next item.
        cx.subscribe_in(&input, window, move |this, _, _: &Submit, window, cx| {
            let Some(detail) = &this.detail else {
                return;
            };
            if let Some(ix) = detail.checklist.rows.iter().position(|r| r.item.id == id) {
                this.focus_checklist_field(ix + 1, window, cx);
            }
        }),
        cx.on_blur(&input.focus_handle(cx), window, |_, window, cx| {
            cx.defer_in(window, |this, _, cx| this.save_detail(true, cx))
        }),
    ];
    ItemRow {
        item,
        input,
        _subscriptions: subscriptions,
    }
}

impl Workspace {
    /// Matches the rows to the open issue's checklist: new items get a row,
    /// rows of deleted items go, and inputs without unsaved edits show the
    /// item's current text.
    pub(in crate::ui::workspace) fn sync_checklist(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(detail) = &mut self.detail else {
            return;
        };
        let items = &detail.issue.checklist;
        let in_sync = items.len() == detail.checklist.rows.len()
            && items
                .iter()
                .zip(&detail.checklist.rows)
                .all(|(i, r)| *i == r.item);
        if in_sync {
            return;
        }
        let mut old: HashMap<ChecklistItemId, ItemRow> = detail
            .checklist
            .rows
            .drain(..)
            .map(|row| (row.item.id, row))
            .collect();
        let mut rows = Vec::with_capacity(items.len());
        for item in items.clone() {
            let row = match old.remove(&item.id) {
                Some(mut row) => {
                    let text = row.input.read(cx).text();
                    if !item_changed(text, &row.item.text) && item_changed(text, &item.text) {
                        let text = item.text.clone();
                        row.input.update(cx, |input, cx| input.reset_text(text, cx));
                    }
                    row.item = item;
                    row
                }
                None => item_row(item, window, cx),
            };
            rows.push(row);
        }
        detail.checklist.rows = rows;
    }

    /// Writes edited items: changed text is saved; emptied items are
    /// deleted when `leaving` (the user is done with the field), along with
    /// adding the draft in the new-item field. Returns whether anything was
    /// written.
    pub(super) fn save_checklist(&mut self, leaving: bool, cx: &mut Context<Self>) -> bool {
        let Some(detail) = &self.detail else {
            return false;
        };
        let mut writes = Vec::new();
        for row in &detail.checklist.rows {
            // Items deleted meanwhile (by a command, or elsewhere) are left alone.
            let Some(saved) = detail.issue.checklist.iter().find(|i| i.id == row.item.id) else {
                continue;
            };
            let text = row.input.read(cx).text();
            if !item_changed(text, &saved.text) {
                continue;
            }
            if !text.trim().is_empty() {
                writes.push(Write::Edit(saved.id, text.to_owned()));
            } else if leaving {
                writes.push(Write::Delete(saved.id));
            }
        }
        if leaving {
            let draft = detail.checklist.new_item.read(cx).text().trim().to_owned();
            if !draft.is_empty() {
                detail
                    .checklist
                    .new_item
                    .update(cx, |input, cx| input.clear(cx));
                writes.push(Write::Add(draft));
            }
        }
        let issue_id = detail.issue.id;
        let mut wrote = false;
        for write in writes {
            let result = match write {
                Write::Edit(id, text) => self.db.edit_checklist_item(id, &text),
                Write::Delete(id) => self.db.delete_checklist_item(id),
                Write::Add(text) => self.db.add_checklist_items(issue_id, &[text]),
            };
            let Some(issue) = self.report(result, cx) else {
                break;
            };
            if let Some(detail) = &mut self.detail {
                detail.issue = issue;
            }
            wrote = true;
        }
        wrote
    }

    /// Enter in the new-item field: adds the item, and stays there for the next.
    fn add_drafted_item(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = &self.detail else {
            return;
        };
        let text = detail.checklist.new_item.read(cx).text().to_owned();
        if text.trim().is_empty() {
            return;
        }
        let id = detail.issue.id;
        if self
            .report(self.db.add_checklist_items(id, &[text]), cx)
            .is_some()
        {
            if let Some(detail) = &self.detail {
                detail
                    .checklist
                    .new_item
                    .update(cx, |input, cx| input.clear(cx));
            }
            self.reload(cx);
        }
    }

    /// Puts the cursor at the end of item `ix`, or of the new-item field
    /// for one past the last item.
    fn focus_checklist_field(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = self
            .detail
            .as_ref()
            .and_then(|d| d.checklist.field(ix))
            .cloned()
        else {
            return;
        };
        window.focus(&input.focus_handle(cx));
        input.update(cx, |input, cx| input.move_to_end(cx));
    }

    /// The item an item command acts on: the given one, else the one being
    /// edited. Shows an error when there is none.
    fn checklist_target(
        &mut self,
        id: Option<ChecklistItemId>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<ChecklistItemId> {
        let id = id.or_else(|| {
            self.detail
                .as_ref()
                .and_then(|d| d.checklist.focused_item(window, cx))
        });
        if id.is_none() {
            self.error = Some("No checklist item is being edited".into());
            cx.notify();
        }
        id
    }

    // ---- actions ------------------------------------------------------------

    pub(in crate::ui::workspace) fn on_add_checklist_item(
        &mut self,
        action: &AddItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(issue) = self.selected_issue(cx) else {
            return;
        };
        if let Some(text) = &action.text {
            let added = self
                .db
                .add_checklist_items(issue.id, std::slice::from_ref(text));
            if self.report(added, cx).is_some() {
                self.reload(cx);
            }
            return;
        }
        if self.detail.as_ref().map(|d| d.issue.id) != Some(issue.id) {
            self.close_modal(window, cx);
            self.open_issue(issue.id, window, cx);
        }
        if let Some(detail) = &self.detail {
            window.focus(&detail.checklist.new_item_focus(cx));
        }
    }

    /// Registered on the detail pane, like `on_delete_checklist_item`:
    /// clicks dispatch it from there.
    pub(super) fn on_toggle_checklist_item(
        &mut self,
        action: &ToggleItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.checklist_target(action.id, window, cx) else {
            return;
        };
        let done = self
            .detail
            .as_ref()
            .and_then(|d| d.issue.checklist.iter().find(|i| i.id == id))
            .is_some_and(|i| i.done);
        if self
            .report(self.db.set_checklist_item_done(id, !done), cx)
            .is_some()
        {
            self.reload(cx);
        }
    }

    pub(super) fn on_delete_checklist_item(
        &mut self,
        action: &DeleteItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.checklist_target(action.id, window, cx) else {
            return;
        };
        // Deleting the item being edited: go on to the next one first, so
        // that the focus doesn't vanish with it.
        if let Some(detail) = &self.detail
            && let Some(ix) = detail.checklist.focused_field(window, cx)
            && detail
                .checklist
                .rows
                .get(ix)
                .is_some_and(|r| r.item.id == id)
        {
            self.focus_checklist_field(ix + 1, window, cx);
        }
        if self.report(self.db.delete_checklist_item(id), cx).is_some() {
            self.reload(cx);
        }
    }

    /// Moves the item being edited one place up (`-1`) or down (`1`).
    fn move_checklist_item(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(detail) = &self.detail else {
            return;
        };
        let rows = &detail.checklist.rows;
        let Some(ix) = detail.checklist.focused_field(window, cx) else {
            return;
        };
        let (Some(row), Some(neighbor)) = (
            rows.get(ix),
            ix.checked_add_signed(delta).and_then(|n| rows.get(n)),
        ) else {
            return;
        };
        let placement = if delta < 0 {
            Placement::Before(neighbor.item.id)
        } else {
            Placement::After(neighbor.item.id)
        };
        let moved = self.db.move_checklist_item(row.item.id, &placement);
        if self.report(moved, cx).is_some() {
            self.reload(cx);
        }
    }

    /// Registered on the detail pane: drops dispatch it from there.
    pub(super) fn on_move_item(
        &mut self,
        action: &MoveItem,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let moved = Placement::from_options(action.before, action.after)
            .and_then(|placement| self.db.move_checklist_item(action.id, &placement));
        if self.report(moved, cx).is_some() {
            self.reload(cx);
        }
    }

    fn on_move_item_up(&mut self, _: &MoveItemUp, window: &mut Window, cx: &mut Context<Self>) {
        self.move_checklist_item(-1, window, cx);
    }

    fn on_move_item_down(&mut self, _: &MoveItemDown, window: &mut Window, cx: &mut Context<Self>) {
        self.move_checklist_item(1, window, cx);
    }

    fn on_focus_next(&mut self, _: &FocusNext, window: &mut Window, cx: &mut Context<Self>) {
        let focused = self
            .detail
            .as_ref()
            .and_then(|d| d.checklist.focused_field(window, cx));
        if let Some(ix) = focused {
            self.focus_checklist_field(ix + 1, window, cx);
        }
    }

    fn on_focus_previous(
        &mut self,
        _: &FocusPrevious,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focused = self
            .detail
            .as_ref()
            .and_then(|d| d.checklist.focused_field(window, cx));
        if let Some(ix) = focused.and_then(|ix| ix.checked_sub(1)) {
            self.focus_checklist_field(ix, window, cx);
        }
    }

    // ---- rendering ------------------------------------------------------------

    pub(super) fn render_checklist(
        &self,
        detail: &IssueDetail,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let pane_focus = detail.focus.clone();
        let header = div()
            .flex()
            .items_center()
            .gap_3()
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Checklist"))
            .children(detail.issue.checklist_progress().map(|(done, total)| {
                let complete = done == total;
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(12.))
                            .text_color(rgb(if complete {
                                theme::ACCENT
                            } else {
                                theme::TEXT_MUTED
                            }))
                            .child(format!("{done}/{total}")),
                    )
                    .child(
                        div()
                            .flex_1()
                            .max_w(px(240.))
                            .h(px(4.))
                            .rounded_full()
                            .bg(rgb(theme::HIGHLIGHT))
                            .child(
                                div()
                                    .h_full()
                                    .rounded_full()
                                    .bg(rgb(theme::ACCENT))
                                    .w(relative(done as f32 / total as f32)),
                            ),
                    )
            }));

        let rows = detail
            .checklist
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let id = row.item.id;
                let dragged = DraggedItem {
                    id,
                    text: row.input.read(cx).text().to_owned().into(),
                    index,
                };
                let handle = drag_handle()
                    .id(("checklist-handle", id.0 as u64))
                    .invisible()
                    .group_hover("checklist-item", |s| s.visible())
                    .cursor(CursorStyle::OpenHand)
                    .on_drag(dragged, |dragged, _, _, cx| cx.new(|_| dragged.clone()));
                let done = row.item.done;
                let checkbox = checkbox(done)
                    .id(("checklist-check", id.0 as u64))
                    .cursor_pointer()
                    .on_click({
                        let focus = pane_focus.clone();
                        move |_: &ClickEvent, window, cx| {
                            window.focus(&focus);
                            window.dispatch_action(ToggleItem { id: Some(id) }.boxed_clone(), cx);
                        }
                    });
                let delete = div()
                    .id(("checklist-delete", id.0 as u64))
                    .flex_none()
                    .px_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .invisible()
                    .group_hover("checklist-item", |s| s.visible())
                    .text_color(rgb(theme::TEXT_MUTED))
                    .hover(|s| s.bg(rgb(theme::HIGHLIGHT)).text_color(rgb(theme::TEXT)))
                    .child("×")
                    .on_click({
                        let focus = pane_focus.clone();
                        move |_: &ClickEvent, window, cx| {
                            window.focus(&focus);
                            window.dispatch_action(DeleteItem { id: Some(id) }.boxed_clone(), cx);
                        }
                    });
                div()
                    .id(("checklist-item", id.0 as u64))
                    .key_context(ITEM_CONTEXT)
                    .group("checklist-item")
                    .flex()
                    .items_center()
                    .gap_1()
                    .pr_1()
                    .py(px(2.))
                    .line_height(px(22.))
                    .rounded_sm()
                    .hover(|s| s.bg(rgb(theme::HOVER)))
                    .can_drop(move |value, _, _| {
                        value
                            .downcast_ref::<DraggedItem>()
                            .is_some_and(|d| d.id != id)
                    })
                    .drag_over::<DraggedItem>(move |style, dragged, _, _| {
                        if dragged.id == id {
                            return style;
                        }
                        // A line where the item will land; the padding it
                        // replaces keeps the row height.
                        let style = style.rounded_none().border_color(rgb(theme::ACCENT));
                        if dragged.index < index {
                            style.border_b_2().pb(px(0.))
                        } else {
                            style.border_t_2().pt(px(0.))
                        }
                    })
                    .on_drop({
                        let focus = pane_focus.clone();
                        move |dragged: &DraggedItem, window, cx| {
                            if dragged.id == id {
                                return;
                            }
                            let (before, after) = if dragged.index < index {
                                (None, Some(id))
                            } else {
                                (Some(id), None)
                            };
                            let action = MoveItem {
                                id: dragged.id,
                                before,
                                after,
                            };
                            focus.dispatch_action(&action, window, cx);
                        }
                    })
                    .child(handle)
                    .child(div().mr_1().child(checkbox))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .when(done, |s| {
                                s.text_color(rgb(theme::TEXT_MUTED)).line_through()
                            })
                            .child(row.input.clone()),
                    )
                    .when(row.item.author.is_agent(), |s| {
                        s.child(
                            div()
                                .flex_none()
                                .text_size(px(11.))
                                .text_color(rgb(theme::TEXT_MUTED))
                                .child(format!("by {}", row.item.author)),
                        )
                    })
                    .child(delete)
            });

        let new_item = div()
            .flex()
            .items_center()
            .gap_1()
            .pr_1()
            .py(px(2.))
            .line_height(px(22.))
            // Where the handles are on items.
            .child(drag_handle().invisible())
            .child(
                checkbox(false)
                    .border_dashed()
                    .text_color(rgb(theme::TEXT_MUTED))
                    .child("+"),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .child(detail.checklist.new_item.clone()),
            );

        div()
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::on_move_item_up))
            .on_action(cx.listener(Self::on_move_item_down))
            .on_action(cx.listener(Self::on_focus_next))
            .on_action(cx.listener(Self::on_focus_previous))
            .mt_4()
            .flex()
            .flex_col()
            .gap_1()
            .line_height(px(26.))
            .child(header)
            .children(rows)
            .child(new_item)
    }
}

/// Two columns of three dots, to drag an item by.
fn drag_handle() -> gpui::Div {
    let column =
        || {
            div().flex().flex_col().gap(px(2.)).children(
                (0..3).map(|_| div().size(px(2.)).rounded_full().bg(rgb(theme::TEXT_MUTED))),
            )
        };
    div()
        .w(px(14.))
        .h(px(18.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(2.))
        .rounded_sm()
        .hover(|s| s.bg(rgb(theme::HIGHLIGHT)))
        .child(column())
        .child(column())
}

fn checkbox(done: bool) -> gpui::Div {
    let checkbox = div()
        .size(px(16.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .border_1()
        .text_size(px(11.))
        .line_height(px(16.))
        .font_weight(FontWeight::BOLD);
    if done {
        checkbox
            .bg(rgb(theme::ACCENT))
            .border_color(rgb(theme::ACCENT))
            .text_color(rgb(theme::BACKGROUND))
            .child("✓")
    } else {
        checkbox.border_color(rgb(theme::TEXT_MUTED))
    }
}
