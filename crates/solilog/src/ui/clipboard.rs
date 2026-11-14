//! What can be attached from the clipboard: copied files (e.g. from the
//! Finder), else an image (e.g. a screenshot).
//!
//! GPUI only reads text and images from the clipboard, and prefers text: a
//! file copied in the Finder comes out as its name. On macOS the pasteboard
//! is read directly instead.

use std::path::PathBuf;

/// What the clipboard holds that can be attached.
pub enum Attachable {
    Files(Vec<PathBuf>),
    /// Image data, with the file extension of its format.
    Image(Vec<u8>, &'static str),
}

#[cfg(target_os = "macos")]
pub fn read(_: &gpui::App) -> Option<Attachable> {
    use objc2_app_kit::{
        NSPasteboard, NSPasteboardTypeFileURL, NSPasteboardTypePNG, NSPasteboardTypeTIFF,
    };
    use objc2_foundation::{NSString, NSURL};

    let pasteboard = NSPasteboard::generalPasteboard();
    let files: Vec<PathBuf> = pasteboard
        .pasteboardItems()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let url = item.stringForType(unsafe { NSPasteboardTypeFileURL })?;
                    let url = NSURL::URLWithString(&url)?;
                    url.isFileURL()
                        .then(|| url.path())
                        .flatten()
                        .map(|path| PathBuf::from(path.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    if !files.is_empty() {
        return Some(Attachable::Files(files));
    }
    let jpeg = NSString::from_str("public.jpeg");
    let formats = [
        (unsafe { NSPasteboardTypePNG }, "png"),
        (&*jpeg, "jpg"),
        (unsafe { NSPasteboardTypeTIFF }, "tiff"),
    ];
    formats.into_iter().find_map(|(ty, ext)| {
        let data = pasteboard.dataForType(ty)?;
        (!data.is_empty()).then(|| Attachable::Image(data.to_vec(), ext))
    })
}

#[cfg(not(target_os = "macos"))]
pub fn read(cx: &gpui::App) -> Option<Attachable> {
    use gpui::{ClipboardEntry, ImageFormat};

    let item = cx.read_from_clipboard()?;
    item.entries().iter().find_map(|entry| match entry {
        ClipboardEntry::Image(image) => {
            let ext = match image.format {
                ImageFormat::Png => "png",
                ImageFormat::Jpeg => "jpg",
                ImageFormat::Webp => "webp",
                ImageFormat::Gif => "gif",
                ImageFormat::Svg => "svg",
                ImageFormat::Bmp => "bmp",
                ImageFormat::Tiff => "tiff",
            };
            Some(Attachable::Image(image.bytes.clone(), ext))
        }
        ClipboardEntry::String(_) => None,
    })
}
