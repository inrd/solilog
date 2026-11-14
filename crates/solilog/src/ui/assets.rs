//! Assets compiled into the binary (icons), for `svg().path(…)`.

use std::borrow::Cow;

use anyhow::Result;
use gpui::{AssetSource, SharedString};

/// Paths of the icons, as given to `svg().path(…)`.
pub const PAPERCLIP: &str = "icons/paperclip.svg";
pub const WINDOW_MINIMIZE: &str = "icons/window-minimize.svg";
pub const WINDOW_MAXIMIZE: &str = "icons/window-maximize.svg";
pub const WINDOW_RESTORE: &str = "icons/window-restore.svg";
pub const WINDOW_CLOSE: &str = "icons/window-close.svg";

const ASSETS: &[(&str, &[u8])] = &[
    (
        PAPERCLIP,
        include_bytes!("../../assets/icons/paperclip.svg"),
    ),
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
