//! The GPUI desktop app.

pub mod actions;
mod assets;
mod clipboard;
pub mod keymap;
mod picker;
mod status_icon;
mod text_input;
mod theme;
mod window_frame;
mod workspace;

use anyhow::Result;
use gpui::{
    App, Application, Bounds, SharedString, TitlebarOptions, WindowBounds, WindowDecorations,
    WindowOptions, prelude::*, px, size,
};
use solilog_core::Db;

use actions::workspace::Quit;
use window_frame::WindowFrame;
use workspace::Workspace;

const APP_NAME: &str = "Solilog";
/// Identifies the app to Linux desktops: the name of its `.desktop` file
/// (and the macOS bundle identifier).
const APP_ID: &str = "dev.solilog.Solilog";

pub fn run(db: Db) -> Result<()> {
    Application::new()
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            use_dark_appearance();
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
                    app_id: Some(APP_ID.to_string()),
                    // Drawn by the system when it can; else by `WindowFrame`.
                    window_decorations: Some(WindowDecorations::Server),
                    ..Default::default()
                },
                |window, cx| {
                    let workspace = cx.new(|cx| Workspace::new(db, window, cx));
                    cx.new(|_| WindowFrame::new(workspace))
                },
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

/// The app is dark whatever the system setting, so its title bar (and other
/// native parts) must be too.
#[cfg(target_os = "macos")]
fn use_dark_appearance() {
    use objc2_app_kit::{NSAppearance, NSAppearanceNameDarkAqua, NSApplication};

    let Some(mtm) = objc2::MainThreadMarker::new() else {
        return;
    };
    let dark = NSAppearance::appearanceNamed(unsafe { NSAppearanceNameDarkAqua });
    NSApplication::sharedApplication(mtm).setAppearance(dark.as_deref());
}

#[cfg(not(target_os = "macos"))]
fn use_dark_appearance() {}
