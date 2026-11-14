//! Editing the filter in effect (`filter::` actions) and showing it as chips
//! in the header.
//!
//! Every criterion is toggled by an action with an optional argument; without
//! one, a picker lists the choices, each dispatching the same action with the
//! argument filled in. Removing a chip dispatches the action too.

use gpui::{
    Action, AnyElement, ClickEvent, Context, FontWeight, IntoElement, SharedString, Window, div,
    prelude::*, px, rgb,
};
use solilog_core::{IssueFilter, Status};

use super::Workspace;
use super::scope::Source;
use crate::ui::actions::filter;
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
                let entries = Status::ALL
                    .into_iter()
                    .map(|status| {
                        let on = self.scope.filter.statuses.contains(&status);
                        let item = PickerItem::new(status.label())
                            .icon(move || status_icon(status))
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

    /// Scope title, filter chips (click × to remove), status bar, issue count.
    pub(super) fn render_filter_header(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let edited = self.filter_edited();
        let title = self.scope.source.title().to_string();
        let chips: Vec<AnyElement> = criteria(&self.scope.filter)
            .into_iter()
            .enumerate()
            .map(|(ix, criterion)| filter_chip(ix, criterion, cx))
            .collect();
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .px_4()
            .py_1()
            .border_b_1()
            .border_color(rgb(theme::DIVIDER))
            .text_size(px(12.))
            .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
            .when(edited && self.scope.source != Source::All, |s| {
                s.child(div().text_color(rgb(theme::TEXT_MUTED)).child("edited"))
            })
            .children(chips)
            .child(status_bar(&self.scope.filter, cx))
            .child(
                div()
                    .flex_none()
                    .text_color(rgb(theme::TEXT_MUTED))
                    .child(self.issues.len().to_string()),
            )
    }
}

fn filter_chip(ix: usize, criterion: Criterion, cx: &mut Context<Workspace>) -> AnyElement {
    let Criterion { text, remove } = criterion;
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .pl_2()
        .pr_1()
        .rounded_full()
        .border_1()
        .border_color(rgb(theme::BORDER))
        .line_height(px(18.))
        .child(SharedString::from(text))
        .child(
            div()
                .id(("filter-chip", ix))
                .px_1()
                .rounded_full()
                .cursor_pointer()
                .text_color(rgb(theme::TEXT_MUTED))
                .hover(|s| s.bg(rgb(theme::HIGHLIGHT)).text_color(rgb(theme::TEXT)))
                .on_click(cx.listener(move |_, _: &ClickEvent, window, cx| {
                    window.dispatch_action(remove.boxed_clone(), cx);
                }))
                .child("×"),
        )
        .into_any_element()
}

/// "All" and "Open", plus one button per status, each toggling that part of
/// the filter.
/// A status whose issues are filtered out is dimmed; one the filter asks for
/// is highlighted.
fn status_bar(filter: &IssueFilter, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
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
        let shown = (filter.statuses.is_empty() || chosen)
            && !(filter.open_only && status.is_closed());
        div()
            .id(SharedString::from(format!("filter-status-{}", status.key())))
            .size(px(20.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded_sm()
            .cursor_pointer()
            .when(chosen, |s| s.bg(rgb(theme::HIGHLIGHT)))
            .when(!shown, |s| s.opacity(0.35))
            .hover(|s| s.bg(rgb(theme::HIGHLIGHT)).opacity(1.))
            .on_click(cx.listener(move |_, _: &ClickEvent, window, cx| {
                let action = filter::ToggleStatus {
                    status: Some(status),
                };
                window.dispatch_action(action.boxed_clone(), cx);
            }))
            .child(status_icon(status))
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
    div()
        .id(id)
        .flex_none()
        .px_2()
        .rounded_full()
        .border_1()
        .border_color(rgb(theme::BORDER))
        .line_height(px(18.))
        .cursor_pointer()
        .map(|s| {
            if on {
                s.bg(rgb(theme::HIGHLIGHT)).text_color(rgb(theme::TEXT))
            } else {
                s.text_color(rgb(theme::TEXT_MUTED))
            }
        })
        .hover(|s| s.text_color(rgb(theme::TEXT)))
        .on_click(cx.listener(move |_, _: &ClickEvent, window, cx| {
            window.dispatch_action(action.boxed_clone(), cx);
        }))
        .child(text)
}
