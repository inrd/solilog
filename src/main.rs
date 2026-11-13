mod db;
mod issue;
mod text_input;

use gpui::{
    AnyElement, App, Application, Bounds, Context, Entity, FocusHandle, Focusable, Hsla,
    KeyBinding, MouseButton, SharedString, TitlebarOptions, Window, WindowBounds, WindowOptions,
    actions, anchored, deferred, div, point, prelude::*, px, rgb, size,
};

use db::Db;
use issue::{Issue, Status};
use text_input::{Submit, TextInput};

const APP_NAME: &str = "Solilog";
const MENU_CONTEXT: &str = "StatusMenu";

actions!(solilog, [Quit]);
actions!(status_menu, [MenuUp, MenuDown, MenuConfirm, MenuDismiss]);

/// The open status picker: which issue it edits and which entry is highlighted.
struct StatusMenu {
    issue_id: i64,
    highlighted: usize,
}

struct Solilog {
    db: Db,
    input: Entity<TextInput>,
    /// Newest first.
    issues: Vec<Issue>,
    status_menu: Option<StatusMenu>,
    menu_focus: FocusHandle,
}

impl Solilog {
    fn new(db: Db, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextInput::new("New issue…", cx));
        cx.subscribe(&input, |this, input, _: &Submit, cx| {
            let title = input.read(cx).text().trim().to_string();
            if title.is_empty() {
                return;
            }
            match this.db.create_issue(&title) {
                Ok(issue) => {
                    this.issues.insert(0, issue);
                    input.update(cx, |input, cx| input.clear(cx));
                    cx.notify();
                }
                // Keep the text in the input so nothing is lost.
                Err(err) => eprintln!("failed to create issue: {err:#}"),
            }
        })
        .detach();

        window.focus(&input.read(cx).focus_handle(cx));

        let issues = db.list_issues().unwrap_or_else(|err| {
            eprintln!("failed to load issues: {err:#}");
            Vec::new()
        });

        Self {
            db,
            input,
            issues,
            status_menu: None,
            menu_focus: cx.focus_handle(),
        }
    }

    fn open_status_menu(&mut self, issue_id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(issue) = self.issues.iter().find(|i| i.id == issue_id) else {
            return;
        };
        let highlighted = Status::ALL
            .iter()
            .position(|s| *s == issue.status)
            .unwrap_or(0);
        self.status_menu = Some(StatusMenu {
            issue_id,
            highlighted,
        });
        window.focus(&self.menu_focus);
        cx.notify();
    }

    fn close_status_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.status_menu.take().is_some() {
            window.focus(&self.input.focus_handle(cx));
            cx.notify();
        }
    }

    fn set_status(&mut self, issue_id: i64, status: Status, cx: &mut Context<Self>) {
        if let Err(err) = self.db.set_status(issue_id, status) {
            eprintln!("failed to set status: {err:#}");
            return;
        }
        if let Some(issue) = self.issues.iter_mut().find(|i| i.id == issue_id) {
            issue.status = status;
        }
        cx.notify();
    }

    fn menu_up(&mut self, _: &MenuUp, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = &mut self.status_menu {
            menu.highlighted = menu
                .highlighted
                .checked_sub(1)
                .unwrap_or(Status::ALL.len() - 1);
            cx.notify();
        }
    }

    fn menu_down(&mut self, _: &MenuDown, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = &mut self.status_menu {
            menu.highlighted = (menu.highlighted + 1) % Status::ALL.len();
            cx.notify();
        }
    }

    fn menu_confirm(&mut self, _: &MenuConfirm, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = &self.status_menu {
            let (id, status) = (menu.issue_id, Status::ALL[menu.highlighted]);
            self.set_status(id, status, cx);
        }
        self.close_status_menu(window, cx);
    }

    fn menu_dismiss(&mut self, _: &MenuDismiss, window: &mut Window, cx: &mut Context<Self>) {
        self.close_status_menu(window, cx);
    }

    fn render_issue(&self, issue: &Issue, cx: &mut Context<Self>) -> impl IntoElement {
        let issue_id = issue.id;
        let menu_open = self
            .status_menu
            .as_ref()
            .is_some_and(|m| m.issue_id == issue_id);

        let trigger = div()
            .id(("status", issue_id as u64))
            .size(px(20.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded_sm()
            .cursor_pointer()
            .hover(|s| s.bg(rgb(0x2c2c31)))
            .when(menu_open, |s| s.bg(rgb(0x2c2c31)))
            .child(status_icon(issue.status))
            .on_click(
                cx.listener(move |this, _, window, cx| this.open_status_menu(issue_id, window, cx)),
            )
            .when(menu_open, |s| s.child(self.render_status_menu(cx)));

        div()
            .flex()
            .items_center()
            .gap_3()
            .px_4()
            .py_1()
            .child(trigger)
            .child(
                div()
                    .w(px(52.))
                    .flex_none()
                    .text_color(rgb(0x7a7a85))
                    .child(format!("SOL-{issue_id}")),
            )
            .child(
                div()
                    .flex_1()
                    .truncate()
                    .when(issue.status.is_closed(), |s| s.text_color(rgb(0x7a7a85)))
                    .child(issue.title.clone()),
            )
    }

    /// The popover, anchored just below the status icon.
    fn render_status_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(menu) = &self.status_menu else {
            return div().into_any_element();
        };
        let issue_id = menu.issue_id;

        let items = Status::ALL.into_iter().enumerate().map(|(ix, status)| {
            div()
                .id(status.key())
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .py_1()
                .rounded_sm()
                .cursor_pointer()
                .when(ix == menu.highlighted, |s| s.bg(rgb(0x34343b)))
                .on_mouse_move(cx.listener(move |this, _, _, cx| {
                    if let Some(menu) = &mut this.status_menu
                        && menu.highlighted != ix
                    {
                        menu.highlighted = ix;
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.set_status(issue_id, status, cx);
                    this.close_status_menu(window, cx);
                }))
                .child(status_icon(status))
                .child(status.label())
        });

        deferred(
            anchored()
                .offset(point(px(0.), px(24.)))
                .snap_to_window_with_margin(px(8.))
                .child(
                    div()
                        .id("status-menu")
                        .key_context(MENU_CONTEXT)
                        .track_focus(&self.menu_focus)
                        .on_action(cx.listener(Self::menu_up))
                        .on_action(cx.listener(Self::menu_down))
                        .on_action(cx.listener(Self::menu_confirm))
                        .on_action(cx.listener(Self::menu_dismiss))
                        .occlude()
                        .w(px(180.))
                        .p_1()
                        .flex()
                        .flex_col()
                        .bg(rgb(0x26262b))
                        .border_1()
                        .border_color(rgb(0x3a3a42))
                        .rounded_md()
                        .shadow_lg()
                        .children(items),
                ),
        )
        .with_priority(2)
        .into_any_element()
    }
}

impl Render for Solilog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<_> = self
            .issues
            .iter()
            .map(|issue| self.render_issue(issue, cx).into_any_element())
            .collect();

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x1c1c1f))
            .text_color(rgb(0xe6e6e6))
            .text_size(px(14.))
            .line_height(px(20.))
            .child(
                div()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(rgb(0x2c2c31))
                    .child(self.input.clone()),
            )
            .child(
                div()
                    .id("issues")
                    .flex_1()
                    .flex()
                    .flex_col()
                    .overflow_y_scroll()
                    .py_1()
                    .children(rows),
            )
            // While the menu is open, a transparent backdrop catches clicks outside it.
            .when(self.status_menu.is_some(), |root| {
                root.child(
                    deferred(
                        div()
                            .id("menu-backdrop")
                            .absolute()
                            .inset_0()
                            .occlude()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, window, cx| {
                                    this.close_status_menu(window, cx)
                                }),
                            )
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(|this, _, window, cx| {
                                    this.close_status_menu(window, cx)
                                }),
                            ),
                    )
                    .with_priority(1),
                )
            })
    }
}

/// Small glyph for a status, in the spirit of Linear's status icons.
fn status_icon(status: Status) -> impl IntoElement {
    let (color, glyph): (Hsla, Option<&str>) = match status {
        Status::Backlog => (rgb(0x6e6e78).into(), None),
        Status::Todo => (rgb(0xb4b4bc).into(), None),
        Status::InProgress => (rgb(0xf2c94c).into(), None),
        Status::Blocked => (rgb(0xeb5757).into(), Some("!")),
        Status::Done => (rgb(0x7c8cff).into(), Some("✓")),
        Status::Cancelled => (rgb(0x6e6e78).into(), Some("×")),
    };

    let circle = div()
        .size(px(14.))
        .flex()
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
        _ => circle.bg(color).text_color(rgb(0x1c1c1f)).children(glyph),
    }
}

fn main() -> anyhow::Result<()> {
    let db = Db::open_default()?;

    Application::new().run(move |cx: &mut App| {
        text_input::bind_keys(cx);
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("up", MenuUp, Some(MENU_CONTEXT)),
            KeyBinding::new("down", MenuDown, Some(MENU_CONTEXT)),
            KeyBinding::new("enter", MenuConfirm, Some(MENU_CONTEXT)),
            KeyBinding::new("escape", MenuDismiss, Some(MENU_CONTEXT)),
        ]);
        cx.on_action(|_: &Quit, cx| cx.quit());

        let bounds = Bounds::centered(None, size(px(1000.), px(700.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some(SharedString::from(APP_NAME)),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Solilog::new(db, window, cx)),
        )
        .unwrap();

        // Quit the app when the last window is closed.
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        cx.activate(true);
    });

    Ok(())
}
