//! The image preview: an image attachment of the open issue, over the whole
//! window, scaled down to fit. It goes through the issue's images in
//! attachment order, and can show the file in the file manager.
//!
//! Key context: `ImagePreview` (outside `IssueDetail`, so the issue's
//! single-key bindings don't apply while it is open).

use gpui::{
    Action, AnyElement, ClickEvent, Context, FocusHandle, FontWeight, IntoElement, ObjectFit,
    StyledImage, Window, deferred, div, img, prelude::*, px,
};
use solilog_core::{Attachment, AttachmentId, AttachmentKind, format_size};

use super::super::Workspace;
use crate::ui::actions::attachment::{Preview, Reveal};
use crate::ui::actions::image_preview::{Close, Next, Previous};
use crate::ui::{SHOW_IN_FILE_MANAGER, theme};

pub const CONTEXT: &str = "ImagePreview";

pub(super) struct ImagePreview {
    /// The image shown.
    id: AttachmentId,
    /// Its index among the issue's images, to show a neighbor if it goes.
    index: usize,
    focus: FocusHandle,
}

impl Workspace {
    /// The open issue's image attachments, in order.
    fn preview_images(&self) -> Vec<Attachment> {
        self.detail
            .as_ref()
            .map(|d| {
                d.issue()
                    .attachments
                    .iter()
                    .filter(|a| a.kind == AttachmentKind::Image)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Keeps the preview on an image that still exists: the same one, else
    /// the one now at its place; closes it when the issue has none left.
    pub(in crate::ui::workspace) fn sync_image_preview(&mut self, window: &mut Window) {
        let images = self.preview_images();
        let Some(detail) = &mut self.detail else {
            return;
        };
        let Some(preview) = &mut detail.preview else {
            return;
        };
        if let Some(ix) = images.iter().position(|a| a.id == preview.id) {
            preview.index = ix;
        } else if let Some(last) = images.len().checked_sub(1) {
            preview.index = preview.index.min(last);
            preview.id = images[preview.index].id;
        } else {
            let had_focus = preview.focus.is_focused(window);
            detail.preview = None;
            if had_focus {
                window.focus(&detail.focus);
            }
        }
    }

    /// Shows the image `delta` places away, wrapping around.
    fn step_preview(&mut self, delta: isize, cx: &mut Context<Self>) {
        let images = self.preview_images();
        let Some(preview) = self.detail.as_mut().and_then(|d| d.preview.as_mut()) else {
            return;
        };
        if images.is_empty() {
            return;
        }
        let ix = (preview.index as isize + delta).rem_euclid(images.len() as isize) as usize;
        preview.index = ix;
        preview.id = images[ix].id;
        cx.notify();
    }

    fn previewed(&self) -> Option<Attachment> {
        let id = self.detail.as_ref()?.preview.as_ref()?.id;
        self.preview_images().into_iter().find(|a| a.id == id)
    }

    // ---- actions ------------------------------------------------------------

    /// Registered on the detail pane: the open issue's images.
    pub(super) fn on_preview_attachment(
        &mut self,
        action: &Preview,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let images = self.preview_images();
        let Some(detail) = &mut self.detail else {
            return;
        };
        let found = match action.id {
            Some(id) => images
                .iter()
                .position(|a| a.id == id)
                .ok_or_else(|| format!("{} has no image attachment {id}", detail.issue().id)),
            None if images.is_empty() => Err(format!("{} has no images", detail.issue().id)),
            None => Ok(0),
        };
        let index = match found {
            Ok(index) => index,
            Err(err) => {
                self.error = Some(err);
                cx.notify();
                return;
            }
        };
        let focus = cx.focus_handle();
        window.focus(&focus);
        detail.preview = Some(ImagePreview {
            id: images[index].id,
            index,
            focus,
        });
        cx.notify();
    }

    fn on_next_image(&mut self, _: &Next, _: &mut Window, cx: &mut Context<Self>) {
        self.step_preview(1, cx);
    }

    fn on_previous_image(&mut self, _: &Previous, _: &mut Window, cx: &mut Context<Self>) {
        self.step_preview(-1, cx);
    }

    fn on_close_preview(&mut self, _: &Close, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(detail) = &mut self.detail {
            detail.preview = None;
            window.focus(&detail.focus);
            cx.notify();
        }
    }

    /// `attachment::Reveal` from the preview: the image shown, unless given
    /// another one.
    fn on_reveal_previewed(&mut self, action: &Reveal, _: &mut Window, cx: &mut Context<Self>) {
        let attachment = match action.id {
            Some(id) => self.report(self.db.get_attachment(id), cx),
            None => self.previewed(),
        };
        if let Some(attachment) = attachment {
            self.reveal_attachment(&attachment, cx);
        }
    }

    // ---- rendering ------------------------------------------------------------

    pub(in crate::ui::workspace) fn render_image_preview(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let preview = self.detail.as_ref()?.preview.as_ref()?;
        let images = self.preview_images();
        let image = images.get(preview.index)?.clone();
        let path = image.path.clone()?;
        let focus = preview.focus.clone();
        // Buttons dispatch from the preview, so its handlers get the action.
        let t = theme::current(cx);
        let button = |id: &'static str, label: &'static str, action: Box<dyn Action>| {
            let focus = focus.clone();
            div()
                .id(id)
                .flex_none()
                .px_2()
                .rounded(t.radius_sm())
                .cursor_pointer()
                .text_color(t.colors.text_muted)
                .hover(|s| s.bg(t.colors.highlight).text_color(t.colors.highlight_text))
                .child(label)
                .on_click(move |_: &ClickEvent, window, cx| {
                    window.focus(&focus);
                    focus.dispatch_action(action.as_ref(), window, cx);
                })
        };
        let several = images.len() > 1;

        let header = div()
            .flex()
            .items_center()
            .gap_3()
            .px_4()
            .py_2()
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(image.name.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(12.))
                    .text_color(t.colors.text_muted)
                    .child(format_size(image.size)),
            )
            .when(several, |s| {
                s.child(
                    div()
                        .flex_none()
                        .text_size(px(12.))
                        .text_color(t.colors.text_muted)
                        .child(format!("{} / {}", preview.index + 1, images.len())),
                )
            })
            .child(div().flex_1())
            .child(button(
                "preview-reveal",
                SHOW_IN_FILE_MANAGER,
                Box::new(Reveal { id: Some(image.id) }),
            ))
            .child(button("preview-close", "×", Box::new(Close)));

        let side = |id: &'static str, label: &'static str, action: Box<dyn Action>| {
            div()
                .w(px(48.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .when(several, |s| {
                    s.child(button(id, label, action).text_size(px(28.)).py_2())
                })
        };
        let muted = t.colors.text_muted;
        let picture = img(path)
            .size_full()
            .object_fit(ObjectFit::ScaleDown)
            .with_fallback(move || fallback("Can't show this image", muted))
            .with_loading(move || fallback("", muted));

        let overlay = div()
            .id("image-preview")
            .key_context(CONTEXT)
            .track_focus(&focus)
            .on_action(cx.listener(Self::on_next_image))
            .on_action(cx.listener(Self::on_previous_image))
            .on_action(cx.listener(Self::on_close_preview))
            .on_action(cx.listener(Self::on_reveal_previewed))
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .flex_col()
            .bg(t.colors.overlay)
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .pb_4()
                    .child(side("preview-previous", "‹", Box::new(Previous)))
                    .child(div().flex_1().min_w_0().h_full().child(picture))
                    .child(side("preview-next", "›", Box::new(Next))),
            );
        Some(deferred(overlay))
    }
}

fn fallback(text: &'static str, muted: gpui::Hsla) -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .text_color(muted)
        .child(text)
        .into_any_element()
}
