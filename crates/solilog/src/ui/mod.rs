//! The GPUI desktop app.

pub mod actions;
mod assets;
mod clipboard;
pub mod keymap;
mod picker;
mod status_icon;
mod text_input;
mod theme;
mod tooltip;
mod window_frame;
mod workspace;

use anyhow::Result;
use gpui::{
    App, Application, Bounds, SharedString, TitlebarOptions, WindowBounds, WindowOptions, point,
    prelude::*, px, size,
};
use solilog_core::Db;

use actions::workspace::Quit;
use window_frame::WindowFrame;
use workspace::Workspace;

const APP_NAME: &str = "Solilog";
/// Identifies the app to Linux desktops: the name of its `.desktop` file
/// (and the macOS bundle identifier).
const APP_ID: &str = "dev.solilog.Solilog";
/// Label of `attachment::Reveal` in the app, naming the system's file manager.
const SHOW_IN_FILE_MANAGER: &str = if cfg!(target_os = "macos") {
    "Show in Finder"
} else {
    "Show in File Manager"
};

pub fn run(db: Db) -> Result<()> {
    Application::new()
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            assets::load_fonts(cx);
            let saved_theme = db.theme().ok().flatten();
            theme::init(saved_theme.as_deref(), cx);
            keymap::load(cx);
            cx.on_action(|_: &Quit, cx| cx.quit());

            let bounds = Bounds::centered(None, size(px(1000.), px(700.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    // Below this the sidebar, list and footer no longer fit.
                    window_min_size: Some(size(px(600.), px(400.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some(SharedString::from(APP_NAME)),
                        appears_transparent: cfg!(target_os = "macos"),
                        traffic_light_position: cfg!(target_os = "macos")
                            .then(|| point(px(10.), px(9.))),
                    }),
                    app_id: Some(APP_ID.to_string()),
                    window_decorations: Some(window_frame::decorations(
                        theme::current(cx).uses_custom_titlebar(),
                    )),
                    ..Default::default()
                },
                |window, cx| {
                    // gpui ignores `TitlebarOptions::title` on Wayland.
                    window.set_window_title(APP_NAME);
                    let workspace = cx.new(|cx| Workspace::new(db, window, cx));
                    cx.new(|_| WindowFrame::new(workspace))
                },
            )
            .expect("failed to open window");
            // Apply macOS traffic-light visibility once the window exists.
            theme::apply_window_chrome(&theme::current(cx));

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
