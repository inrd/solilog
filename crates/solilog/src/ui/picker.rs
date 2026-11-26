//! A filterable list popup: a text field above a list of items. The command
//! palette and the status picker both use it.
//!
//! Key context: `Picker` (see `assets/keymap.json`). Emits [`PickerEvent`];
//! the owner decides what confirming an item means and closes the picker.
//! With [`Picker::creatable`], typing something that matches no item exactly
//! offers a "Create …" row as well.

use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, Pixels, ScrollHandle,
    SharedString, Window, div, prelude::*, px,
};

use super::actions::picker::{Confirm, Dismiss, SelectNext, SelectPrevious};
use super::text_input::{Changed, Submit, TextInput};
use super::theme;
use super::tooltip::truncated_text;

pub const CONTEXT: &str = "Picker";

#[derive(Clone)]
pub struct PickerItem {
    pub label: SharedString,
    /// Right-aligned secondary text, e.g. a key binding.
    pub hint: Option<SharedString>,
    pub icon: Option<Rc<dyn Fn() -> AnyElement>>,
}

impl PickerItem {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            hint: None,
            icon: None,
        }
    }

    pub fn hint(mut self, hint: Option<impl Into<SharedString>>) -> Self {
        self.hint = hint.map(Into::into);
        self
    }

    pub fn icon<E: IntoElement>(mut self, icon: impl Fn() -> E + 'static) -> Self {
        self.icon = Some(Rc::new(move || icon().into_any_element()));
        self
    }
}

pub enum PickerEvent {
    /// Index into the items the picker was created with.
    Confirmed(usize),
    /// The "Create …" row was confirmed; carries the trimmed query.
    Create(String),
    Dismissed,
}

pub struct Picker {
    input: Entity<TextInput>,
    items: Vec<PickerItem>,
    /// Indices into `items` that match the query, best first.
    matches: Vec<usize>,
    /// Index into `matches`; `matches.len()` is the create row, if shown.
    highlighted: usize,
    scroll: ScrollHandle,
    /// Label prefix of the create row ("Create label"); `None` disables it.
    create_label: Option<SharedString>,
    /// The query the create row would create, when it is shown.
    create_query: Option<String>,
    width: Pixels,
}

impl EventEmitter<PickerEvent> for Picker {}

impl Picker {
    pub fn new(
        placeholder: &str,
        items: Vec<PickerItem>,
        highlighted: usize,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| TextInput::new(placeholder.to_owned(), cx));
        cx.subscribe(&input, |this, _, _: &Changed, cx| this.update_matches(cx))
            .detach();
        cx.subscribe(&input, |this, _, _: &Submit, cx| this.confirm(cx))
            .detach();
        let matches: Vec<usize> = (0..items.len()).collect();
        let highlighted = highlighted.min(items.len().saturating_sub(1));
        let scroll = ScrollHandle::new();
        scroll.scroll_to_item(highlighted);
        Self {
            input,
            items,
            matches,
            highlighted,
            scroll,
            create_label: None,
            create_query: None,
            width: px(480.),
        }
    }

    pub fn width(mut self, width: Pixels) -> Self {
        self.width = width;
        self
    }

    /// Starts with `query` typed in, selected so that typing replaces it.
    pub fn query(self, query: &str, cx: &mut Context<Self>) -> Self {
        self.input
            .update(cx, |input, cx| input.set_text(query.to_owned(), cx));
        self
    }

    /// Offers a "`label` “query”" row when the query matches no item exactly.
    pub fn creatable(mut self, label: impl Into<SharedString>) -> Self {
        self.create_label = Some(label.into());
        self
    }

    fn row_count(&self) -> usize {
        self.matches.len() + usize::from(self.create_query.is_some())
    }

    fn update_matches(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).text().to_owned();
        self.matches = rank(&query, self.items.iter().map(|i| i.label.as_ref()));
        let trimmed = query.trim();
        self.create_query = (self.create_label.is_some()
            && !trimmed.is_empty()
            && !self
                .items
                .iter()
                .any(|i| i.label.eq_ignore_ascii_case(trimmed)))
        .then(|| trimmed.to_owned());
        self.highlighted = 0;
        self.scroll.scroll_to_item(0);
        cx.notify();
    }

    fn move_highlight(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.row_count();
        if len == 0 {
            return;
        }
        self.highlighted = (self.highlighted as isize + delta).rem_euclid(len as isize) as usize;
        self.scroll.scroll_to_item(self.highlighted);
        cx.notify();
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        if let Some(&ix) = self.matches.get(self.highlighted) {
            cx.emit(PickerEvent::Confirmed(ix));
        } else if let Some(query) = self.create_query.clone() {
            cx.emit(PickerEvent::Create(query));
        }
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.move_highlight(1, cx);
    }

    fn select_previous(&mut self, _: &SelectPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.move_highlight(-1, cx);
    }

    fn on_confirm(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
        self.confirm(cx);
    }

    fn on_dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(PickerEvent::Dismissed);
    }

    fn render_item(&self, row: usize, item_ix: usize, cx: &mut Context<Self>) -> AnyElement {
        self.render_row(row, &self.items[item_ix], cx)
    }

    fn render_create_row(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let query = self.create_query.as_ref()?;
        let label = self.create_label.as_ref()?;
        let item = PickerItem::new(format!("{label} “{query}”"));
        Some(self.render_row(self.matches.len(), &item, cx))
    }

    fn render_row(&self, row: usize, item: &PickerItem, cx: &mut Context<Self>) -> AnyElement {
        let t = theme::current(cx);
        let is_highlighted = row == self.highlighted;
        div()
            .id(row)
            .relative()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .rounded(t.radius_sm())
            .cursor_pointer()
            .when(is_highlighted, |s| {
                s.bg(t.colors.highlight)
                    .text_color(t.colors.highlight_text)
                    .children(t.dotted_focus(t.colors.highlight_text))
                    .children(t.glass_selection_overlay())
            })
            .on_mouse_move(cx.listener(move |this, _, _, cx| {
                if this.highlighted != row {
                    this.highlighted = row;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.highlighted = row;
                this.confirm(cx);
            }))
            .children(item.icon.as_ref().map(|icon| icon()))
            .child(
                div()
                    .flex_1()
                    .truncate()
                    .child(truncated_text(("picker-label", row), item.label.clone())),
            )
            .children(item.hint.clone().map(|hint| {
                div()
                    .flex_none()
                    .text_size(px(12.))
                    .text_color(if is_highlighted {
                        t.colors.highlight_text
                    } else {
                        t.colors.text_muted
                    })
                    .child(hint)
            }))
            .into_any_element()
    }
}

impl Focusable for Picker {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl Render for Picker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::current(cx);
        let mut rows: Vec<_> = self
            .matches
            .iter()
            .enumerate()
            .map(|(row, &ix)| self.render_item(row, ix, cx))
            .collect();
        rows.extend(self.render_create_row(cx));
        let empty = rows.is_empty();

        let input_box = if t.is_bevel() {
            div().p_2().child(
                div()
                    .relative()
                    .px_2()
                    .py_1()
                    .bg(t.colors.input)
                    .text_color(t.colors.text)
                    .child(self.input.clone())
                    .children(t.sunken_bevel()),
            )
        } else {
            div()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(t.colors.border)
                .child(self.input.clone())
        };

        t.apply_popup_shadow(
            div()
                .key_context(CONTEXT)
                .on_action(cx.listener(Self::select_next))
                .on_action(cx.listener(Self::select_previous))
                .on_action(cx.listener(Self::on_confirm))
                .on_action(cx.listener(Self::on_dismiss))
                .relative()
                .w(self.width)
                .flex()
                .flex_col()
                .bg(t.colors.surface)
                .text_color(t.colors.text)
                .when(!t.is_bevel(), |d| {
                    d.border_1()
                        .border_color(t.colors.border)
                        .rounded(t.radius_md())
                })
                .when(t.is_bevel(), |d| d.p(px(2.)))
                .child(input_box)
                .child(
                    div()
                        .id("picker-items")
                        .max_h(px(320.))
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll)
                        .p_1()
                        .flex()
                        .flex_col()
                        .children(rows)
                        .when(empty, |s| {
                            s.child(
                                div()
                                    .px_2()
                                    .py_1()
                                    .text_color(t.colors.text_muted)
                                    .child("No matches"),
                            )
                        }),
                )
                .children(t.raised_bevel()),
        )
    }
}

/// Indices of `labels` matching `query`, best first. Every whitespace-separated
/// query word must occur in the label as a case-insensitive subsequence.
/// Ranking: contiguous substring hits beat scattered ones, earlier beats later,
/// ties keep their original order.
pub fn rank<'a>(query: &str, labels: impl Iterator<Item = &'a str>) -> Vec<usize> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut scored: Vec<(usize, usize)> = labels
        .enumerate()
        .filter_map(|(ix, label)| {
            let label = label.to_lowercase();
            let mut score = 0;
            for word in &words {
                score += match label.find(word.as_str()) {
                    Some(pos) if pos == 0 || !label.as_bytes()[pos - 1].is_ascii_alphanumeric() => {
                        0
                    }
                    Some(pos) => 1 + pos.min(50),
                    None if is_subsequence(word, &label) => 100,
                    None => return None,
                };
            }
            Some((score, ix))
        })
        .collect();
    scored.sort();
    scored.into_iter().map(|(_, ix)| ix).collect()
}

fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut hay = haystack.chars();
    needle.chars().all(|c| hay.any(|h| h == c))
}

#[cfg(test)]
mod tests {
    use super::rank;

    #[test]
    fn ranking() {
        let labels = ["Issue: Set Status", "Workspace: Quit", "Issue: New", "Done"];
        let r = |q| rank(q, labels.iter().copied());
        assert_eq!(r(""), [0, 1, 2, 3]);
        assert_eq!(r("quit"), [1]);
        assert_eq!(r("iss new"), [2]);
        assert_eq!(r("sst"), [0]); // subsequence
        assert_eq!(r("n"), [2, 3]); // word start beats inner match
        assert!(r("zzz").is_empty());
    }
}
