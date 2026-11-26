use gpui::{IntoElement, ParentElement, Styled, div, prelude::*, px, rgba};
use solilog_core::{ProjectStatus, Status};

use super::theme::Theme;

/// Small glyph for a status, in the spirit of Linear's status icons.
pub fn status_icon(status: Status, theme: &Theme) -> impl IntoElement + use<> {
    let color = theme.status_color(status);
    let glyph = match status {
        Status::Blocked => Some("!"),
        Status::Done => Some("✓"),
        Status::Cancelled => Some("×"),
        _ => None,
    };

    let pixel = theme.is_pixel_icons();
    let base = div()
        .size(px(14.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .when(!pixel, |s| s.rounded_full())
        .text_size(px(10.))
        .line_height(px(14.))
        .font_weight(gpui::FontWeight::BOLD);

    match status {
        Status::Backlog => base.border_1().border_dashed().border_color(color),
        Status::Todo => base.border_1().border_color(color),
        Status::InProgress => base.border_1().border_color(color).child(
            div()
                .size(px(6.))
                .when(!pixel, |s| s.rounded_full())
                .bg(color),
        ),
        // Almost full: one step from done.
        Status::InReview => base.border_1().border_color(color).child(
            div()
                .size(px(9.))
                .when(!pixel, |s| s.rounded_full())
                .bg(color),
        ),
        _ => base
            .relative()
            .bg(color)
            .text_color(theme.colors.status.glyph)
            .when(theme.is_glass(), |s| {
                s.child(
                    div()
                        .absolute()
                        .top(px(1.5))
                        .left(px(2.5))
                        .w(px(6.))
                        .h(px(3.))
                        .rounded_full()
                        .bg(rgba(0xffffff66)),
                )
            })
            .children(glyph),
    }
}

/// Small glyph for a project status: a rounded square, so projects don't read
/// as issues.
pub fn project_status_icon(status: ProjectStatus, theme: &Theme) -> impl IntoElement + use<> {
    let color = theme.project_status_color(status);
    let inner_radius = if theme.radius <= 0. { px(0.) } else { px(1.5) };

    let square = div()
        .size(px(14.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(theme.radius_sm())
        .text_size(px(10.))
        .line_height(px(14.))
        .font_weight(gpui::FontWeight::BOLD);

    match status {
        ProjectStatus::Planned => square.border_1().border_dashed().border_color(color),
        ProjectStatus::Active => square
            .border_1()
            .border_color(color)
            .child(div().size(px(6.)).rounded(inner_radius).bg(color)),
        ProjectStatus::Done => square
            .bg(color)
            .text_color(theme.colors.status.glyph)
            .child("✓"),
        ProjectStatus::Archived => square
            .bg(color)
            .child(div().w(px(6.)).h(px(1.5)).bg(theme.colors.status.glyph)),
    }
}
