//! Assets compiled into the binary (icons), for `svg().path(…)`.

use std::borrow::Cow;

use anyhow::Result;
use gpui::{AssetSource, SharedString};

/// Paths of the icons, as given to `svg().path(…)`.
pub const PAPERCLIP: &str = "icons/paperclip.svg";

const ASSETS: &[(&str, &[u8])] = &[(
    PAPERCLIP,
    include_bytes!("../../assets/icons/paperclip.svg"),
)];

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
