//! The review handoff in the GUI: approving issues in review or sending
//! them back (`issue::Approve`, `issue::SendBack`), and the panel an open
//! issue in review shows above its description, with its latest receipt.

use gpui::{
    Action, AnyElement, ClickEvent, Context, FocusHandle, FontWeight, Hsla, IntoElement,
    SharedString, Window, div, prelude::*, px,
};
use solilog_core::{IssueId, Receipt, Status, format_age, latest_receipt};

use super::super::{CreateAction, Workspace};
use super::IssueDetail;
use crate::ui::actions::issue::{Approve, SendBack};
use crate::ui::theme;

/// The checks, commits and files of a receipt, one per line.
pub(super) fn receipt_details(receipt: &Receipt, t: &theme::Theme) -> impl IntoElement + use<> {
    let line = |mark: &'static str, color: Hsla, text: &str| {
        div()
            .flex()
            .gap_2()
            .text_size(px(12.))
            .line_height(px(18.))
            .child(div().flex_none().w(px(44.)).text_color(color).child(mark))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_color(t.colors.text)
                    .child(SharedString::from(text.to_owned())),
            )
    };
    let checks = receipt.checks.iter().map(|check| {
        if check.passed {
            line("✓ pass", t.colors.passed, &check.command)
        } else {
            line("✗ fail", t.colors.error, &check.command)
        }
    });
    let commits = receipt
        .commits
        .iter()
        .map(|c| line("commit", t.colors.text_muted, c));
    let files = receipt
        .files
        .iter()
        .map(|f| line("file", t.colors.text_muted, f));
    div()
        .flex()
        .flex_col()
        .children(checks)
        .children(commits)
        .children(files)
}

fn button(
    id: &'static str,
    label: &'static str,
    primary: bool,
    focus: &FocusHandle,
    t: &theme::Theme,
    action: Box<dyn Action>,
) -> impl IntoElement {
    let focus = focus.clone();
    div()
        .id(id)
        .relative()
        .px_2()
        .py(px(1.))
        .when(!t.is_bevel(), |s| {
            s.rounded(t.radius_sm()).border_1().border_color(if primary {
                t.colors.review
            } else {
                t.colors.border
            })
        })
        .when(t.is_bevel(), |s| s.bg(t.colors.surface))
        .text_size(px(12.))
        .text_color(t.colors.text)
        .cursor_pointer()
        .hover(|s| {
            if t.is_bevel() {
                s.bg(t.colors.hover)
            } else {
                s.bg(t.colors.highlight)
            }
        })
        .when(t.is_bevel(), |s| {
            s.active(|s| s.pt(px(2.)).pl(px(9.)))
        })
        .child(label)
        .children(t.raised_bevel())
        .children(t.gloss_overlay())
        .on_click(move |_: &ClickEvent, window, cx| {
            window.focus(&focus);
            window.dispatch_action(action.boxed_clone(), cx);
        })
}

impl Workspace {
    /// The target issues that are in review; an error is shown when there
    /// is none.
    fn target_issues_in_review(&mut self, cx: &mut Context<Self>) -> Vec<IssueId> {
        let ids: Vec<IssueId> = self
            .target_issues(cx)
            .into_iter()
            .filter(|i| i.status == Status::InReview)
            .map(|i| i.id)
            .collect();
        if ids.is_empty() && self.error.is_none() {
            self.error = Some("No issue in review selected".into());
            cx.notify();
        }
        ids
    }

    pub(in crate::ui::workspace) fn on_approve(
        &mut self,
        _: &Approve,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ids = self.target_issues_in_review(cx);
        if ids.is_empty() {
            return;
        }
        if self.report(self.db.approve_issues(&ids, ""), cx).is_some() {
            self.reload(cx);
            self.notice = Some(match ids.as_slice() {
                [id] => format!("Approved {id}"),
                many => format!("Approved {} issues", many.len()),
            });
            cx.notify();
        }
    }

    pub(in crate::ui::workspace) fn on_send_back(
        &mut self,
        action: &SendBack,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ids = if action.ids.is_empty() {
            self.target_issues_in_review(cx)
        } else {
            action.ids.clone()
        };
        if ids.is_empty() {
            return;
        }
        let Some(message) = &action.message else {
            let create: CreateAction = Box::new(move |message| {
                Box::new(SendBack {
                    ids: ids.clone(),
                    message: Some(message),
                })
            });
            return self.open_creatable_action_picker(
                "What should change?",
                Vec::new(),
                0,
                Some(("Send back", create)),
                window,
                cx,
            );
        };
        if self
            .report(self.db.send_back_issues(&ids, message), cx)
            .is_some()
        {
            self.reload(cx);
            self.notice = Some(match ids.as_slice() {
                [id] => format!("Sent {id} back"),
                many => format!("Sent {} issues back", many.len()),
            });
            cx.notify();
        }
    }

    /// Above the description of an issue in review: its latest receipt, and
    /// buttons to approve it or send it back.
    pub(super) fn render_review_panel(
        &self,
        detail: &IssueDetail,
        focus: &FocusHandle,
        t: &theme::Theme,
    ) -> Option<AnyElement> {
        if detail.issue().status != Status::InReview {
            return None;
        }
        let receipt = latest_receipt(detail.notes.all());
        let byline = receipt.map(|(note, _)| {
            let author = if note.author.is_user() {
                "you".to_owned()
            } else {
                note.author.to_string()
            };
            format!("from {author}, {}", format_age(note.created_at))
        });
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(t.colors.review)
                    .child("Ready for review"),
            )
            .children(byline.map(|b| {
                div()
                    .text_size(px(12.))
                    .text_color(t.colors.text_muted)
                    .child(b)
            }))
            .child(div().flex_1())
            .child(button(
                "review-send-back",
                "Send back…",
                false,
                focus,
                t,
                Box::new(SendBack::default()),
            ))
            .child(button(
                "review-approve",
                "Approve",
                true,
                focus,
                t,
                Box::new(Approve),
            ));
        let body = match receipt {
            Some((note, receipt)) => div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .line_height(px(22.))
                        .child(SharedString::from(note.text.clone())),
                )
                .child(receipt_details(receipt, t)),
            None => div()
                .text_size(px(12.))
                .text_color(t.colors.text_muted)
                .child("No receipt: set to In Review by hand."),
        };
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
                .border_color(t.colors.review)
                .child(header)
                .child(body)
                .children(t.raised_bevel_thin())
                .into_any_element(),
        )
    }
}
