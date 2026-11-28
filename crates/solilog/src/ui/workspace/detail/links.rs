//! The links of the open issue, in its side pane: where its work lives
//! outside Solilog (a branch, a pull request, other pages), one row each
//! (a titled one with its target below).
//! Clicking a URL opens it in the browser; clicking a branch copies its
//! name (`link::Open`). Adding and removing go through `link::Add` /
//! `link::Remove`, which act on the target issues, so a link can be added
//! to several selected issues at once.

use gpui::{Action, ClickEvent, ClipboardItem, Context, IntoElement, Window, div, prelude::*, px};
use solilog_core::{IssueChange, Link, LinkId, LinkKind, NewLink, normalize_link};

use super::super::Workspace;
use super::super::section::{section_action, section_count, section_header};
use super::IssueDetail;
use crate::ui::actions::link::{Add, Copy, Open, Remove};
use crate::ui::picker::PickerItem;
use crate::ui::theme;
use crate::ui::tooltip::truncated_text;

/// Short name of a kind, for rows and pickers.
fn kind_tag(kind: LinkKind) -> &'static str {
    match kind {
        LinkKind::Branch => "branch",
        LinkKind::Pr => "PR",
        LinkKind::Url => "link",
    }
}

impl Workspace {
    /// The link with this id, else one picked among the target issues'
    /// links (each entry dispatching `action(id)`). `None` when a picker
    /// opened or there is nothing to pick.
    fn link_target(
        &mut self,
        id: Option<LinkId>,
        prompt: &str,
        action: impl Fn(LinkId) -> Box<dyn Action>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Link> {
        if let Some(id) = id {
            return self.report(self.db.get_link(id), cx);
        }
        let issues = self.target_issues(cx);
        if issues.is_empty() {
            return None;
        }
        let links: Vec<&Link> = issues.iter().flat_map(|i| &i.links).collect();
        match links.as_slice() {
            [] => {
                self.footer.error = Some(match issues.as_slice() {
                    [issue] => format!("{} has no links", issue.id),
                    _ => "The selected issues have no links".into(),
                });
                cx.notify();
                None
            }
            [link] => Some((*link).clone()),
            _ => {
                let several = issues.len() > 1;
                let entries = links
                    .iter()
                    .map(|l| {
                        let label = if several {
                            format!("{}  {}", l.issue_id, l.display_text())
                        } else {
                            l.display_text().to_owned()
                        };
                        (
                            PickerItem::new(label).hint(Some(kind_tag(l.kind))),
                            action(l.id),
                        )
                    })
                    .collect();
                self.open_action_picker(prompt, entries, 0, window, cx);
                None
            }
        }
    }

    // ---- actions ------------------------------------------------------------

    pub(in crate::ui::workspace) fn on_add_link(
        &mut self,
        action: &Add,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let issues = self.target_issues(cx);
        if issues.is_empty() {
            return;
        }
        let ids: Vec<_> = issues.iter().map(|i| i.id).collect();
        if let Some(target) = &action.target {
            let change = IssueChange::AddLinks(vec![NewLink::new(target.clone())]);
            return self.update_issues(&ids, change, cx);
        }
        // A branch or URL on the clipboard is offered first.
        let pasted = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .and_then(|text| normalize_link(&NewLink::new(text)).ok())
            .filter(|(kind, target, _)| {
                !issues.iter().all(|i| {
                    i.links
                        .iter()
                        .any(|l| l.kind == *kind && l.target == *target)
                })
            });
        let entries = pasted
            .into_iter()
            .map(|(kind, target, _)| {
                let item = PickerItem::new(format!("Paste “{target}”")).hint(Some(kind_tag(kind)));
                let add = Add {
                    target: Some(target),
                };
                (item, add.boxed_clone())
            })
            .collect();
        let create: super::super::CreateAction = Box::new(|target| {
            Box::new(Add {
                target: Some(target),
            })
        });
        let placeholder = format!("Branch or URL to link{}…", super::super::of_issues(&issues));
        self.open_creatable_action_picker(
            &placeholder,
            entries,
            0,
            Some(("Add link", create)),
            window,
            cx,
        );
    }

    pub(in crate::ui::workspace) fn on_open_link(
        &mut self,
        action: &Open,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let open = |id| Box::new(Open { id: Some(id) }) as Box<dyn Action>;
        let Some(link) = self.link_target(action.id, "Open link…", open, window, cx) else {
            return;
        };
        if link.is_url() {
            cx.open_url(&link.target);
        } else {
            self.copy_link(&link, cx);
        }
    }

    pub(in crate::ui::workspace) fn on_copy_link(
        &mut self,
        action: &Copy,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let copy = |id| Box::new(Copy { id: Some(id) }) as Box<dyn Action>;
        if let Some(link) = self.link_target(action.id, "Copy link…", copy, window, cx) {
            self.copy_link(&link, cx);
        }
    }

    fn copy_link(&mut self, link: &Link, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(link.target.clone()));
        self.footer.error = None;
        self.footer.notice = Some(format!("Copied {}", link.target));
        cx.notify();
    }

    pub(in crate::ui::workspace) fn on_remove_link(
        &mut self,
        action: &Remove,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let remove = |id| Box::new(Remove { id: Some(id) }) as Box<dyn Action>;
        let Some(link) = self.link_target(action.id, "Remove link…", remove, window, cx) else {
            return;
        };
        if self.report(self.db.remove_link(link.id), cx).is_some() {
            self.reload(cx);
        }
    }

    // ---- rendering ------------------------------------------------------------

    pub(super) fn render_links(
        &self,
        detail: &IssueDetail,
        t: &theme::Theme,
    ) -> impl IntoElement + use<> {
        let pane_focus = detail.focus.clone();
        let links = &detail.issue().links;
        let add = section_action(
            "links-add",
            "+ Add",
            Add::default().boxed_clone(),
            Some(pane_focus.clone()),
            t,
        );
        let header = section_header(
            "Links",
            section_count(links.len(), t),
            [add.into_any_element()],
        );
        let rows = links.iter().map(|link| {
            let id = link.id;
            let remove = div()
                .id(("link-remove", id.0 as u64))
                .flex_none()
                .px_1()
                .rounded(t.radius_sm())
                .cursor_pointer()
                .invisible()
                .group_hover("link", |s| s.visible())
                .text_color(t.colors.text_muted)
                .hover(|s| s.bg(t.colors.highlight).text_color(t.colors.highlight_text))
                .child("×")
                .on_click({
                    let focus = pane_focus.clone();
                    move |_: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        window.focus(&focus);
                        window.dispatch_action(Remove { id: Some(id) }.boxed_clone(), cx);
                    }
                });
            // Titled links show their target too, muted, below the title
            // (lined up with it): the pane is too narrow for both on a line.
            let target = link.title.is_some().then(|| {
                div().flex().pl(px(44. + 8.)).child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(12.))
                        .line_height(px(16.))
                        .text_color(t.colors.text_muted)
                        .child(truncated_text(
                            ("link-target", id.0 as u64),
                            link.target.clone(),
                        )),
                )
            });
            let line = div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_none()
                        .w(px(44.))
                        .text_size(px(12.))
                        .text_color(t.colors.text_muted)
                        .child(kind_tag(link.kind)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .when(link.is_url(), |s| s.text_color(t.colors.accent))
                        .child(truncated_text(
                            ("link-text", id.0 as u64),
                            link.display_text().to_owned(),
                        )),
                )
                .child(remove);
            div()
                .id(("link", id.0 as u64))
                .group("link")
                .flex()
                .flex_col()
                .px_1()
                .py(px(2.))
                .rounded(t.radius_sm())
                .cursor_pointer()
                .hover(|s| s.bg(t.colors.hover))
                .on_click({
                    let focus = pane_focus.clone();
                    move |_: &ClickEvent, window, cx| {
                        window.focus(&focus);
                        window.dispatch_action(Open { id: Some(id) }.boxed_clone(), cx);
                    }
                })
                .child(line)
                .children(target)
        });

        div()
            .flex()
            .flex_col()
            .gap_1()
            .line_height(px(24.))
            .child(header)
            .children(rows)
            .when(links.is_empty(), |s| {
                s.child(
                    div()
                        .px_1()
                        .text_size(px(12.))
                        .text_color(t.colors.text_muted)
                        .child("The branch, pull request or pages of this work"),
                )
            })
    }
}
