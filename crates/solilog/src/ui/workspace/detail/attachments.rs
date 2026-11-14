//! The attachments of the open issue, below its description: one row per
//! file (an icon for its kind, its name and size). Clicking an image
//! previews it in the app (see `preview.rs`); clicking another file shows it
//! in the file manager. Files are added by dropping them on the open issue,
//! by pasting (files copied in the file manager, or an image), or from a file
//! dialog (`attachment::Add`).

use std::path::PathBuf;

use gpui::{
    Action, ClickEvent, Context, FontWeight, IntoElement, PathPromptOptions, Window, div,
    prelude::*, px, rgb,
};
use solilog_core::{Attachment, AttachmentId, AttachmentKind, IssueId, format_size};

use super::super::Workspace;
use super::IssueDetail;
use crate::ui::actions::attachment::{Add, Delete, Paste, Preview, Reveal};
use crate::ui::clipboard::{self, Attachable};
use crate::ui::picker::PickerItem;
use crate::ui::{SHOW_IN_FILE_MANAGER, theme};

/// Name of a pasted image, before its extension (numbered if taken).
const PASTED_IMAGE_NAME: &str = "Pasted image";

impl Workspace {
    /// The open issue's id and attachments.
    fn detail_attachments(&self) -> Option<(IssueId, Vec<Attachment>)> {
        let issue = self.detail.as_ref()?.issue();
        Some((issue.id, issue.attachments.clone()))
    }

    fn attach_files(&mut self, issue: IssueId, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if paths.is_empty() {
            return;
        }
        if self
            .report(self.db.add_attachments(issue, &paths), cx)
            .is_some()
        {
            self.reload(cx);
        }
    }

    /// The open issue's attachment with this id, else one picked with
    /// `pick` (each entry dispatching `action(id)`). `None` when a picker
    /// opened or there is nothing to pick.
    fn attachment_target(
        &mut self,
        id: Option<AttachmentId>,
        prompt: &str,
        action: impl Fn(AttachmentId) -> Box<dyn Action>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Attachment> {
        let (issue, attachments) = self.detail_attachments()?;
        if let Some(id) = id {
            return self.report(self.db.get_attachment(id), cx);
        }
        if attachments.is_empty() {
            self.error = Some(format!("{issue} has no attachments"));
            cx.notify();
            return None;
        }
        let entries = attachments
            .iter()
            .map(|a| {
                let item = PickerItem::new(a.name.clone()).hint(Some(format_size(a.size)));
                (item, action(a.id))
            })
            .collect();
        self.open_action_picker(prompt, entries, 0, window, cx);
        None
    }

    // ---- actions ------------------------------------------------------------

    pub(super) fn on_add_attachments(
        &mut self,
        action: &Add,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((issue, _)) = self.detail_attachments() else {
            return;
        };
        if !action.paths.is_empty() {
            return self.attach_files(issue, action.paths.clone(), cx);
        }
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let paths = match chosen.await {
                Ok(Ok(Some(paths))) => paths,
                Ok(Err(err)) => {
                    this.update(cx, |this, cx| this.report::<()>(Err(err), cx))
                        .ok();
                    return;
                }
                _ => return,
            };
            this.update(cx, |this, cx| this.attach_files(issue, paths, cx))
                .ok();
        })
        .detach();
    }

    pub(super) fn on_paste_attachments(
        &mut self,
        _: &Paste,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((issue, _)) = self.detail_attachments() else {
            return;
        };
        match clipboard::read(cx) {
            Some(Attachable::Files(paths)) => self.attach_files(issue, paths, cx),
            Some(Attachable::Image(data, ext)) => {
                let name = format!("{PASTED_IMAGE_NAME}.{ext}");
                let added = self.db.add_attachment_data(issue, &name, &data);
                if self.report(added, cx).is_some() {
                    self.reload(cx);
                }
            }
            None => {
                self.error = Some("There are no files or image on the clipboard".into());
                cx.notify();
            }
        }
    }

    pub(super) fn on_reveal_attachment(
        &mut self,
        action: &Reveal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let reveal = |id| Box::new(Reveal { id: Some(id) }) as Box<dyn Action>;
        let prompt = format!("{SHOW_IN_FILE_MANAGER}…");
        let Some(attachment) = self.attachment_target(action.id, &prompt, reveal, window, cx)
        else {
            return;
        };
        self.reveal_attachment(&attachment, cx);
    }

    /// Shows the file in the file manager, or an error if it is gone.
    pub(in crate::ui::workspace) fn reveal_attachment(
        &mut self,
        attachment: &Attachment,
        cx: &mut Context<Self>,
    ) {
        match &attachment.path {
            Some(path) if path.exists() => cx.reveal_path(path),
            Some(path) => {
                self.error = Some(format!("{} is missing", path.display()));
                cx.notify();
            }
            None => {}
        }
    }

    pub(super) fn on_delete_attachment(
        &mut self,
        action: &Delete,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delete = |id| {
            Box::new(Delete {
                id: Some(id),
                confirmed: false,
            }) as Box<dyn Action>
        };
        let Some(attachment) =
            self.attachment_target(action.id, "Delete attachment…", delete, window, cx)
        else {
            return;
        };
        if !action.confirmed {
            let item = PickerItem::new(format!("Delete “{}”", attachment.name))
                .hint(Some("deletes the file"));
            let delete = Delete {
                id: Some(attachment.id),
                confirmed: true,
            };
            return self.open_action_picker(
                &format!("Delete attachment “{}”?", attachment.name),
                vec![(item, Box::new(delete))],
                0,
                window,
                cx,
            );
        }
        if self
            .report(self.db.delete_attachment(attachment.id), cx)
            .is_some()
        {
            self.reload(cx);
        }
    }

    // ---- rendering ------------------------------------------------------------

    pub(super) fn render_attachments(&self, detail: &IssueDetail) -> impl IntoElement + use<> {
        let pane_focus = detail.focus.clone();
        let add = div()
            .id("attachments-add")
            .px_1()
            .rounded_sm()
            .cursor_pointer()
            .text_size(px(12.))
            .text_color(rgb(theme::TEXT_MUTED))
            .hover(|s| s.bg(rgb(theme::HIGHLIGHT)).text_color(rgb(theme::TEXT)))
            .child("Add files…")
            .on_click({
                let focus = pane_focus.clone();
                move |_: &ClickEvent, window, cx| {
                    window.focus(&focus);
                    window.dispatch_action(Add::default().boxed_clone(), cx);
                }
            });
        let header = div()
            .flex()
            .items_center()
            .gap_3()
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Attachments"))
            .child(add);

        let attachments = &detail.issue().attachments;
        let rows = attachments.iter().map(|attachment| {
            let id = attachment.id;
            let delete = div()
                .id(("attachment-delete", id.0 as u64))
                .flex_none()
                .px_1()
                .rounded_sm()
                .cursor_pointer()
                .invisible()
                .group_hover("attachment", |s| s.visible())
                .text_color(rgb(theme::TEXT_MUTED))
                .hover(|s| s.bg(rgb(theme::HIGHLIGHT)).text_color(rgb(theme::TEXT)))
                .child("×")
                .on_click({
                    let focus = pane_focus.clone();
                    move |_: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        window.focus(&focus);
                        let action = Delete {
                            id: Some(id),
                            confirmed: false,
                        };
                        window.dispatch_action(action.boxed_clone(), cx);
                    }
                });
            div()
                .id(("attachment", id.0 as u64))
                .group("attachment")
                .flex()
                .items_center()
                .gap_2()
                .px_1()
                .py(px(2.))
                .rounded_sm()
                .cursor_pointer()
                .hover(|s| s.bg(rgb(theme::HOVER)))
                .on_click({
                    let focus = pane_focus.clone();
                    let image = attachment.kind == AttachmentKind::Image;
                    move |_: &ClickEvent, window, cx| {
                        window.focus(&focus);
                        let action = if image {
                            Preview { id: Some(id) }.boxed_clone()
                        } else {
                            Reveal { id: Some(id) }.boxed_clone()
                        };
                        window.dispatch_action(action, cx);
                    }
                })
                .child(kind_icon(attachment.kind))
                .child(div().min_w_0().truncate().child(attachment.name.clone()))
                .child(
                    div()
                        .flex_none()
                        .text_size(px(12.))
                        .text_color(rgb(theme::TEXT_MUTED))
                        .child(format_size(attachment.size)),
                )
                .child(div().flex_1())
                .child(delete)
        });

        div()
            .mt_4()
            .flex()
            .flex_col()
            .gap_1()
            .line_height(px(24.))
            .child(header)
            .children(rows)
            .when(attachments.is_empty(), |s| {
                s.child(
                    div()
                        .px_1()
                        .text_size(px(12.))
                        .text_color(rgb(theme::TEXT_MUTED))
                        .child("Drop files here, or paste them"),
                )
            })
    }
}

/// A page with lines for a file; a frame with a sun and a hill for an image.
fn kind_icon(kind: AttachmentKind) -> impl IntoElement {
    let outline = || {
        div()
            .flex_none()
            .rounded(px(2.))
            .border_1()
            .border_color(rgb(theme::TEXT_MUTED))
    };
    let icon = match kind {
        AttachmentKind::File => outline()
            .w(px(12.))
            .h(px(15.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .pt(px(4.))
            .px(px(2.))
            .children((0..3).map(|_| div().h(px(1.)).bg(rgb(theme::TEXT_MUTED)))),
        AttachmentKind::Image => outline()
            .w(px(16.))
            .h(px(13.))
            .relative()
            .child(
                div()
                    .absolute()
                    .top(px(2.))
                    .right(px(2.))
                    .size(px(3.))
                    .rounded_full()
                    .bg(rgb(theme::ACCENT)),
            )
            .child(
                div()
                    .absolute()
                    .bottom(px(2.))
                    .left(px(2.))
                    .right(px(2.))
                    .h(px(3.))
                    .rounded(px(1.))
                    .bg(rgb(theme::ACCENT)),
            ),
    };
    div()
        .size(px(18.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .child(icon)
}
