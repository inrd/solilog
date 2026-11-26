//! The header above the issue list while it shows a label: its name (in its
//! color), how many issues carry it or a sublabel, its paths (folders where
//! its code lives; see `path_chips.rs`) and the decisions that apply to it
//! (see `decisions.rs`).

use gpui::{Action as _, Context, FontWeight, IntoElement, div, prelude::*, px};
use solilog_core::{DecisionOwner, Label};

use super::Workspace;
use super::path_chips::path_chips;
use super::scope::Source;
use crate::ui::actions::label;
use crate::ui::theme;
use crate::ui::tooltip::truncated_text;

impl Workspace {
    /// The label the issue list shows, if it shows one.
    pub(super) fn shown_label(&self) -> Option<&Label> {
        match &self.scope.source {
            Source::Label(name) => self.labels.iter().find(|l| l.name == *name),
            _ => None,
        }
    }

    pub(super) fn render_label_header(
        &self,
        shown: &Label,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = theme::current(cx);
        let count = match shown.issue_count {
            0 => "No issues yet".to_string(),
            1 => "1 issue".to_string(),
            n => format!("{n} issues"),
        };
        let paths = path_chips(
            "label-path",
            &shown.paths,
            |path| {
                Box::new(label::RemovePath {
                    name: None,
                    path: Some(path),
                })
            },
            label::AddPath::default().boxed_clone(),
            &t,
        );
        let here = DecisionOwner::label(&shown.name);
        let record = self.record_decision_chip(&here, &t);
        let decisions = self.render_decisions(&self.scope_decisions, &here, false, cx);
        div()
            .flex()
            .flex_col()
            .gap_2()
            .px_4()
            .pt_3()
            .pb_2()
            .when(t.surface_toolbars(), |s| s.bg(t.colors.surface))
            .border_b_1()
            .border_color(t.colors.divider)
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(18.))
                            .line_height(px(26.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.label_color(shown.display_color))
                            .child(truncated_text("label-header-name", shown.name.clone())),
                    )
                    .child(
                        div()
                            .ml_auto()
                            .pl_3()
                            .flex_none()
                            .text_size(px(12.))
                            .text_color(t.colors.text_muted)
                            .child(count),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_1()
                    .text_size(px(12.))
                    .children(paths)
                    .children(record),
            )
            .children(decisions)
    }
}
