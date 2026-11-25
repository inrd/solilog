//! The window's root view: the workspace, plus the window frame when the
//! app has to draw it itself.
//!
//! Solilog asks for server-side decorations (title bar and borders drawn by
//! the system) where they exist: on macOS, X11 and most Wayland compositors.
//! Some Wayland compositors (GNOME's) don't provide them, and then the app
//! draws a title bar with the window buttons, a border, and edges to resize
//! the window.

use gpui::{
    AnyElement, App, ClickEvent, Context, CursorStyle, Decorations, Entity, FontWeight,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ResizeEdge, Tiling,
    Window, WindowDecorations, div, linear_color_stop, linear_gradient, prelude::*, px, rgba, svg,
};

use super::workspace::Workspace;
use super::{APP_NAME, assets, theme};

const TITLE_BAR_HEIGHT: f32 = 34.;
const MACOS_TITLE_BAR_HEIGHT: f32 = 28.;
const CLASSIC_TITLE_BAR_HEIGHT: f32 = 24.;
const GLASS_TITLE_BAR_HEIGHT: f32 = 28.;
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
pub fn decorations(classic_titlebar: bool) -> WindowDecorations {
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    if classic_titlebar || wayland::lacks_server_decorations() {
        return WindowDecorations::Client;
    }
    let _ = classic_titlebar;
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
    /// Last `uses_custom_titlebar` value requested from the Linux WM, so theme
    /// changes toggle client/server decorations live without redundant requests.
    last_custom_titlebar: Option<bool>,
}

impl WindowFrame {
    pub fn new(workspace: Entity<Workspace>) -> Self {
        Self {
            workspace,
            drag_pending: false,
            last_custom_titlebar: None,
        }
    }

    fn render_crt_title_bar(
        &self,
        window: &Window,
        t: &theme::Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active = window.is_window_active();
        let maximized = window.is_maximized();
        let (bg, fg) = if active {
            (t.colors.titlebar_active_start, t.colors.titlebar_text)
        } else {
            (
                t.colors.titlebar_inactive_start,
                t.colors.titlebar_inactive_text,
            )
        };
        let pixel = t.is_pixel_icons();
        div()
            .id("crt-title-bar")
            .flex_none()
            .h(px(CLASSIC_TITLE_BAR_HEIGHT))
            .px(px(6.))
            .flex()
            .items_center()
            .gap(px(6.))
            .bg(bg)
            .border_b_1()
            .border_color(t.colors.border)
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
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(fg)
                    .text_size(px(12.))
                    .font_weight(FontWeight::BOLD)
                    .child(format!("[ {} ]", APP_NAME.to_uppercase())),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(3.))
                    .child(crt_window_button(
                        "minimize",
                        assets::icon_for(assets::WINDOW_MINIMIZE, pixel),
                        fg,
                        t,
                        |window, _| window.minimize_window(),
                    ))
                    .child(crt_window_button(
                        "maximize",
                        assets::icon_for(
                            if maximized {
                                assets::WINDOW_RESTORE
                            } else {
                                assets::WINDOW_MAXIMIZE
                            },
                            pixel,
                        ),
                        fg,
                        t,
                        |window, _| window.zoom_window(),
                    ))
                    .child(crt_window_button(
                        "close",
                        assets::icon_for(assets::WINDOW_CLOSE, pixel),
                        fg,
                        t,
                        |window, _| window.remove_window(),
                    )),
            )
    }

    fn render_glass_title_bar(
        &self,
        window: &Window,
        t: &theme::Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active = window.is_window_active();
        let maximized = window.is_maximized();
        let (start, end, text_color) = if active {
            (
                t.colors.titlebar_active_start,
                t.colors.titlebar_active_end,
                t.colors.titlebar_text,
            )
        } else {
            (
                t.colors.titlebar_inactive_start,
                t.colors.titlebar_inactive_end,
                t.colors.titlebar_inactive_text,
            )
        };
        let pixel = t.is_pixel_icons();
        div()
            .id("glass-title-bar")
            .relative()
            .flex_none()
            .h(px(GLASS_TITLE_BAR_HEIGHT))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(8.))
            .bg(linear_gradient(
                105.,
                linear_color_stop(start, 0.),
                linear_color_stop(end, 1.),
            ))
            .border_b_1()
            .border_color(t.colors.border)
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
            .children(t.gloss_overlay())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(text_color)
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(APP_NAME),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(3.))
                    .child(glass_window_button(
                        "minimize",
                        assets::icon_for(assets::WINDOW_MINIMIZE, pixel),
                        false,
                        t,
                        |window, _| window.minimize_window(),
                    ))
                    .child(glass_window_button(
                        "maximize",
                        assets::icon_for(
                            if maximized {
                                assets::WINDOW_RESTORE
                            } else {
                                assets::WINDOW_MAXIMIZE
                            },
                            pixel,
                        ),
                        false,
                        t,
                        |window, _| window.zoom_window(),
                    ))
                    .child(glass_window_button(
                        "close",
                        assets::icon_for(assets::WINDOW_CLOSE, pixel),
                        true,
                        t,
                        |window, _| window.remove_window(),
                    )),
            )
    }

    fn render_classic_title_bar(
        &self,
        window: &Window,
        t: &theme::Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active = window.is_window_active();
        let maximized = window.is_maximized();
        let (start, end, text_color) = if active {
            (
                t.colors.titlebar_active_start,
                t.colors.titlebar_active_end,
                t.colors.titlebar_text,
            )
        } else {
            (
                t.colors.titlebar_inactive_start,
                t.colors.titlebar_inactive_end,
                t.colors.titlebar_inactive_text,
            )
        };
        let pixel = t.is_pixel_icons();
        div()
            .id("classic-title-bar")
            .flex_none()
            .h(px(CLASSIC_TITLE_BAR_HEIGHT))
            .px(px(4.))
            .flex()
            .items_center()
            .gap(px(4.))
            .bg(linear_gradient(
                90.,
                linear_color_stop(start, 0.),
                linear_color_stop(end, 1.),
            ))
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
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .pl(px(2.))
                    .text_color(text_color)
                    .text_size(px(12.))
                    .font_weight(FontWeight::BOLD)
                    .child(APP_NAME),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .child(classic_window_button(
                        "minimize",
                        assets::icon_for(assets::WINDOW_MINIMIZE, pixel),
                        t,
                        |window, _| window.minimize_window(),
                    ))
                    .child(classic_window_button(
                        "maximize",
                        assets::icon_for(
                            if maximized {
                                assets::WINDOW_RESTORE
                            } else {
                                assets::WINDOW_MAXIMIZE
                            },
                            pixel,
                        ),
                        t,
                        |window, _| window.zoom_window(),
                    ))
                    .child(
                        div().ml(px(2.)).child(classic_window_button(
                            "close",
                            assets::icon_for(assets::WINDOW_CLOSE, pixel),
                            t,
                            |window, _| window.remove_window(),
                        )),
                    ),
            )
    }

    fn render_macos_native_bar(
        &self,
        t: &theme::Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id("macos-title-bar")
            .flex_none()
            .h(px(MACOS_TITLE_BAR_HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .bg(t.colors.sidebar)
            .border_b_1()
            .border_color(t.colors.divider)
            .text_color(t.colors.text_muted)
            .text_size(px(12.))
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
            .child(APP_NAME)
    }

    fn render_title_bar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::current(cx);
        let controls = window.window_controls();
        let maximized = window.is_maximized();
        let pixel = t.is_pixel_icons();
        div()
            .id("title-bar")
            .flex_none()
            .h(px(TITLE_BAR_HEIGHT))
            .flex()
            .items_center()
            .relative()
            .bg(t.colors.sidebar)
            .border_b_1()
            .border_color(t.colors.divider)
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
                    .text_color(t.colors.text_muted)
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
                            assets::icon_for(assets::WINDOW_MINIMIZE, pixel),
                            &t,
                            |window, _| window.minimize_window(),
                        ))
                    })
                    .when(controls.maximize, |row| {
                        let icon = if maximized {
                            assets::WINDOW_RESTORE
                        } else {
                            assets::WINDOW_MAXIMIZE
                        };
                        row.child(window_button(
                            "maximize",
                            assets::icon_for(icon, pixel),
                            &t,
                            |window, _| window.zoom_window(),
                        ))
                    })
                    .child(window_button(
                        "close",
                        assets::icon_for(assets::WINDOW_CLOSE, pixel),
                        &t,
                        |window, _| window.remove_window(),
                    )),
            )
    }
}

impl Render for WindowFrame {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::current(cx);
        let custom = t.uses_custom_titlebar();
        if cfg!(any(target_os = "linux", target_os = "freebsd"))
            && self.last_custom_titlebar != Some(custom)
        {
            self.last_custom_titlebar = Some(custom);
            window.request_decorations(decorations(custom));
        }
        let workspace = div()
            .flex_1()
            .min_h_0()
            .relative()
            .child(self.workspace.clone());

        if let Decorations::Client { tiling } = window.window_decorations() {
            if t.is_classic_titlebar() {
                return div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .relative()
                    .p(px(3.))
                    .gap(px(2.))
                    .bg(t.colors.surface)
                    .when_some(t.resolved_font_family(), |d, f| d.font_family(f))
                    .child(self.render_classic_title_bar(window, &t, cx))
                    .child(workspace)
                    .children(t.raised_bevel())
                    .children(resize_edges(tiling));
            }
            if t.is_crt_titlebar() {
                return div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .relative()
                    .bg(t.colors.background)
                    .when_some(t.resolved_font_family(), |d, f| d.font_family(f))
                    .border_color(t.colors.border)
                    .when(!tiling.top, |d| d.border_t_1())
                    .when(!tiling.bottom, |d| d.border_b_1())
                    .when(!tiling.left, |d| d.border_l_1())
                    .when(!tiling.right, |d| d.border_r_1())
                    .child(self.render_crt_title_bar(window, &t, cx))
                    .child(workspace)
                    .children(resize_edges(tiling));
            }
            if t.is_glass_titlebar() {
                return div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .relative()
                    .bg(t.colors.background)
                    .when_some(t.resolved_font_family(), |d, f| d.font_family(f))
                    .border_color(t.colors.border)
                    .when(!tiling.top, |d| d.border_t_1())
                    .when(!tiling.bottom, |d| d.border_b_1())
                    .when(!tiling.left, |d| d.border_l_1())
                    .when(!tiling.right, |d| d.border_r_1())
                    .child(self.render_glass_title_bar(window, &t, cx))
                    .child(workspace)
                    .children(resize_edges(tiling));
            }
            return div()
                .size_full()
                .flex()
                .flex_col()
                .relative()
                .bg(t.colors.background)
                .when_some(t.resolved_font_family(), |d, f| d.font_family(f))
                .border_color(t.colors.border)
                .when(!tiling.top, |d| d.border_t_1())
                .when(!tiling.bottom, |d| d.border_b_1())
                .when(!tiling.left, |d| d.border_l_1())
                .when(!tiling.right, |d| d.border_r_1())
                .child(self.render_title_bar(window, cx))
                .child(workspace)
                .children(resize_edges(tiling));
        }

        if cfg!(target_os = "macos") {
            if t.is_classic_titlebar() {
                return div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .relative()
                    .p(px(3.))
                    .gap(px(2.))
                    .bg(t.colors.surface)
                    .when_some(t.resolved_font_family(), |d, f| d.font_family(f))
                    .child(self.render_classic_title_bar(window, &t, cx))
                    .child(workspace)
                    .children(t.raised_bevel());
            }
            if t.is_crt_titlebar() {
                return div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .bg(t.colors.background)
                    .border_1()
                    .border_color(t.colors.border)
                    .when_some(t.resolved_font_family(), |d, f| d.font_family(f))
                    .child(self.render_crt_title_bar(window, &t, cx))
                    .child(workspace);
            }
            if t.is_glass_titlebar() {
                return div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .bg(t.colors.background)
                    .when_some(t.resolved_font_family(), |d, f| d.font_family(f))
                    .child(self.render_glass_title_bar(window, &t, cx))
                    .child(workspace);
            }
            return div()
                .size_full()
                .flex()
                .flex_col()
                .bg(t.colors.background)
                .when_some(t.resolved_font_family(), |d, f| d.font_family(f))
                .child(self.render_macos_native_bar(&t, cx))
                .child(workspace);
        }

        div().size_full().flex().flex_col().child(workspace)
    }
}

/// Crisp square bordered CRT window button (`_`, `□`, `×`).
fn crt_window_button(
    id: &'static str,
    icon: &'static str,
    fg: gpui::Hsla,
    t: &theme::Theme,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .w(px(18.))
        .h(px(16.))
        .flex()
        .items_center()
        .justify_center()
        .border_1()
        .border_color(fg)
        .hover(|s| s.bg(t.colors.background).text_color(t.colors.text))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            on_click(window, cx);
        })
        .child(svg().path(icon).size(px(10.)).text_color(fg))
}

/// Glossy Vista-style capsule window button (`–`, `□`, `×`).
fn glass_window_button(
    id: &'static str,
    icon: &'static str,
    is_close: bool,
    t: &theme::Theme,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let base_bg = if is_close {
        rgba(0xc83c3c99)
    } else {
        rgba(0xffffff24)
    };
    let hover_bg = if is_close {
        rgba(0xe84848e6)
    } else {
        rgba(0x7cd4ff59)
    };
    div()
        .id(id)
        .relative()
        .w(if is_close { px(32.) } else { px(24.) })
        .h(px(18.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .bg(base_bg)
        .border_1()
        .border_color(rgba(0xffffff44))
        .hover(move |s| s.bg(hover_bg).border_color(rgba(0xffffff99)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            on_click(window, cx);
        })
        .children(t.gloss_overlay())
        .child(
            svg()
                .path(icon)
                .size(px(11.))
                .text_color(t.colors.titlebar_text),
        )
}

/// Classic 18×16 raised titlebar button (`_`, `□`, `×`).
fn classic_window_button(
    id: &'static str,
    icon: &'static str,
    t: &theme::Theme,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .relative()
        .w(px(18.))
        .h(px(16.))
        .flex()
        .items_center()
        .justify_center()
        .bg(t.colors.surface)
        .hover(|s| s.bg(t.colors.hover))
        .active(|s| s.pt(px(1.)).pl(px(1.)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            on_click(window, cx);
        })
        .child(svg().path(icon).size(px(10.)).text_color(t.colors.text))
        .children(t.raised_bevel())
}

/// A title bar button: an icon in a circle (or raised square in bevel themes).
fn window_button(
    id: &'static str,
    icon: &'static str,
    t: &theme::Theme,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .relative()
        .size(px(24.))
        .flex()
        .items_center()
        .justify_center()
        .when(!t.is_bevel(), |d| d.rounded_full())
        .bg(t.colors.surface)
        .hover(|s| s.bg(t.colors.hover))
        // Not a press on the title bar: don't start moving the window.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            on_click(window, cx);
        })
        .child(svg().path(icon).size(px(14.)).text_color(t.colors.text))
        .children(t.raised_bevel())
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
