//! Editing the filter in effect (`filter::` actions) and showing it as chips
//! in the header.
//!
//! Every criterion is toggled by an action with an optional argument; without
//! one, a picker lists the choices, each dispatching the same action with the
//! argument filled in. Removing a chip dispatches the action too.

use gpui::{
    Action, AnyElement, ClickEvent, Context, FontWeight, IntoElement, SharedString, Window, div,
    prelude::*, px,
};
use solilog_core::{IssueFilter, Status};

use super::Workspace;
use super::scope::Source;
use crate::ui::actions::{filter, issue};
use crate::ui::picker::PickerItem;
use crate::ui::status_icon::status_icon;
use crate::ui::theme;

/// Adds `name` to `names`, or removes it if present (case-insensitive).
fn toggle(names: &mut Vec<String>, name: String) {
    match names.iter().position(|n| n.eq_ignore_ascii_case(&name)) {
        Some(ix) => {
            names.remove(ix);
        }
        None => names.push(name),
    }
}

fn contains(names: &[String], name: &str) -> bool {
    names.iter().any(|n| n.eq_ignore_ascii_case(name))
}

/// "✓" for items already in the filter.
fn check(on: bool) -> Option<&'static str> {
    on.then_some("✓")
}

/// One criterion of the filter, shown as a removable chip.
struct Criterion {
    text: String,
    /// Removes the criterion.
    remove: Box<dyn Action>,
}

fn criteria(filter: &IssueFilter) -> Vec<Criterion> {
    // Statuses and "open only" are shown by the status bar instead.
    let mut out = Vec::new();
    for label in &filter.labels {
        out.push(Criterion {
            text: format!("label: {label}"),
            remove: Box::new(filter::ToggleLabel {
                label: Some(label.clone()),
            }),
        });
    }
    for label in &filter.without_labels {
        out.push(Criterion {
            text: format!("not: {label}"),
            remove: Box::new(filter::ExcludeLabel {
                label: Some(label.clone()),
            }),
        });
    }
    for project in &filter.projects {
        out.push(Criterion {
            text: format!("project: {project}"),
            remove: Box::new(filter::ToggleProject {
                project: Some(project.clone()),
            }),
        });
    }
    if filter.no_project {
        out.push(Criterion {
            text: "no project".into(),
            remove: Box::new(filter::ToggleNoProject),
        });
    }
    if filter.ready {
        out.push(Criterion {
            text: "ready".into(),
            remove: Box::new(filter::ToggleReady),
        });
    }
    if filter.waiting {
        out.push(Criterion {
            text: "waiting".into(),
            remove: Box::new(filter::ToggleWaiting),
        });
    }
    if filter.unreviewed {
        out.push(Criterion {
            text: "unreviewed".into(),
            remove: Box::new(filter::ToggleUnreviewed),
        });
    }
    out
}

impl Workspace {
    /// Whether the filter differs from what the scope's source stands for.
    pub(super) fn filter_edited(&self) -> bool {
        self.scope.filter != self.scope.source.filter()
    }

    fn edit_filter(&mut self, edit: impl FnOnce(&mut IssueFilter), cx: &mut Context<Self>) {
        edit(&mut self.scope.filter);
        self.error = None;
        self.reload(cx);
    }

    pub(super) fn on_toggle_label_filter(
        &mut self,
        action: &filter::ToggleLabel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match &action.label {
            Some(name) => {
                let Some(label) = self.report(self.db.get_label(name), cx) else {
                    return;
                };
                self.edit_filter(|f| toggle(&mut f.labels, label.name), cx);
            }
            None => {
                let entries = self
                    .labels
                    .iter()
                    .map(|l| {
                        let on = contains(&self.scope.filter.labels, &l.name);
                        let action = filter::ToggleLabel {
                            label: Some(l.name.clone()),
                        };
                        (
                            PickerItem::new(l.name.clone()).hint(check(on)),
                            Box::new(action) as Box<dyn Action>,
                        )
                    })
                    .collect();
                self.open_action_picker("Filter by label…", entries, 0, window, cx);
            }
        }
    }

    pub(super) fn on_exclude_label_filter(
        &mut self,
        action: &filter::ExcludeLabel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match &action.label {
            Some(name) => {
                let Some(label) = self.report(self.db.get_label(name), cx) else {
                    return;
                };
                self.edit_filter(|f| toggle(&mut f.without_labels, label.name), cx);
            }
            None => {
                let entries = self
                    .labels
                    .iter()
                    .map(|l| {
                        let on = contains(&self.scope.filter.without_labels, &l.name);
                        let action = filter::ExcludeLabel {
                            label: Some(l.name.clone()),
                        };
                        (
                            PickerItem::new(l.name.clone()).hint(check(on)),
                            Box::new(action) as Box<dyn Action>,
                        )
                    })
                    .collect();
                self.open_action_picker("Exclude label…", entries, 0, window, cx);
            }
        }
    }

    pub(super) fn on_toggle_project_filter(
        &mut self,
        action: &filter::ToggleProject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match &action.project {
            Some(name) => {
                let Some(project) = self.report(self.db.get_project(name), cx) else {
                    return;
                };
                self.edit_filter(
                    |f| {
                        f.no_project = false;
                        toggle(&mut f.projects, project.name);
                    },
                    cx,
                );
            }
            None => {
                let entries = self
                    .projects
                    .iter()
                    .map(|p| {
                        let on = contains(&self.scope.filter.projects, &p.name);
                        let hint = check(on).or(p.status.is_closed().then(|| p.status.label()));
                        let action = filter::ToggleProject {
                            project: Some(p.name.clone()),
                        };
                        (
                            PickerItem::new(p.name.clone()).hint(hint),
                            Box::new(action) as Box<dyn Action>,
                        )
                    })
                    .collect();
                self.open_action_picker("Filter by project…", entries, 0, window, cx);
            }
        }
    }

    pub(super) fn on_toggle_status_filter(
        &mut self,
        action: &filter::ToggleStatus,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action.status {
            Some(status) => self.edit_filter(
                |f| match f.statuses.iter().position(|s| *s == status) {
                    Some(ix) => {
                        f.statuses.remove(ix);
                    }
                    None => {
                        f.statuses.push(status);
                        // Asking for a closed status means showing closed issues.
                        if status.is_closed() {
                            f.open_only = false;
                        }
                    }
                },
                cx,
            ),
            None => {
                let t = theme::current(cx);
                let entries = Status::ALL
                    .into_iter()
                    .map(|status| {
                        let on = self.scope.filter.statuses.contains(&status);
                        let t = t.clone();
                        let item = PickerItem::new(status.label())
                            .icon(move || status_icon(status, &t))
                            .hint(check(on));
                        let action = filter::ToggleStatus {
                            status: Some(status),
                        };
                        (item, Box::new(action) as Box<dyn Action>)
                    })
                    .collect();
                self.open_action_picker("Filter by status…", entries, 0, window, cx);
            }
        }
    }

    pub(super) fn on_toggle_open_only(
        &mut self,
        _: &filter::ToggleOpenOnly,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit_filter(
            |f| {
                f.open_only = !f.open_only;
                if f.open_only {
                    f.statuses.retain(|s| !s.is_closed());
                } else if !f.statuses.is_empty() {
                    // Closed issues come back alongside the chosen statuses.
                    for status in Status::ALL {
                        if status.is_closed() && !f.statuses.contains(&status) {
                            f.statuses.push(status);
                        }
                    }
                    if f.statuses.len() == Status::ALL.len() {
                        f.statuses.clear();
                    }
                }
            },
            cx,
        );
    }

    pub(super) fn on_toggle_no_project(
        &mut self,
        _: &filter::ToggleNoProject,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit_filter(
            |f| {
                f.no_project = !f.no_project;
                f.projects.clear();
            },
            cx,
        );
    }

    pub(super) fn on_toggle_ready(
        &mut self,
        _: &filter::ToggleReady,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit_filter(
            |f| {
                f.ready = !f.ready;
                f.waiting = false;
            },
            cx,
        );
    }

    pub(super) fn on_toggle_waiting(
        &mut self,
        _: &filter::ToggleWaiting,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit_filter(
            |f| {
                f.waiting = !f.waiting;
                f.ready = false;
            },
            cx,
        );
    }

    pub(super) fn on_toggle_unreviewed(
        &mut self,
        _: &filter::ToggleUnreviewed,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit_filter(|f| f.unreviewed = !f.unreviewed, cx);
    }

    pub(super) fn on_all_statuses(
        &mut self,
        _: &filter::AllStatuses,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit_filter(
            |f| {
                f.open_only = false;
                f.statuses.clear();
            },
            cx,
        );
    }

    pub(super) fn on_clear_filter(
        &mut self,
        _: &filter::Clear,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit_filter(|f| *f = IssueFilter::default(), cx);
    }

    /// Scope title (but a project's), filter chips (click × to remove), status bar, issue count.
    pub(super) fn render_filter_header(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let edited = self.filter_edited();
        let title = self.scope.source.title().to_string();
        let chips: Vec<AnyElement> = criteria(&self.scope.filter)
            .into_iter()
            .enumerate()
            .map(|(ix, criterion)| filter_chip(ix, criterion, cx))
            .collect();
        let has_unreviewed = matches!(self.scope.source, Source::Unreviewed)
            && self
                .all_issues
                .iter()
                .any(|i| i.unreviewed && self.scope.filter.matches(i));
        let t = theme::current(cx);
        div()
            .relative()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .px_4()
            .py_1()
            .when(t.is_bevel() || t.is_glass(), |s| s.bg(t.colors.surface))
            .border_b_1()
            .border_color(t.colors.divider)
            .text_size(px(12.))
            .children(t.glass_bar_overlay())
            // A project's name is in the project header already.
            .when(!matches!(self.scope.source, Source::Project(_)), |s| {
                s.child(div().font_weight(FontWeight::SEMIBOLD).child(title))
            })
            .when(edited && self.scope.source != Source::All, |s| {
                s.child(div().text_color(t.colors.text_muted).child("edited"))
            })
            .children(chips)
            .child(status_bar(&self.scope.filter, cx))
            .when(!self.selection.is_empty(), |s| {
                s.child(
                    div()
                        .flex_none()
                        .text_color(t.colors.accent)
                        .child(format!("{} selected", self.selection.len())),
                )
            })
            .child(
                div().flex_none().text_color(t.colors.text_muted).child(
                    self.sections
                        .iter()
                        .map(|(_, n)| n)
                        .sum::<usize>()
                        .to_string(),
                ),
            )
            .when(has_unreviewed, |s| {
                s.child(
                    div()
                        .flex_none()
                        .w(px(1.))
                        .h(px(16.))
                        .mx_1()
                        .bg(t.colors.divider),
                )
                .child(mark_all_reviewed_button(cx))
            })
    }
}

fn filter_chip(ix: usize, criterion: Criterion, cx: &mut Context<Workspace>) -> AnyElement {
    let t = theme::current(cx);
    let Criterion { text, remove } = criterion;
    div()
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .pl_2()
        .pr_1()
        .when(t.is_bevel(), |s| s.bg(t.colors.surface))
        .when(!t.is_bevel(), |s| {
            s.rounded_full().border_1().border_color(t.colors.border)
        })
        .line_height(px(18.))
        .child(SharedString::from(text))
        .child(
            div()
                .id(("filter-chip", ix))
                .px_1()
                .when(!t.is_bevel(), |s| s.rounded_full())
                .cursor_pointer()
                .text_color(t.colors.text_muted)
                .hover(|s| s.bg(t.colors.highlight).text_color(t.colors.highlight_text))
                .on_click(cx.listener(move |_, _: &ClickEvent, window, cx| {
                    window.dispatch_action(remove.boxed_clone(), cx);
                }))
                .child("×"),
        )
        .children(t.raised_bevel_thin())
        .into_any_element()
}

/// "All" and "Open", plus one button per status, each toggling that part of
/// the filter.
/// A status whose issues are filtered out is dimmed; one the filter asks for
/// is highlighted.
fn status_bar(filter: &IssueFilter, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let t = theme::current(cx);
    let every_status = !filter.open_only && filter.statuses.is_empty();
    let all = toggle_pill(
        "filter-all-statuses",
        "All",
        every_status,
        Box::new(filter::AllStatuses),
        cx,
    );
    let open = toggle_pill(
        "filter-open-only",
        "Open",
        filter.open_only,
        Box::new(filter::ToggleOpenOnly),
        cx,
    );
    let statuses = Status::ALL.into_iter().map(|status| {
        let chosen = filter.statuses.contains(&status);
        let shown =
            (filter.statuses.is_empty() || chosen) && !(filter.open_only && status.is_closed());
        div()
            .id(SharedString::from(format!(
                "filter-status-{}",
                status.key()
            )))
            .relative()
            .size(px(20.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(t.radius_sm())
            .cursor_pointer()
            .when(chosen && !t.is_bevel(), |s| s.bg(t.colors.highlight))
            .when(chosen && t.is_bevel(), |s| s.bg(t.colors.hover))
            .when(!shown, |s| s.opacity(0.35))
            .hover(|s| s.bg(t.colors.hover).opacity(1.))
            .on_click(cx.listener(move |_, _: &ClickEvent, window, cx| {
                let action = filter::ToggleStatus {
                    status: Some(status),
                };
                window.dispatch_action(action.boxed_clone(), cx);
            }))
            .child(status_icon(status, &t))
            .when(chosen, |s| s.children(t.sunken_bevel_thin()))
    });
    div()
        .ml_auto()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .child(all)
        .child(open)
        .children(statuses)
}

/// A small text button, highlighted when `on`, that dispatches `action`.
fn toggle_pill(
    id: &'static str,
    text: &'static str,
    on: bool,
    action: Box<dyn Action>,
    cx: &mut Context<Workspace>,
) -> impl IntoElement + use<> {
    let t = theme::current(cx);
    div()
        .id(id)
        .relative()
        .flex_none()
        .px_2()
        .when(!t.is_bevel(), |s| {
            s.rounded_full().border_1().border_color(t.colors.border)
        })
        .line_height(px(18.))
        .cursor_pointer()
        .map(|s| {
            if t.is_bevel() {
                if on {
                    s.bg(t.colors.hover)
                        .text_color(t.colors.text)
                        .children(t.sunken_bevel_thin())
                } else {
                    s.bg(t.colors.surface)
                        .text_color(t.colors.text)
                        .children(t.raised_bevel_thin())
                }
            } else if on {
                s.bg(t.colors.highlight)
                    .text_color(t.colors.highlight_text)
                    .children(t.gloss_overlay())
            } else {
                s.text_color(t.colors.text_muted)
            }
        })
        .hover(|s| s.text_color(t.colors.text))
        .on_click(cx.listener(move |_, _: &ClickEvent, window, cx| {
            window.dispatch_action(action.boxed_clone(), cx);
        }))
        .child(text)
}

fn mark_all_reviewed_button(cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let t = theme::current(cx);
    div()
        .id("mark-all-reviewed")
        .relative()
        .flex_none()
        .flex()
        .items_center()
        .gap_1()
        .px(px(10.))
        .py(px(2.))
        .bg(t.colors.surface)
        .when(!t.is_bevel(), |s| {
            s.rounded(t.radius_sm())
                .border_1()
                .border_color(t.colors.border)
        })
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.colors.text)
        .cursor_pointer()
        .hover(|s| {
            if t.is_bevel() {
                s.bg(t.colors.hover)
            } else {
                s.bg(t.colors.highlight).border_color(t.colors.accent)
            }
        })
        .when(t.is_bevel(), |s| {
            s.active(|s| s.pt(px(3.)).pl(px(11.)))
        })
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            window.focus(&this.list_focus);
            window.dispatch_action(issue::MarkAllReviewed::default().boxed_clone(), cx);
        }))
        .child(div().text_color(t.colors.accent).child("✓"))
        .child("Mark all reviewed")
        .children(t.raised_bevel())
        .children(t.gloss_overlay())
}
