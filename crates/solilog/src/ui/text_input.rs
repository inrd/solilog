//! Text input: single-line (the default) or multi-line ([`TextInput::multiline`]).
//!
//! GPUI ships no input widget; this is adapted from gpui's `examples/input.rs`.
//! A single-line input emits [`Submit`] when Enter is pressed; a multi-line
//! one inserts a line break and wraps its text to its width. Key bindings
//! live in the keymap (`assets/keymap.json`, context `TextInput`; multi-line
//! inputs also have `multiline` in their context, for bindings like `up`).

use std::ops::Range;

use gpui::{
    App, AvailableSpace, Bounds, ClipboardItem, Context, CursorStyle, ElementId,
    ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable,
    GlobalElementId, KeyContext, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PaintQuad, Pixels, Point, ScrollHandle, SharedString, Style, TextAlign, TextRun,
    UTF16Selection, UnderlineStyle, Window, WrappedLine, actions, div, fill, point, prelude::*, px,
    relative, rgb, rgba, size,
};
use unicode_segmentation::*;

const CONTEXT: &str = "TextInput";
/// Added to [`CONTEXT`] for multi-line inputs.
const MULTILINE: &str = "multiline";

actions!(
    text_input,
    [
        Backspace,
        Delete,
        /// Delete back to the start of the previous word.
        DeleteWordLeft,
        Left,
        Right,
        /// Move to the start of the previous word.
        WordLeft,
        /// Move to the end of the next word.
        WordRight,
        SelectLeft,
        SelectRight,
        SelectWordLeft,
        SelectWordRight,
        SelectAll,
        /// Move to the start of the (visual) line.
        Home,
        /// Move to the end of the (visual) line.
        End,
        SelectHome,
        SelectEnd,
        /// Multi-line: move up a line.
        Up,
        /// Multi-line: move down a line.
        Down,
        SelectUp,
        SelectDown,
        /// Multi-line: move to the start of the text.
        MoveToStart,
        /// Multi-line: move to the end of the text.
        MoveToEnd,
        SelectToStart,
        SelectToEnd,
        /// Single-line: submit. Multi-line: insert a line break.
        Enter,
        ShowCharacterPalette,
        Paste,
        Cut,
        Copy,
    ]
);

/// Emitted when the user presses Enter in a single-line input.
pub struct Submit;

/// Emitted whenever the text changes.
pub struct Changed;

pub struct TextInput {
    focus_handle: FocusHandle,
    content: SharedString,
    placeholder: SharedString,
    multiline: bool,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    /// Where up/down movement tries to stay (x, from the left edge), so
    /// that crossing a short line doesn't lose the column.
    goal_x: Option<Pixels>,
    /// The scrolling container to scroll so that the cursor stays visible.
    autoscroll: Option<ScrollHandle>,
    /// The cursor moved since the last paint: scroll to it.
    cursor_moved: bool,
    last_layout: Option<TextLayout>,
    last_bounds: Option<Bounds<Pixels>>,
    is_selecting: bool,
}

impl EventEmitter<Submit> for TextInput {}
impl EventEmitter<Changed> for TextInput {}

impl TextInput {
    pub fn new(placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            content: SharedString::default(),
            placeholder: placeholder.into(),
            multiline: false,
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            goal_x: None,
            autoscroll: None,
            cursor_moved: false,
            last_layout: None,
            last_bounds: None,
            is_selecting: false,
        }
    }

    /// Makes this a multi-line input: Enter inserts a line break and the
    /// text wraps to the input's width.
    pub fn multiline(mut self) -> Self {
        self.multiline = true;
        self
    }

    /// Keeps the cursor visible by scrolling this container, which holds
    /// the input.
    pub fn autoscroll(mut self, scroll: ScrollHandle) -> Self {
        self.autoscroll = Some(scroll);
        self
    }

    pub fn text(&self) -> &str {
        &self.content
    }

    /// Replaces the text and selects all of it, so typing replaces it.
    pub fn set_text(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.content = self.sanitize(text.into());
        self.selected_range = 0..self.content.len();
        self.selection_reversed = false;
        self.marked_range = None;
        cx.emit(Changed);
        cx.notify();
    }

    /// Replaces the text, keeping the cursor where it was as far as the new
    /// text allows. For updates from elsewhere that the user didn't make.
    pub fn reset_text(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        let text = self.sanitize(text.into());
        if text == self.content {
            return;
        }
        self.content = text;
        let clamp = |ix: usize| {
            let mut ix = ix.min(self.content.len());
            while !self.content.is_char_boundary(ix) {
                ix -= 1;
            }
            ix
        };
        self.selected_range = clamp(self.selected_range.start)..clamp(self.selected_range.end);
        self.marked_range = None;
        cx.emit(Changed);
        cx.notify();
    }

    /// Puts the cursor at the end of the text.
    pub fn move_to_end(&mut self, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.content = SharedString::default();
        self.selected_range = 0..0;
        self.selection_reversed = false;
        self.marked_range = None;
        cx.emit(Changed);
        cx.notify();
    }

    /// Single-line inputs hold no line breaks.
    fn sanitize(&self, text: SharedString) -> SharedString {
        if self.multiline {
            if text.contains('\r') {
                text.replace("\r\n", "\n").replace('\r', "\n").into()
            } else {
                text
            }
        } else if text.contains(['\n', '\r']) {
            text.replace("\r\n", " ").replace(['\n', '\r'], " ").into()
        } else {
            text
        }
    }

    fn enter(&mut self, _: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        // Ignore Enter while an IME composition is in progress.
        if self.marked_range.is_some() {
            return;
        }
        if self.multiline {
            self.replace_text_in_range(None, "\n", window, cx);
        } else {
            cx.emit(Submit);
        }
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx)
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.selected_range.end), cx);
        } else {
            self.move_to(self.selected_range.end, cx)
        }
    }

    fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.previous_word_start(self.cursor_offset()), cx);
    }

    fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.next_word_end(self.cursor_offset()), cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_word_start(self.cursor_offset()), cx);
    }

    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_word_end(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx)
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.line_start(self.cursor_offset()), cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.line_end(self.cursor_offset()), cx);
    }

    fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.line_start(self.cursor_offset()), cx);
    }

    fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.line_end(self.cursor_offset()), cx);
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertically(-1, false, cx);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertically(1, false, cx);
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertically(-1, true, cx);
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertically(1, true, cx);
    }

    fn move_to_start(&mut self, _: &MoveToStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn move_to_end_action(&mut self, _: &MoveToEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    fn select_to_start(&mut self, _: &SelectToStart, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(0, cx);
    }

    fn select_to_text_end(&mut self, _: &SelectToEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.content.len(), cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete_word_left(
        &mut self,
        _: &DeleteWordLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_word_start(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle);
        self.is_selecting = true;
        if event.modifiers.shift {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        } else {
            self.move_to(self.index_for_mouse_position(event.position), cx)
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let text = self.sanitize(text.into());
            self.replace_text_in_range(None, &text, window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx)
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        self.goal_x = None;
        self.cursor_moved = true;
        cx.notify()
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    /// Moves the cursor `rows` visual lines down (up when negative), keeping
    /// its column; past the first or last line, to the start or end.
    fn move_vertically(&mut self, rows: i32, select: bool, cx: &mut Context<Self>) {
        let Some(layout) = self.last_layout.as_ref() else {
            return;
        };
        let cursor = if select || self.selected_range.is_empty() {
            self.cursor_offset()
        } else if rows < 0 {
            self.selected_range.start
        } else {
            self.selected_range.end
        };
        let pos = layout.position_for_index(cursor);
        let x = self.goal_x.unwrap_or(pos.x);
        let y = pos.y + layout.line_height * rows as f32 + layout.line_height / 2.;
        let target = if y < px(0.) {
            0
        } else if y >= layout.height() {
            self.content.len()
        } else {
            layout.index_for_position(point(x, y))
        };
        if select {
            self.select_to(target, cx);
        } else {
            self.move_to(target, cx);
        }
        self.goal_x = Some(x);
    }

    /// Start of the visual line holding `offset`.
    fn line_start(&self, offset: usize) -> usize {
        match &self.last_layout {
            Some(layout) => {
                let pos = layout.position_for_index(offset);
                layout.index_for_position(point(px(0.), pos.y + layout.line_height / 2.))
            }
            None => 0,
        }
    }

    /// End of the visual line holding `offset`.
    fn line_end(&self, offset: usize) -> usize {
        match &self.last_layout {
            Some(layout) => {
                let pos = layout.position_for_index(offset);
                layout.index_for_position(point(Pixels::MAX, pos.y + layout.line_height / 2.))
            }
            None => self.content.len(),
        }
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }
        let (Some(bounds), Some(layout)) = (self.last_bounds.as_ref(), self.last_layout.as_ref())
        else {
            return 0;
        };
        if position.y < bounds.top() {
            return 0;
        }
        if position.y > bounds.top() + layout.height() {
            return self.content.len();
        }
        layout.index_for_position(position - bounds.origin)
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset
        } else {
            self.selected_range.end = offset
        };
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        self.goal_x = None;
        self.cursor_moved = true;
        cx.notify()
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;
        for ch in self.content.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }
        utf8_offset
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;
        for ch in self.content.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }
        utf16_offset
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range_utf16.start)..self.offset_from_utf16(range_utf16.end)
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(idx, _)| (idx < offset).then_some(idx))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(idx, _)| (idx > offset).then_some(idx))
            .unwrap_or(self.content.len())
    }

    fn previous_word_start(&self, offset: usize) -> usize {
        previous_word_start(&self.content, offset)
    }

    fn next_word_end(&self, offset: usize) -> usize {
        next_word_end(&self.content, offset)
    }
}

/// Start of the word before `offset` (skipping whitespace and punctuation
/// in between).
fn previous_word_start(text: &str, offset: usize) -> usize {
    text.split_word_bound_indices()
        .rev()
        .find(|(ix, word)| *ix < offset && is_word(word))
        .map_or(0, |(ix, _)| ix)
}

/// End of the word after `offset` (skipping whitespace and punctuation in
/// between).
fn next_word_end(text: &str, offset: usize) -> usize {
    text.split_word_bound_indices()
        .map(|(ix, word)| (ix + word.len(), word))
        .find(|(end, word)| *end > offset && is_word(word))
        .map_or(text.len(), |(end, _)| end)
}

fn is_word(segment: &str) -> bool {
    segment.chars().any(char::is_alphanumeric)
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        self.selected_range = range.start + new_text.len()..range.start + new_text.len();
        self.selection_reversed = false;
        self.marked_range.take();
        self.goal_x = None;
        self.cursor_moved = true;
        cx.emit(Changed);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        if !new_text.is_empty() {
            self.marked_range = Some(range.start..range.start + new_text.len());
        } else {
            self.marked_range = None;
        }
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .map(|new_range| new_range.start + range.start..new_range.end + range.end)
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());
        self.goal_x = None;
        self.cursor_moved = true;

        cx.emit(Changed);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        let start = layout.position_for_index(range.start);
        let end = layout.position_for_index(range.end);
        Some(Bounds::from_corners(
            bounds.origin + start,
            bounds.origin + point(end.x, end.y + layout.line_height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let bounds = self.last_bounds?;
        let layout = self.last_layout.as_ref()?;
        let utf8_index = layout.index_for_position(point - bounds.origin);
        Some(self.offset_to_utf16(utf8_index))
    }
}

/// Shaped text: one [`WrappedLine`] per line of the text (split at `\n`),
/// each taking one or more visual lines.
#[derive(Clone)]
struct TextLayout {
    lines: Vec<WrappedLine>,
    /// Byte offset where each line starts in the text.
    starts: Vec<usize>,
    /// Distance from the top to each line.
    tops: Vec<Pixels>,
    line_height: Pixels,
}

impl TextLayout {
    fn new(lines: Vec<WrappedLine>, line_height: Pixels) -> Self {
        let mut starts = Vec::with_capacity(lines.len());
        let mut tops = Vec::with_capacity(lines.len());
        let (mut start, mut top) = (0, px(0.));
        for line in &lines {
            starts.push(start);
            tops.push(top);
            start += line.len() + 1;
            top += line.size(line_height).height;
        }
        Self {
            lines,
            starts,
            tops,
            line_height,
        }
    }

    fn height(&self) -> Pixels {
        match (self.lines.last(), self.tops.last()) {
            (Some(line), Some(top)) => *top + line.size(self.line_height).height,
            _ => self.line_height,
        }
    }

    /// Top-left corner of the character at `offset`, from the top-left of the text.
    fn position_for_index(&self, offset: usize) -> Point<Pixels> {
        let ix = self
            .starts
            .partition_point(|s| *s <= offset)
            .saturating_sub(1);
        let Some(line) = self.lines.get(ix) else {
            return point(px(0.), px(0.));
        };
        let local = (offset - self.starts[ix]).min(line.len());
        let pos = line
            .position_for_index(local, self.line_height)
            .unwrap_or_default();
        point(pos.x, pos.y + self.tops[ix])
    }

    /// The offset closest to `position` (from the top-left of the text).
    fn index_for_position(&self, position: Point<Pixels>) -> usize {
        let ix = self
            .tops
            .partition_point(|top| *top <= position.y)
            .saturating_sub(1);
        let Some(line) = self.lines.get(ix) else {
            return 0;
        };
        let height = line.size(self.line_height).height;
        // Stay within the line's rows: below them, it reports offset 0.
        let y = (position.y - self.tops[ix]).clamp(px(0.), height - self.line_height / 2.);
        let local = match line.closest_index_for_position(point(position.x, y), self.line_height) {
            Ok(ix) | Err(ix) => ix,
        };
        self.starts[ix] + local.min(line.len())
    }
}

/// Low-level element that shapes and paints the text, selection and cursor.
struct TextElement {
    input: Entity<TextInput>,
}

struct PrepaintState {
    layout: Option<TextLayout>,
    cursor: Option<PaintQuad>,
    selection: Vec<PaintQuad>,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// Text, color and runs to shape: the content, or the placeholder when empty.
fn display_runs(input: &TextInput, window: &Window) -> (SharedString, Vec<TextRun>) {
    let style = window.text_style();
    let (display_text, text_color) = if input.content.is_empty() {
        (input.placeholder.clone(), style.color.opacity(0.35))
    } else {
        (input.content.clone(), style.color)
    };
    let run = TextRun {
        len: display_text.len(),
        font: style.font(),
        color: text_color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let runs = match input.marked_range.as_ref() {
        Some(marked_range) => [
            TextRun {
                len: marked_range.start,
                ..run.clone()
            },
            TextRun {
                len: marked_range.end - marked_range.start,
                underline: Some(UnderlineStyle {
                    color: Some(run.color),
                    thickness: px(1.0),
                    wavy: false,
                }),
                ..run.clone()
            },
            TextRun {
                len: display_text.len() - marked_range.end,
                ..run
            },
        ]
        .into_iter()
        .filter(|run| run.len > 0)
        .collect(),
        None => vec![run],
    };
    (display_text, runs)
}

fn shape(
    text: SharedString,
    runs: &[TextRun],
    wrap_width: Option<Pixels>,
    window: &Window,
) -> TextLayout {
    let style = window.text_style();
    let font_size = style.font_size.to_pixels(window.rem_size());
    let lines = window
        .text_system()
        .shape_text(text, font_size, runs, wrap_width, None)
        .map(|lines| lines.into_vec())
        .unwrap_or_default();
    TextLayout::new(lines, window.line_height())
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        let input = self.input.read(cx);
        if !input.multiline {
            style.size.height = window.line_height().into();
            return (window.request_layout(style, [], cx), ());
        }
        // Multi-line: as tall as the text, wrapped to the width given.
        let (text, runs) = display_runs(input, window);
        let layout_id =
            window.request_measured_layout(style, move |known, available, window, _| {
                let width = known.width.or(match available.width {
                    AvailableSpace::Definite(width) => Some(width),
                    _ => None,
                });
                let layout = shape(text.clone(), &runs, width, window);
                size(width.unwrap_or_default(), layout.height())
            });
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let selected_range = input.selected_range.clone();
        let cursor = input.cursor_offset();
        let (text, runs) = display_runs(input, window);
        let wrap_width = input.multiline.then_some(bounds.size.width);
        let layout = shape(text, &runs, wrap_width, window);
        let line_height = layout.line_height;
        let cursor_color = rgb(0x7c8cff);
        let selection_color = rgba(0x7c8cff40);

        let selection = if selected_range.is_empty() {
            Vec::new()
        } else {
            let start = layout.position_for_index(selected_range.start);
            let end = layout.position_for_index(selected_range.end);
            let row = |top: Pixels, left: Pixels, right: Pixels| {
                fill(
                    Bounds::from_corners(
                        point(bounds.left() + left, bounds.top() + top),
                        point(bounds.left() + right, bounds.top() + top + line_height),
                    ),
                    selection_color,
                )
            };
            if start.y == end.y {
                vec![row(start.y, start.x, end.x)]
            } else {
                // First row to the right edge, full rows, last row from the left edge.
                let mut quads = vec![row(start.y, start.x, bounds.size.width)];
                let mut top = start.y + line_height;
                while top < end.y {
                    quads.push(row(top, px(0.), bounds.size.width));
                    top += line_height;
                }
                quads.push(row(end.y, px(0.), end.x));
                quads
            }
        };
        let cursor_pos = layout.position_for_index(cursor);
        let cursor_quad = selected_range.is_empty().then(|| {
            fill(
                Bounds::new(
                    point(bounds.left() + cursor_pos.x, bounds.top() + cursor_pos.y),
                    size(px(1.5), line_height),
                ),
                cursor_color,
            )
        });

        // Keep the cursor in view.
        if input.cursor_moved
            && input.focus_handle.is_focused(window)
            && let Some(scroll) = &input.autoscroll
        {
            let view = scroll.bounds();
            let top = bounds.top() + cursor_pos.y;
            let bottom = top + line_height;
            let margin = line_height;
            let mut offset = scroll.offset();
            let old_y = offset.y;
            if top < view.top() + margin {
                offset.y += view.top() + margin - top;
            } else if bottom > view.bottom() - margin {
                offset.y -= bottom - (view.bottom() - margin);
            }
            offset.y = offset.y.clamp(-scroll.max_offset().height, px(0.));
            if offset.y != old_y {
                scroll.set_offset(offset);
                window.request_animation_frame();
            }
        }

        PrepaintState {
            layout: Some(layout),
            cursor: cursor_quad,
            selection,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        for selection in prepaint.selection.drain(..) {
            window.paint_quad(selection)
        }
        let Some(layout) = prepaint.layout.take() else {
            return;
        };
        for (line, top) in layout.lines.iter().zip(&layout.tops) {
            let origin = point(bounds.left(), bounds.top() + *top);
            // Painting only fails if a glyph can't be rasterized; skip it then.
            line.paint(
                origin,
                layout.line_height,
                TextAlign::Left,
                None,
                window,
                cx,
            )
            .ok();
        }

        let focused = focus_handle.is_focused(window);
        if focused && let Some(cursor) = prepaint.cursor.take() {
            window.paint_quad(cursor);
        }

        self.input.update(cx, |input, _| {
            input.last_layout = Some(layout);
            input.last_bounds = Some(bounds);
            if focused {
                input.cursor_moved = false;
            }
        });
    }
}

impl Render for TextInput {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut context = KeyContext::new_with_defaults();
        context.add(CONTEXT);
        if self.multiline {
            context.add(MULTILINE);
        }
        div()
            .flex()
            .w_full()
            .when(self.multiline, |s| s.flex_col().flex_1())
            .key_context(context)
            .track_focus(&self.focus_handle(cx))
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .when(self.multiline, |s| {
                s.on_action(cx.listener(Self::up))
                    .on_action(cx.listener(Self::down))
                    .on_action(cx.listener(Self::select_up))
                    .on_action(cx.listener(Self::select_down))
                    .on_action(cx.listener(Self::move_to_start))
                    .on_action(cx.listener(Self::move_to_end_action))
                    .on_action(cx.listener(Self::select_to_start))
                    .on_action(cx.listener(Self::select_to_text_end))
            })
            .on_action(cx.listener(Self::enter))
            .on_action(cx.listener(Self::show_character_palette))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .child(TextElement { input: cx.entity() })
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_movement() {
        let text = "fix the  reverb-tail, now";
        assert_eq!(previous_word_start(text, text.len()), 22);
        assert_eq!(previous_word_start(text, 22), 16);
        assert_eq!(previous_word_start(text, 9), 4);
        assert_eq!(previous_word_start(text, 2), 0);
        assert_eq!(previous_word_start(text, 0), 0);
        assert_eq!(next_word_end(text, 0), 3);
        assert_eq!(next_word_end(text, 3), 7);
        assert_eq!(next_word_end(text, 7), 15);
        assert_eq!(next_word_end(text, 22), text.len());
        assert_eq!(next_word_end(text, text.len()), text.len());
    }
}
