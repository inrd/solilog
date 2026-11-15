//! A row of chips that gives way to its neighbors: the chips that don't fit
//! are left out and counted in a last "+N" chip.

use gpui::{
    AnyElement, App, AvailableSpace, Bounds, ContentMask, Element, ElementId, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, Pixels, Style, Window, point, px, size,
};

/// Space between chips.
const GAP: Pixels = px(12.);

/// Lays out `chips` side by side. The row takes their whole width when it
/// can, and shrinks otherwise; `more(n)` makes the chip standing for the `n`
/// that no longer fit.
pub(super) fn chip_row(
    chips: Vec<AnyElement>,
    more: impl Fn(usize) -> AnyElement + 'static,
) -> ChipRow {
    ChipRow {
        chips,
        more: Box::new(more),
    }
}

pub(super) struct ChipRow {
    chips: Vec<AnyElement>,
    more: Box<dyn Fn(usize) -> AnyElement>,
}

pub(super) struct Layout {
    chips: Vec<LayoutId>,
}

pub(super) struct Prepaint {
    /// How many chips, from the start, are shown.
    shown: usize,
    more: Option<AnyElement>,
}

impl IntoElement for ChipRow {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for ChipRow {
    type RequestLayoutState = Layout;
    type PrepaintState = Prepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Layout) {
        let chips: Vec<_> = self
            .chips
            .iter_mut()
            .map(|chip| chip.request_layout(window, cx))
            .collect();
        let mut style = Style {
            display: gpui::Display::Flex,
            align_items: Some(gpui::AlignItems::Center),
            gap: size(GAP.into(), GAP.into()),
            flex_shrink: 1.,
            ..Style::default()
        };
        // Without it, the row would never be narrower than its chips.
        style.min_size.width = px(0.).into();
        let id = window.request_layout(style, chips.iter().copied(), cx);
        (id, Layout { chips })
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Layout,
        window: &mut Window,
        cx: &mut App,
    ) -> Prepaint {
        let rights: Vec<Pixels> = layout
            .chips
            .iter()
            .map(|&id| window.layout_bounds(id).right())
            .collect();
        let fits = |right: Pixels| right <= bounds.right() + px(0.5);

        let mut shown = rights.len();
        let mut more = None;
        if !rights.last().is_none_or(|&right| fits(right)) {
            // Leave out chips from the end until the rest and "+N" fit.
            shown = rights.iter().take_while(|&&right| fits(right)).count();
            loop {
                let mut chip = (self.more)(rights.len() - shown);
                let chip_size = chip.layout_as_root(AvailableSpace::min_size(), window, cx);
                let left = match shown {
                    0 => bounds.left(),
                    n => rights[n - 1] + GAP,
                };
                if shown == 0 || fits(left + chip_size.width) {
                    let top = bounds.top() + (bounds.size.height - chip_size.height) / 2.;
                    more = Some((chip, point(left, top)));
                    break;
                }
                shown -= 1;
            }
        }

        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for chip in &mut self.chips[..shown] {
                chip.prepaint(window, cx);
            }
            let more = more.map(|(mut chip, origin)| {
                chip.prepaint_at(origin, window, cx);
                chip
            });
            Prepaint { shown, more }
        })
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Layout,
        prepaint: &mut Prepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for chip in &mut self.chips[..prepaint.shown] {
                chip.paint(window, cx);
            }
            if let Some(more) = &mut prepaint.more {
                more.paint(window, cx);
            }
        });
    }
}
