//! The GPUI desktop app.

pub mod actions;
pub mod keymap;
mod picker;
mod status_icon;
mod text_input;
mod theme;
mod workspace;

use anyhow::Result;
use gpui::{
    App, Application, Bounds, SharedString, TitlebarOptions, WindowBounds, WindowOptions,
    prelude::*, px, size,
};
use solilog_core::Db;

use actions::workspace::Quit;
use workspace::Workspace;

const APP_NAME: &str = "Solilog";

pub fn run(db: Db) -> Result<()> {
    Application::new().run(move |cx: &mut App| {
        keymap::load(cx);
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
            |window, cx| cx.new(|cx| Workspace::new(db, window, cx)),
        )
        .expect("failed to open window");

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
