use gpui::{Hsla, IntoElement, ParentElement, Styled, div, px, rgb};
use solilog_core::{ProjectStatus, Status};

use super::theme;

/// Small glyph for a status, in the spirit of Linear's status icons.
pub fn status_icon(status: Status) -> impl IntoElement {
    let (color, glyph): (Hsla, Option<&str>) = match status {
        Status::Backlog => (rgb(0x6e6e78).into(), None),
        Status::Todo => (rgb(0xb4b4bc).into(), None),
        Status::InProgress => (rgb(0xf2c94c).into(), None),
        Status::Blocked => (rgb(theme::ERROR).into(), Some("!")),
        Status::Done => (rgb(theme::ACCENT).into(), Some("✓")),
        Status::Cancelled => (rgb(0x6e6e78).into(), Some("×")),
    };

    let circle = div()
        .size(px(14.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded_full()
        .text_size(px(10.))
        .line_height(px(14.))
        .font_weight(gpui::FontWeight::BOLD);

    match status {
        Status::Backlog => circle.border_1().border_dashed().border_color(color),
        Status::Todo => circle.border_1().border_color(color),
        Status::InProgress => circle
            .border_1()
            .border_color(color)
            .child(div().size(px(6.)).rounded_full().bg(color)),
        _ => circle
            .bg(color)
            .text_color(rgb(theme::BACKGROUND))
            .children(glyph),
    }
}

/// Small glyph for a project status: a rounded square, so projects don't read
/// as issues.
pub fn project_status_icon(status: ProjectStatus) -> impl IntoElement {
    let color: Hsla = match status {
        ProjectStatus::Planned => rgb(0xb4b4bc).into(),
        ProjectStatus::Active => rgb(0xf2c94c).into(),
        ProjectStatus::Done => rgb(theme::ACCENT).into(),
        ProjectStatus::Archived => rgb(0x6e6e78).into(),
    };

    let square = div()
        .size(px(14.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .text_size(px(10.))
        .line_height(px(14.))
        .font_weight(gpui::FontWeight::BOLD);

    match status {
        ProjectStatus::Planned => square.border_1().border_dashed().border_color(color),
        ProjectStatus::Active => square
            .border_1()
            .border_color(color)
            .child(div().size(px(6.)).rounded(px(1.5)).bg(color)),
        ProjectStatus::Done => square
            .bg(color)
            .text_color(rgb(theme::BACKGROUND))
            .child("✓"),
        ProjectStatus::Archived => square
            .bg(color)
            .child(div().w(px(6.)).h(px(1.5)).bg(rgb(theme::BACKGROUND))),
    }
}
