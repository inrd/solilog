//! Assets compiled into the binary (icons), for `svg().path(…)`.

use std::borrow::Cow;

use anyhow::Result;
use gpui::{App, AssetSource, SharedString};

/// Registers bundled fonts (such as `W95FA` for the MMX theme) with GPUI's
/// text system so themes can reference them by `font_family`.
pub fn load_fonts(cx: &App) {
    const W95FA: &[u8] = include_bytes!("../../assets/fonts/W95F.otf");
    if let Err(err) = cx.text_system().add_fonts(vec![Cow::Borrowed(W95FA)]) {
        eprintln!("loading bundled fonts: {err:#}");
    }
}

/// Paths of the icons, as given to `svg().path(…)`.
pub const PAPERCLIP: &str = "icons/paperclip.svg";
pub const CLOCK: &str = "icons/clock.svg";
pub const PLUS: &str = "icons/plus.svg";
pub const CHEVRON_RIGHT: &str = "icons/chevron-right.svg";
pub const CHEVRON_DOWN: &str = "icons/chevron-down.svg";
pub const WINDOW_MINIMIZE: &str = "icons/window-minimize.svg";
pub const WINDOW_MAXIMIZE: &str = "icons/window-maximize.svg";
pub const WINDOW_RESTORE: &str = "icons/window-restore.svg";
pub const WINDOW_CLOSE: &str = "icons/window-close.svg";

const PIXEL_PAPERCLIP: &str = "icons/pixel/paperclip.svg";
const PIXEL_CLOCK: &str = "icons/pixel/clock.svg";
const PIXEL_PLUS: &str = "icons/pixel/plus.svg";
const PIXEL_CHEVRON_RIGHT: &str = "icons/pixel/chevron-right.svg";
const PIXEL_CHEVRON_DOWN: &str = "icons/pixel/chevron-down.svg";
const PIXEL_WINDOW_MINIMIZE: &str = "icons/pixel/window-minimize.svg";
const PIXEL_WINDOW_MAXIMIZE: &str = "icons/pixel/window-maximize.svg";
const PIXEL_WINDOW_RESTORE: &str = "icons/pixel/window-restore.svg";
const PIXEL_WINDOW_CLOSE: &str = "icons/pixel/window-close.svg";

/// Returns the pixel-art variant of `default_path` when `pixel` is true.
pub fn icon_for(default_path: &'static str, pixel: bool) -> &'static str {
    if !pixel {
        return default_path;
    }
    match default_path {
        PAPERCLIP => PIXEL_PAPERCLIP,
        CLOCK => PIXEL_CLOCK,
        PLUS => PIXEL_PLUS,
        CHEVRON_RIGHT => PIXEL_CHEVRON_RIGHT,
        CHEVRON_DOWN => PIXEL_CHEVRON_DOWN,
        WINDOW_MINIMIZE => PIXEL_WINDOW_MINIMIZE,
        WINDOW_MAXIMIZE => PIXEL_WINDOW_MAXIMIZE,
        WINDOW_RESTORE => PIXEL_WINDOW_RESTORE,
        WINDOW_CLOSE => PIXEL_WINDOW_CLOSE,
        other => other,
    }
}

const ASSETS: &[(&str, &[u8])] = &[
    (
        CHEVRON_RIGHT,
        include_bytes!("../../assets/icons/chevron-right.svg"),
    ),
    (
        CHEVRON_DOWN,
        include_bytes!("../../assets/icons/chevron-down.svg"),
    ),
    (
        PAPERCLIP,
        include_bytes!("../../assets/icons/paperclip.svg"),
    ),
    (PLUS, include_bytes!("../../assets/icons/plus.svg")),
    (CLOCK, include_bytes!("../../assets/icons/clock.svg")),
    (
        WINDOW_MINIMIZE,
        include_bytes!("../../assets/icons/window-minimize.svg"),
    ),
    (
        WINDOW_MAXIMIZE,
        include_bytes!("../../assets/icons/window-maximize.svg"),
    ),
    (
        WINDOW_RESTORE,
        include_bytes!("../../assets/icons/window-restore.svg"),
    ),
    (
        WINDOW_CLOSE,
        include_bytes!("../../assets/icons/window-close.svg"),
    ),
    (
        PIXEL_CHEVRON_RIGHT,
        include_bytes!("../../assets/icons/pixel/chevron-right.svg"),
    ),
    (
        PIXEL_CHEVRON_DOWN,
        include_bytes!("../../assets/icons/pixel/chevron-down.svg"),
    ),
    (
        PIXEL_PAPERCLIP,
        include_bytes!("../../assets/icons/pixel/paperclip.svg"),
    ),
    (
        PIXEL_PLUS,
        include_bytes!("../../assets/icons/pixel/plus.svg"),
    ),
    (
        PIXEL_CLOCK,
        include_bytes!("../../assets/icons/pixel/clock.svg"),
    ),
    (
        PIXEL_WINDOW_MINIMIZE,
        include_bytes!("../../assets/icons/pixel/window-minimize.svg"),
    ),
    (
        PIXEL_WINDOW_MAXIMIZE,
        include_bytes!("../../assets/icons/pixel/window-maximize.svg"),
    ),
    (
        PIXEL_WINDOW_RESTORE,
        include_bytes!("../../assets/icons/pixel/window-restore.svg"),
    ),
    (
        PIXEL_WINDOW_CLOSE,
        include_bytes!("../../assets/icons/pixel/window-close.svg"),
    ),
];

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ASSETS
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ASSETS
            .iter()
            .filter(|(p, _)| p.starts_with(path))
            .map(|(p, _)| (*p).into())
            .collect())
    }
}
