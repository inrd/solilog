//! Decisions in the GUI: those that apply to the open issue (above its
//! attachments), or to the label or project the issue list shows (in its
//! header), with what an agent withdrew still awaiting review. Recording,
//! replacing, withdrawing and confirming go through `decision::*` actions.

use gpui::{
    Action, AnyElement, ClickEvent, Context, FontWeight, IntoElement, SharedString, Window, div,
    prelude::*, px,
};
use solilog_core::{Decision, DecisionId, DecisionOwner, DecisionState, format_age};

use super::{CreateAction, Workspace};
use crate::ui::actions::decision::{MarkReviewed, Record, Replace, Withdraw};
use crate::ui::picker::PickerItem;
use crate::ui::theme;

/// How much of a decision pickers show.
const PREVIEW_CHARS: usize = 60;

/// A decision on one line, shortened.
fn preview(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= PREVIEW_CHARS {
        return line;
    }
    let cut: String = line.chars().take(PREVIEW_CHARS - 1).collect();
    format!("{}…", cut.trim_end())
}

/// The owner, as a `Record` action recording on it.
fn record_on(owner: &DecisionOwner, text: Option<String>) -> Record {
    let mut record = Record {
        text,
        ..Default::default()
    };
    match owner {
        DecisionOwner::Issue { id } => record.issue = Some(*id),
        DecisionOwner::Label { name } => record.label = Some(name.clone()),
        DecisionOwner::Project { name } => record.project = Some(name.clone()),
    }
    record
}

/// `SOL-12`, `label acme`, `project v2`.
fn owner_name(owner: &DecisionOwner) -> String {
    owner.to_string()
}

impl Workspace {
    /// What decision commands act on: the open issue, else the label or
    /// project the list shows.
    fn decision_target(&self) -> Option<DecisionOwner> {
        if let Some(detail) = &self.detail {
            return Some(DecisionOwner::issue(detail.issue().id));
        }
        if let Some(label) = self.shown_label() {
            return Some(DecisionOwner::label(&label.name));
        }
        self.shown_project()
            .map(|p| DecisionOwner::project(&p.name))
    }

    /// The decisions shown for the target, as last read.
    fn target_decisions(&self) -> &[Decision] {
        match &self.detail {
            Some(detail) => detail.decisions(),
            None => &self.scope_decisions,
        }
    }

    /// Re-reads the decisions shown with the label or project the list
    /// shows (the open issue's are read with it).
    pub(super) fn refresh_scope_decisions(&mut self) {
        let owner = if let Some(label) = self.shown_label() {
            Some(DecisionOwner::label(&label.name))
        } else {
            self.shown_project()
                .map(|p| DecisionOwner::project(&p.name))
        };
        self.scope_decisions = owner
            .and_then(|owner| self.db.shown_decisions(&owner).ok())
            .unwrap_or_default();
    }

    /// Opens a picker of the shown decisions that `keep` accepts, each
    /// dispatching `action(id)`; an error says so when there is none.
    fn pick_decision(
        &mut self,
        placeholder: &str,
        keep: impl Fn(&Decision) -> bool,
        action: impl Fn(DecisionId) -> Box<dyn Action>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let entries: Vec<(PickerItem, Box<dyn Action>)> = self
            .target_decisions()
            .iter()
            .filter(|d| keep(d))
            .map(|d| {
                let item = PickerItem::new(preview(&d.text)).hint(Some(format!(
                    "{}, {}",
                    owner_name(&d.owner),
                    format_age(d.created_at)
                )));
                (item, action(d.id))
            })
            .collect();
        if entries.is_empty() {
            self.error = Some("No such decision here".into());
            cx.notify();
            return;
        }
        self.open_action_picker(placeholder, entries, 0, window, cx);
    }

    // ---- actions ------------------------------------------------------------

    pub(super) fn on_record_decision(
        &mut self,
        action: &Record,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let owner = if let Some(id) = action.issue {
            DecisionOwner::issue(id)
        } else if let Some(name) = &action.label {
            DecisionOwner::label(name)
        } else if let Some(name) = &action.project {
            DecisionOwner::project(name)
        } else if let Some(detail) = &self.detail {
            // On the open issue, one of its projects, or one of its labels
            // (or their ancestors).
            let issue = detail.issue();
            let mut owners = vec![DecisionOwner::issue(issue.id)];
            owners.extend(issue.projects.iter().map(DecisionOwner::project));
            let mut labels: Vec<String> = Vec::new();
            for label in &issue.labels {
                let mut name = label.as_str();
                loop {
                    if !labels.iter().any(|l| l == name) {
                        labels.push(name.to_owned());
                    }
                    match name.rsplit_once(solilog_core::LABEL_SEPARATOR) {
                        Some((parent, _)) => name = parent,
                        None => break,
                    }
                }
            }
            owners.extend(labels.into_iter().map(DecisionOwner::label));
            if owners.len() > 1 {
                let entries = owners
                    .iter()
                    .map(|owner| {
                        let item = PickerItem::new(owner_name(owner));
                        let record = record_on(owner, action.text.clone());
                        (item, Box::new(record) as Box<dyn Action>)
                    })
                    .collect();
                return self.open_action_picker("Record a decision on…", entries, 0, window, cx);
            }
            owners.remove(0)
        } else if let Some(owner) = self.decision_target() {
            owner
        } else {
            self.error = Some("Open an issue, a label or a project to record a decision".into());
            cx.notify();
            return;
        };
        let Some(text) = &action.text else {
            let create: CreateAction = {
                let owner = owner.clone();
                Box::new(move |text| Box::new(record_on(&owner, Some(text))))
            };
            return self.open_creatable_action_picker(
                &format!("Decision on {}…", owner_name(&owner)),
                Vec::new(),
                0,
                Some(("Record", create)),
                window,
                cx,
            );
        };
        if self
            .report(self.db.record_decision(&owner, text), cx)
            .is_some()
        {
            self.reload(cx);
        }
    }

    pub(super) fn on_replace_decision(
        &mut self,
        action: &Replace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = action.id else {
            let text = action.text.clone();
            return self.pick_decision(
                "Replace decision…",
                Decision::in_force,
                |id| {
                    Box::new(Replace {
                        id: Some(id),
                        text: text.clone(),
                    })
                },
                window,
                cx,
            );
        };
        let Some(text) = &action.text else {
            let create: CreateAction = Box::new(move |text| {
                Box::new(Replace {
                    id: Some(id),
                    text: Some(text),
                })
            });
            return self.open_creatable_action_picker(
                "Replace it with…",
                Vec::new(),
                0,
                Some(("Replace with", create)),
                window,
                cx,
            );
        };
        if self
            .report(self.db.replace_decision(id, text), cx)
            .is_some()
        {
            self.reload(cx);
        }
    }

    pub(super) fn on_withdraw_decision(
        &mut self,
        action: &Withdraw,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = action.id else {
            return self.pick_decision(
                "Withdraw decision…",
                Decision::in_force,
                |id| {
                    Box::new(Withdraw {
                        id: Some(id),
                        confirmed: false,
                    })
                },
                window,
                cx,
            );
        };
        if !action.confirmed {
            let Some(decision) = self.report(self.db.get_decision(id), cx) else {
                return;
            };
            let item = PickerItem::new(format!("Withdraw “{}”", preview(&decision.text)))
                .hint(Some("it no longer applies"));
            let withdraw = Withdraw {
                id: Some(id),
                confirmed: true,
            };
            return self.open_action_picker(
                "Withdraw this decision?",
                vec![(item, Box::new(withdraw))],
                0,
                window,
                cx,
            );
        }
        if self.report(self.db.withdraw_decision(id), cx).is_some() {
            self.reload(cx);
        }
    }

    pub(super) fn on_mark_decision_reviewed(
        &mut self,
        action: &MarkReviewed,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ids: Vec<DecisionId> = match action.id {
            Some(id) => vec![id],
            None => self
                .target_decisions()
                .iter()
                .filter(|d| !d.reviewed)
                .map(|d| d.id)
                .collect(),
        };
        if ids.is_empty() {
            self.error = Some("No unreviewed decision here".into());
            cx.notify();
            return;
        }
        if self
            .report(self.db.mark_decisions_reviewed(&ids), cx)
            .is_some()
        {
            self.reload(cx);
        }
    }

    // ---- rendering ------------------------------------------------------------

    /// A "+ Decision" chip for a header's chip row, recording one on
    /// `here`; nothing once it has decisions (their list has a button).
    pub(super) fn record_decision_chip(
        &self,
        here: &DecisionOwner,
        t: &theme::Theme,
    ) -> Option<AnyElement> {
        if !self.scope_decisions.is_empty() {
            return None;
        }
        let action = record_on(here, None);
        Some(
            div()
                .id("header-decision-record")
                .relative()
                .flex_none()
                .px_2()
                .when(t.is_bevel(), |s| s.bg(t.colors.surface))
                .when(!t.is_bevel(), |s| {
                    s.rounded_full()
                        .border_1()
                        .border_dashed()
                        .border_color(t.colors.border)
                })
                .line_height(px(18.))
                .text_color(t.colors.text_muted)
                .cursor_pointer()
                .hover(|s| s.bg(t.colors.hover).text_color(t.colors.text))
                .child("+ Decision")
                .children(t.raised_bevel_thin())
                .on_click(move |_: &ClickEvent, window, cx| {
                    window.dispatch_action(action.boxed_clone(), cx);
                })
                .into_any_element(),
        )
    }

    /// The decisions shown for `here` (the open issue, or the label or
    /// project shown), with a header and a button recording one. `None`
    /// when there are none and `always` is false.
    pub(super) fn render_decisions(
        &self,
        decisions: &[Decision],
        here: &DecisionOwner,
        always: bool,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if decisions.is_empty() && !always {
            return None;
        }
        let t = theme::current(cx);
        let link = |id: SharedString, label: &'static str, action: Box<dyn Action>| {
            div()
                .id(id)
                .flex_none()
                .px_1()
                .rounded(t.radius_sm())
                .cursor_pointer()
                .text_color(t.colors.text_muted)
                .hover(|s| s.bg(t.colors.highlight).text_color(t.colors.highlight_text))
                .child(label)
                .on_click(move |_: &ClickEvent, window, cx| {
                    window.dispatch_action(action.boxed_clone(), cx);
                })
        };
        let header = div()
            .flex()
            .items_center()
            .gap_3()
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Decisions"))
            .when(!decisions.is_empty(), |s| {
                s.child(
                    div()
                        .text_size(px(12.))
                        .text_color(t.colors.text_muted)
                        .child(
                            decisions
                                .iter()
                                .filter(|d| d.in_force())
                                .count()
                                .to_string(),
                        ),
                )
            })
            .child(div().flex_1())
            .child(div().text_size(px(12.)).child(link(
                "decision-record".into(),
                "+ Record",
                Box::new(record_on(here, None)),
            )));

        let rows = decisions.iter().map(|d| {
            let id = d.id;
            let author = if d.author.is_user() {
                "You".to_owned()
            } else {
                d.author.to_string()
            };
            let inherited = (&d.owner != here).then(|| format!("on {}", owner_name(&d.owner)));
            let ended = match d.state {
                DecisionState::InForce => None,
                DecisionState::Superseded => Some("replaced"),
                DecisionState::Withdrawn => Some("withdrawn"),
            };
            let ended_by = d
                .ended_by
                .as_ref()
                .filter(|_| ended.is_some())
                .map(|by| format!("by {by}"));
            let controls = div()
                .flex()
                .gap_1()
                .invisible()
                .group_hover("decision", |s| s.visible())
                .when(d.in_force(), |s| {
                    s.child(link(
                        format!("decision-replace-{id}").into(),
                        "Replace…",
                        Box::new(Replace {
                            id: Some(id),
                            text: None,
                        }),
                    ))
                    .child(link(
                        format!("decision-withdraw-{id}").into(),
                        "Withdraw",
                        Box::new(Withdraw {
                            id: Some(id),
                            confirmed: false,
                        }),
                    ))
                });
            div()
                .id(("decision", id.0 as u64))
                .group("decision")
                .flex()
                .flex_col()
                .pl_3()
                .pr_1()
                .py_1()
                .border_l_2()
                .border_color(if !d.reviewed {
                    t.colors.accent
                } else {
                    t.colors.border
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(12.))
                        .line_height(px(20.))
                        .text_color(t.colors.text_muted)
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(t.colors.text)
                                .child(author),
                        )
                        .children(inherited)
                        .child(format_age(d.created_at))
                        .children(ended.map(|e| div().text_color(t.colors.error).child(e)))
                        .children(ended_by)
                        .when(!d.reviewed, |s| {
                            s.child(div().text_color(t.colors.accent).child("unreviewed"))
                                .child(link(
                                    format!("decision-confirm-{id}").into(),
                                    "Confirm",
                                    Box::new(MarkReviewed { id: Some(id) }),
                                ))
                        })
                        .child(div().flex_1())
                        .child(controls),
                )
                .child(
                    div()
                        .line_height(px(22.))
                        .when(!d.in_force(), |s| {
                            s.text_color(t.colors.text_muted).line_through()
                        })
                        .child(SharedString::from(d.text.clone())),
                )
        });

        Some(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(header)
                .children(rows)
                .into_any_element(),
        )
    }
}
