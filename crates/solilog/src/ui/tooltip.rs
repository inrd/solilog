//! Tooltips for cropped text (truncated titles and single-line inputs).
//!
//! Uses GPUI's window tooltip layer ([`Window::set_tooltip`]) so tooltips
//! stay inside the window, but only activates when the text is actually
//! cropped.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyTooltip, App, Bounds, Context, DispatchPhase, Element, ElementId, GlobalElementId, Hitbox,
    HitboxBehavior, InspectorElementId, IntoElement, LayoutId, MouseDownEvent, MouseMoveEvent,
    Pixels, ScrollWheelEvent, SharedString, StyledText, Task, Window, div, prelude::*, px,
};

use crate::ui::theme;

/// How long the pointer must rest on cropped text before its tooltip shows.
const SHOW_DELAY: Duration = Duration::from_millis(500);

/// State of a conditional tooltip, kept across frames (in element state or on
/// a view entity like [`super::text_input::TextInput`]).
#[derive(Clone, Default)]
pub struct TooltipState {
    active: Rc<RefCell<Option<Active>>>,
}

enum Active {
    Waiting {
        text: SharedString,
        _task: Task<()>,
    },
    Visible {
        text: SharedString,
        tooltip: AnyTooltip,
    },
}

impl Active {
    fn text(&self) -> &SharedString {
        match self {
            Self::Waiting { text, .. } | Self::Visible { text, .. } => text,
        }
    }
}

impl TooltipState {
    /// Call during `prepaint`. Re-submits an active tooltip to the window when
    /// `cropped` is still true and `text` hasn't changed; otherwise clears it.
    pub fn prepaint(&self, cropped: bool, text: &SharedString, window: &mut Window) {
        let stale = self
            .active
            .borrow()
            .as_ref()
            .is_some_and(|active| !cropped || active.text() != text);
        if stale {
            self.active.borrow_mut().take();
            return;
        }
        if let Some(Active::Visible { tooltip, .. }) = self.active.borrow().as_ref() {
            window.set_tooltip(tooltip.clone());
        }
    }

    /// Call during `paint` when the text is cropped. Schedules the tooltip on
    /// hover and dismisses it when the pointer leaves, clicks or scrolls.
    pub fn paint(
        &self,
        hitbox: Hitbox,
        visible_bounds: Bounds<Pixels>,
        text: SharedString,
        window: &mut Window,
    ) {
        let active = self.active.clone();
        window.on_mouse_event({
            let active = active.clone();
            move |_: &MouseMoveEvent, phase, window, cx| {
                let hovered = hitbox.is_hovered(window);
                enum Action {
                    None,
                    Schedule,
                    Cancel,
                    Hide,
                }
                let action = match active.borrow().as_ref() {
                    None if hovered && phase == DispatchPhase::Bubble => Action::Schedule,
                    Some(Active::Waiting { .. }) if !hovered => Action::Cancel,
                    Some(Active::Visible { .. }) if !hovered => Action::Hide,
                    _ => Action::None,
                };
                match action {
                    Action::None => {}
                    Action::Cancel => {
                        active.borrow_mut().take();
                    }
                    Action::Hide => {
                        active.borrow_mut().take();
                        window.refresh();
                    }
                    Action::Schedule => {
                        let task = window.spawn(cx, {
                            let active = active.clone();
                            let text = text.clone();
                            async move |cx| {
                                cx.background_executor().timer(SHOW_DELAY).await;
                                cx.update(|window, cx| {
                                    if !visible_bounds.contains(&window.mouse_position()) {
                                        active.borrow_mut().take();
                                        return;
                                    }
                                    let view = cx.new(|_| Tooltip { text: text.clone() }).into();
                                    let check_active = active.clone();
                                    let tooltip = AnyTooltip {
                                        view,
                                        mouse_position: window.mouse_position(),
                                        check_visible_and_update: Rc::new(move |_, window, _| {
                                            if visible_bounds.contains(&window.mouse_position()) {
                                                true
                                            } else {
                                                check_active.borrow_mut().take();
                                                false
                                            }
                                        }),
                                    };
                                    *active.borrow_mut() = Some(Active::Visible { text, tooltip });
                                    window.refresh();
                                })
                                .ok();
                            }
                        });
                        *active.borrow_mut() = Some(Active::Waiting {
                            text: text.clone(),
                            _task: task,
                        });
                    }
                }
            }
        });

        window.on_mouse_event({
            let active = active.clone();
            move |_: &MouseDownEvent, _, window, _| {
                if active.borrow_mut().take().is_some() {
                    window.refresh();
                }
            }
        });

        window.on_mouse_event(move |_: &ScrollWheelEvent, _, window, _| {
            if active.borrow_mut().take().is_some() {
                window.refresh();
            }
        });
    }
}

/// The tooltip popup itself: wraps long text to a readable width.
struct Tooltip {
    text: SharedString,
}

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::current(cx);
        let border_color = if t.is_bevel() {
            t.colors.bevel_dark
        } else {
            t.colors.border
        };
        t.apply_tooltip_shadow(
            div()
                .max_w(px(480.))
                .px_2()
                .py_1()
                .rounded(t.radius_sm())
                .bg(t.colors.tooltip)
                .border_1()
                .border_color(border_color)
                .text_size(px(12.))
                .line_height(px(17.))
                .text_color(t.colors.text)
                .when_some(t.font_family.clone(), |d, f| d.font_family(f))
                .child(self.text.clone()),
        )
    }
}

/// Text inside a `.truncate()` container that shows its full contents in a
/// tooltip when GPUI actually truncated it.
pub fn truncated_text(id: impl Into<ElementId>, text: impl Into<SharedString>) -> TruncatedText {
    let text = text.into();
    TruncatedText {
        id: id.into(),
        full_text: text.clone(),
        text: StyledText::new(text),
    }
}

pub struct TruncatedText {
    id: ElementId,
    full_text: SharedString,
    text: StyledText,
}

pub struct TruncatedTextPrepaint {
    hitbox: Option<Hitbox>,
    visible_bounds: Bounds<Pixels>,
    tooltip: TooltipState,
}

impl Element for TruncatedText {
    type RequestLayoutState = ();
    type PrepaintState = TruncatedTextPrepaint;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.text.request_layout(None, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.text
            .prepaint(None, inspector_id, bounds, state, window, cx);
        let visible_bounds = bounds.intersect(&window.content_mask().bounds);
        // GPUI's TextLayout can either ellipsize the text or, inside a flex
        // row where it cached the unconstrained measurement, paint the full
        // shaped line wider than `visible_bounds` (clipped by `overflow: hidden`).
        let layout = self.text.layout();
        let ellipsized = layout.text() != self.full_text.as_ref();
        let text_width = layout
            .line_layout_for_index(0)
            .map_or(px(0.), |line| line.width());
        let clipped = text_width > visible_bounds.size.width + px(0.5);
        let cropped = !visible_bounds.is_empty() && (ellipsized || clipped);
        let tooltip =
            window.with_element_state::<TooltipState, _>(global_id.unwrap(), |state, window| {
                let tooltip = state.unwrap_or_default();
                tooltip.prepaint(cropped, &self.full_text, window);
                (tooltip.clone(), tooltip)
            });
        let hitbox = cropped.then(|| window.insert_hitbox(visible_bounds, HitboxBehavior::Normal));
        TruncatedTextPrepaint {
            hitbox,
            visible_bounds,
            tooltip,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.text
            .paint(None, inspector_id, bounds, state, &mut (), window, cx);
        if let Some(hitbox) = prepaint.hitbox.take() {
            prepaint.tooltip.paint(
                hitbox,
                prepaint.visible_bounds,
                self.full_text.clone(),
                window,
            );
        }
    }
}

impl IntoElement for TruncatedText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}
