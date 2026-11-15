//! What can be attached from the clipboard: copied files (e.g. from the
//! Finder), else an image (e.g. a screenshot).
//!
//! GPUI only reads text and images from the clipboard, and prefers text: a
//! file copied in the Finder comes out as its name. On macOS the pasteboard
//! is read directly instead. Linux file managers put copied files in the
//! text too, as `file://` URIs or paths (see [`files_from_text`]).

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
    let files = item.entries().iter().find_map(|entry| match entry {
        ClipboardEntry::String(text) => files_from_text(text.text()),
        _ => None,
    });
    if let Some(files) = files {
        return Some(Attachable::Files(files));
    }
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

/// The files that clipboard text names, if it names only existing files:
/// one per line, as a `file://` URI or an absolute path. GNOME Files
/// starts the text with `x-special/nautilus-clipboard` and `copy` (or
/// `cut`) lines, which are skipped.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn files_from_text(text: &str) -> Option<Vec<PathBuf>> {
    let mut lines = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .peekable();
    if lines.next_if_eq(&"x-special/nautilus-clipboard").is_some() {
        lines.next_if(|l| matches!(*l, "copy" | "cut"));
    }
    let files = lines
        .map(|line| {
            let path = match line.strip_prefix("file://") {
                // Only local files: `file:///path` or `file://localhost/path`.
                Some(rest) => {
                    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
                    PathBuf::from(percent_decode(rest)?)
                }
                None => PathBuf::from(line),
            };
            (path.is_absolute() && path.exists()).then_some(path)
        })
        .collect::<Option<Vec<_>>>()?;
    (!files.is_empty()).then_some(files)
}

/// Decodes `%XX` escapes (as in URIs); `None` if malformed or not UTF-8.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn percent_decode(text: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut rest = text.as_bytes();
    while let Some((&byte, tail)) = rest.split_first() {
        if byte == b'%' {
            let hex = std::str::from_utf8(tail.get(..2)?).ok()?;
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
            rest = &tail[2..];
        } else {
            bytes.push(byte);
            rest = tail;
        }
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode() {
        assert_eq!(
            percent_decode("/a%20b/%C3%A9t%C3%A9").as_deref(),
            Some("/a b/été")
        );
        assert_eq!(percent_decode("/plain").as_deref(), Some("/plain"));
        assert_eq!(percent_decode("/bad%2"), None);
        assert_eq!(percent_decode("/bad%zz"), None);
    }

    #[test]
    fn files_in_text() {
        let dir = std::env::temp_dir().join(format!("solilog-clipboard-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a b.txt");
        let b = dir.join("été.png");
        std::fs::write(&a, "").unwrap();
        std::fs::write(&b, "").unwrap();
        let uri = |p: &PathBuf| {
            let s = p
                .to_str()
                .unwrap()
                .replace(' ', "%20")
                .replace('é', "%C3%A9");
            format!("file://{s}")
        };

        // GNOME Files, other file managers (URIs), and plain paths.
        let nautilus = format!(
            "x-special/nautilus-clipboard\ncopy\n{}\n{}\n",
            uri(&a),
            uri(&b)
        );
        assert_eq!(files_from_text(&nautilus), Some(vec![a.clone(), b.clone()]));
        let uris = format!("{}\r\n{}", uri(&a), uri(&b));
        assert_eq!(files_from_text(&uris), Some(vec![a.clone(), b.clone()]));
        assert_eq!(files_from_text(a.to_str().unwrap()), Some(vec![a.clone()]));

        // Anything else is just text.
        assert_eq!(files_from_text("hello"), None);
        assert_eq!(files_from_text(""), None);
        assert_eq!(files_from_text("relative/path"), None);
        let missing = format!("{}\n{}", uri(&a), dir.join("missing").display());
        assert_eq!(files_from_text(&missing), None);
        assert_eq!(files_from_text("file://host/share/x"), None);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
