//! The window's root view: the workspace, plus the window frame when the
//! app has to draw it itself.
//!
//! Solilog asks for server-side decorations (title bar and borders drawn by
//! the system) where they exist: on macOS, X11 and most Wayland compositors.
//! Some Wayland compositors (GNOME's) don't provide them, and then the app
//! draws a title bar with the window buttons, a border, and edges to resize
//! the window.

use gpui::{
    AnyElement, App, ClickEvent, Context, CursorStyle, Decorations, Entity, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ResizeEdge, Tiling, Window,
    WindowDecorations, div, prelude::*, px, rgb, svg,
};

use super::workspace::Workspace;
use super::{APP_NAME, assets, theme};

const TITLE_BAR_HEIGHT: f32 = 34.;
/// Width of the zone along each window edge that resizes the window.
const RESIZE_EDGE: f32 = 5.;
/// Size of the corner zones, which resize in both directions.
const RESIZE_CORNER: f32 = 12.;

/// The decorations to ask for when opening the window: server-side, unless
/// the Wayland compositor can't draw them.
///
/// gpui can't tell: on Wayland it takes a request for server-side
/// decorations as granted unless the compositor answers otherwise, and
/// compositors without the decoration protocol (GNOME's) never answer, so
/// the window would have no title bar at all. Ask the compositor first.
pub fn decorations() -> WindowDecorations {
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    if wayland::lacks_server_decorations() {
        return WindowDecorations::Client;
    }
    WindowDecorations::Server
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod wayland {
    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::wl_registry::{self, WlRegistry};
    use wayland_client::{Connection, Dispatch, QueueHandle};

    /// The protocol through which a client asks for server-side decorations.
    const DECORATION_MANAGER: &str = "zxdg_decoration_manager_v1";

    /// Whether the app runs on a Wayland compositor that doesn't offer
    /// server-side decorations. False on X11, or when in doubt.
    pub fn lacks_server_decorations() -> bool {
        // gpui uses Wayland when `WAYLAND_DISPLAY` is set. A connection
        // handed over in `WAYLAND_SOCKET` can only be opened once: leave it
        // to gpui.
        if std::env::var_os("WAYLAND_DISPLAY").is_none()
            || std::env::var_os("WAYLAND_SOCKET").is_some()
        {
            return false;
        }
        let Ok(connection) = Connection::connect_to_env() else {
            return false;
        };
        let Ok((globals, _queue)) = registry_queue_init::<Probe>(&connection) else {
            return false;
        };
        !globals.contents().with_list(|list| {
            list.iter()
                .any(|global| global.interface == DECORATION_MANAGER)
        })
    }

    /// Receives registry events; the global list is all we need.
    struct Probe;

    impl Dispatch<WlRegistry, GlobalListContents> for Probe {
        fn event(
            _: &mut Self,
            _: &WlRegistry,
            _: wl_registry::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }
}

pub struct WindowFrame {
    workspace: Entity<Workspace>,
    /// The title bar was pressed and not released yet: moving the mouse now
    /// moves the window. (Moving on press would swallow double clicks.)
    drag_pending: bool,
}

impl WindowFrame {
    pub fn new(workspace: Entity<Workspace>) -> Self {
        Self {
            workspace,
            drag_pending: false,
        }
    }

    fn render_title_bar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let controls = window.window_controls();
        let maximized = window.is_maximized();
        div()
            .id("title-bar")
            .flex_none()
            .h(px(TITLE_BAR_HEIGHT))
            .flex()
            .items_center()
            .relative()
            .bg(rgb(theme::SIDEBAR))
            .border_b_1()
            .border_color(rgb(theme::DIVIDER))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, _| this.drag_pending = true),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| this.drag_pending = false),
            )
            .on_mouse_move(cx.listener(|this, _: &MouseMoveEvent, window, _| {
                if this.drag_pending {
                    this.drag_pending = false;
                    window.start_window_move();
                }
            }))
            .on_click(|event: &ClickEvent, window, _| {
                if event.click_count() == 2 {
                    window.zoom_window();
                }
            })
            .when(controls.window_menu, |bar| {
                bar.on_mouse_down(MouseButton::Right, |event, window, _| {
                    window.show_window_menu(event.position);
                })
            })
            // The title, centered on the whole bar whatever the buttons' width.
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(rgb(theme::TEXT_MUTED))
                    .text_size(px(13.))
                    .child(APP_NAME),
            )
            .child(
                div()
                    .ml_auto()
                    .mr(px(8.))
                    .flex()
                    .gap(px(6.))
                    .when(controls.minimize, |row| {
                        row.child(window_button(
                            "minimize",
                            assets::WINDOW_MINIMIZE,
                            |window, _| window.minimize_window(),
                        ))
                    })
                    .when(controls.maximize, |row| {
                        let icon = if maximized {
                            assets::WINDOW_RESTORE
                        } else {
                            assets::WINDOW_MAXIMIZE
                        };
                        row.child(window_button("maximize", icon, |window, _| {
                            window.zoom_window()
                        }))
                    })
                    .child(window_button("close", assets::WINDOW_CLOSE, |window, _| {
                        window.remove_window()
                    })),
            )
    }
}

impl Render for WindowFrame {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let workspace = div()
            .flex_1()
            .min_h_0()
            .relative()
            .child(self.workspace.clone());
        let Decorations::Client { tiling } = window.window_decorations() else {
            return div().size_full().flex().flex_col().child(workspace);
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .relative()
            .bg(rgb(theme::BACKGROUND))
            .border_color(rgb(theme::BORDER))
            .when(!tiling.top, |d| d.border_t_1())
            .when(!tiling.bottom, |d| d.border_b_1())
            .when(!tiling.left, |d| d.border_l_1())
            .when(!tiling.right, |d| d.border_r_1())
            .child(self.render_title_bar(window, cx))
            .child(workspace)
            .children(resize_edges(tiling))
    }
}

/// A title bar button: an icon in a circle, like GNOME's.
fn window_button(
    id: &'static str,
    icon: &'static str,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .size(px(24.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(rgb(theme::SURFACE))
        .hover(|s| s.bg(rgb(theme::HIGHLIGHT)))
        // Not a press on the title bar: don't start moving the window.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            on_click(window, cx);
        })
        .child(svg().path(icon).size(px(14.)).text_color(rgb(theme::TEXT)))
}

/// Invisible zones along the edges that are not against the screen's (or
/// another window's) edge, to resize the window by dragging them.
fn resize_edges(tiling: Tiling) -> Vec<AnyElement> {
    use ResizeEdge::*;
    let free = |edge: ResizeEdge| match edge {
        Top => !tiling.top,
        Bottom => !tiling.bottom,
        Left => !tiling.left,
        Right => !tiling.right,
        TopLeft => !tiling.top && !tiling.left,
        TopRight => !tiling.top && !tiling.right,
        BottomLeft => !tiling.bottom && !tiling.left,
        BottomRight => !tiling.bottom && !tiling.right,
    };
    // Corners last, so they are above the edges where they overlap.
    [
        Top,
        Bottom,
        Left,
        Right,
        TopLeft,
        TopRight,
        BottomLeft,
        BottomRight,
    ]
    .into_iter()
    .filter(|&edge| free(edge))
    .map(|edge| resize_zone(edge).into_any_element())
    .collect()
}

fn resize_zone(edge: ResizeEdge) -> impl IntoElement {
    use ResizeEdge::*;
    let (edge_px, corner) = (px(RESIZE_EDGE), px(RESIZE_CORNER));
    let zone = div().id(edge_id(edge)).absolute().occlude();
    let zone = match edge {
        Top => zone.top_0().left_0().w_full().h(edge_px),
        Bottom => zone.bottom_0().left_0().w_full().h(edge_px),
        Left => zone.top_0().left_0().h_full().w(edge_px),
        Right => zone.top_0().right_0().h_full().w(edge_px),
        TopLeft => zone.top_0().left_0().size(corner),
        TopRight => zone.top_0().right_0().size(corner),
        BottomLeft => zone.bottom_0().left_0().size(corner),
        BottomRight => zone.bottom_0().right_0().size(corner),
    };
    let cursor = match edge {
        Top | Bottom => CursorStyle::ResizeUpDown,
        Left | Right => CursorStyle::ResizeLeftRight,
        TopLeft | BottomRight => CursorStyle::ResizeUpLeftDownRight,
        TopRight | BottomLeft => CursorStyle::ResizeUpRightDownLeft,
    };
    zone.cursor(cursor)
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            cx.stop_propagation();
            window.start_window_resize(edge);
        })
}

fn edge_id(edge: ResizeEdge) -> &'static str {
    use ResizeEdge::*;
    match edge {
        Top => "resize-top",
        Bottom => "resize-bottom",
        Left => "resize-left",
        Right => "resize-right",
        TopLeft => "resize-top-left",
        TopRight => "resize-top-right",
        BottomLeft => "resize-bottom-left",
        BottomRight => "resize-bottom-right",
    }
}
