//! Images a document links to: found relative to the document's folder,
//! never outside it, loaded and decoded off the UI thread, and kept until the
//! view goes. One that can't be shown draws as a placeholder naming why.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use gpui_kit::{Image, ImageFormat, RenderImage, SvgRenderer};

use crate::error::ImageError;

/// Where a link's image comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ImageLocation {
    File(PathBuf),
    Data(Vec<u8>),
    /// A remote image, left for GPUI to fetch; only when the view allows it.
    Remote(String),
}

/// Find the image `url` names, relative to `base`.
pub(crate) fn locate(url: &str, base: Option<&Path>, allow_remote: bool) -> Result<ImageLocation, ImageError> {
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return if allow_remote {
            Ok(ImageLocation::Remote(url.to_owned()))
        } else {
            Err(ImageError::RemoteBlocked { url: url.to_owned() })
        };
    }
    if lower.starts_with("data:") {
        return data_url(url).map(ImageLocation::Data);
    }
    let path = url.strip_prefix("file://").unwrap_or(url);
    let path = PathBuf::from(percent_decode(path.split(['?', '#']).next().unwrap_or(path)));
    let Some(base) = base else {
        return Err(ImageError::NoBase { url: url.to_owned() });
    };
    let base = normalize(base);
    let joined = normalize(&base.join(&path));
    if !joined.starts_with(&base) {
        return Err(ImageError::OutsideBase { path: joined, base });
    }
    // A link inside the folder can still lead out of it through a symlink.
    if let (Ok(real), Ok(real_base)) = (joined.canonicalize(), base.canonicalize())
        && !real.starts_with(&real_base)
    {
        return Err(ImageError::OutsideBase {
            path: real,
            base: real_base,
        });
    }
    Ok(ImageLocation::File(joined))
}

/// `path` with `.` and `..` resolved without touching the file system.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    out
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%'
            && let Some(hex) = text.get(at + 1..at + 3)
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            out.push(byte);
            at += 3;
            continue;
        }
        out.push(bytes[at]);
        at += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_owned())
}

/// The bytes of a `data:image/...;base64,...` URL.
fn data_url(url: &str) -> Result<Vec<u8>, ImageError> {
    let bad = || ImageError::DataUrl { url: shorten(url) };
    let (header, payload) = url.split_once(',').ok_or_else(bad)?;
    let header = header.to_ascii_lowercase();
    if !header.starts_with("data:image/") || !header.ends_with(";base64") {
        return Err(bad());
    }
    base64(payload).ok_or_else(bad)
}

fn base64(text: &str) -> Option<Vec<u8>> {
    let value = |byte: u8| -> Option<u32> {
        Some(match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32)
    };
    let digits: Vec<u8> = text
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace() && *byte != b'=')
        .collect();
    let mut out = Vec::with_capacity(digits.len() * 3 / 4);
    for chunk in digits.chunks(4) {
        let mut word = 0u32;
        for (ix, byte) in chunk.iter().enumerate() {
            word |= value(*byte)? << (18 - 6 * ix);
        }
        let bytes = word.to_be_bytes();
        match chunk.len() {
            4 => out.extend_from_slice(&bytes[1..4]),
            3 => out.extend_from_slice(&bytes[1..3]),
            2 => out.push(bytes[1]),
            _ => return None,
        }
    }
    Some(out)
}

fn shorten(url: &str) -> String {
    match url.char_indices().nth(48) {
        Some((at, _)) => format!("{}…", &url[..at]),
        None => url.to_owned(),
    }
}

/// The format of image bytes, by their first bytes.
pub(crate) fn sniff(bytes: &[u8]) -> Option<ImageFormat> {
    let starts = |magic: &[u8]| bytes.starts_with(magic);
    Some(if starts(b"\x89PNG\r\n\x1a\n") {
        ImageFormat::Png
    } else if starts(&[0xff, 0xd8, 0xff]) {
        ImageFormat::Jpeg
    } else if starts(b"GIF87a") || starts(b"GIF89a") {
        ImageFormat::Gif
    } else if starts(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        ImageFormat::Webp
    } else if starts(b"BM") {
        ImageFormat::Bmp
    } else if starts(b"II*\0") || starts(b"MM\0*") {
        ImageFormat::Tiff
    } else if starts(&[0, 0, 1, 0]) {
        ImageFormat::Ico
    } else {
        let head = String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]).to_ascii_lowercase();
        let head = head.trim_start_matches('\u{feff}').trim_start();
        if head.starts_with("<svg") || (head.starts_with("<?xml") && head.contains("<svg")) {
            ImageFormat::Svg
        } else {
            return None;
        }
    })
}

/// Read and decode the image at `location`. Runs off the UI thread.
pub(crate) fn load(
    location: &ImageLocation,
    name: &str,
    svg: &SvgRenderer,
) -> Result<Arc<RenderImage>, ImageError> {
    let bytes = match location {
        ImageLocation::File(path) => std::fs::read(path).map_err(|source| ImageError::Read {
            path: path.clone(),
            source,
        })?,
        ImageLocation::Data(bytes) => bytes.clone(),
        ImageLocation::Remote(url) => return Err(ImageError::RemoteBlocked { url: url.clone() }),
    };
    let format = sniff(&bytes).ok_or_else(|| ImageError::Unsupported { name: name.to_owned() })?;
    if format == ImageFormat::Svg {
        // Twice the pixels, so the image stays sharp on a Retina screen.
        return svg
            .render_single_frame(&bytes, 2.0)
            .map_err(|source| ImageError::Decode {
                name: name.to_owned(),
                source: Box::new(source),
            });
    }
    Image::from_bytes(format, bytes)
        .to_image_data(svg.clone())
        .map_err(|source| ImageError::Decode {
            name: name.to_owned(),
            source: source.into(),
        })
}

/// The picture shown for an image that can't be: a frame with a crossed-out
/// picture and the link's name.
pub(crate) fn placeholder_svg(name: &str, foreground: &str, border: &str, font: &str) -> String {
    let name: String = name
        .chars()
        .take(40)
        .map(|ch| match ch {
            '<' => "&lt;".to_owned(),
            '>' => "&gt;".to_owned(),
            '&' => "&amp;".to_owned(),
            '"' => "&quot;".to_owned(),
            other => other.to_string(),
        })
        .collect();
    let font = font.replace(['"', '<', '>', '&'], "");
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="240" height="96" viewBox="0 0 240 96">
<rect x="1" y="1" width="238" height="94" rx="6" fill="none" stroke="{border}" stroke-width="2" stroke-dasharray="6 4"/>
<g transform="translate(104 14)" fill="none" stroke="{foreground}" stroke-width="2">
<rect x="0" y="0" width="32" height="26" rx="3"/>
<path d="M3 22 L12 13 L19 19 L24 15 L29 21"/>
<circle cx="23" cy="8" r="3"/>
<path d="M-2 30 L34 -4"/>
</g>
<text x="120" y="70" text-anchor="middle" font-family="{font}" font-size="12" fill="{foreground}">{name}</text>
</svg>"##
    )
}

/// An image's state in the cache.
#[derive(Clone)]
pub(crate) enum ImageEntry {
    Loading,
    Ready(Arc<RenderImage>),
    /// It can't be shown; the placeholder to show instead, when one could be
    /// drawn, and why.
    Missing {
        placeholder: Option<Arc<RenderImage>>,
        error: Arc<ImageError>,
    },
}

/// Counts of image work, to prove a cached image isn't loaded again.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImageStats {
    /// Images read and decoded.
    pub loads: u64,
    /// Lookups the cache answered.
    pub hits: u64,
}

#[derive(Default)]
struct Images {
    entries: HashMap<String, ImageEntry>,
    stats: ImageStats,
}

/// The images of one view, by link.
#[derive(Clone, Default)]
pub(crate) struct ImageCache {
    inner: Arc<Mutex<Images>>,
}

impl ImageCache {
    fn lock(&self) -> MutexGuard<'_, Images> {
        // A panic while holding the lock leaves the map whole; keep using it.
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn get(&self, url: &str) -> Option<ImageEntry> {
        let mut images = self.lock();
        let entry = images.entries.get(url).cloned();
        if matches!(entry, Some(ImageEntry::Ready(_) | ImageEntry::Missing { .. })) {
            images.stats.hits += 1;
        }
        entry
    }

    /// Mark `url` as loading; false when it is already known.
    pub(crate) fn begin(&self, url: &str) -> bool {
        let mut images = self.lock();
        if images.entries.contains_key(url) {
            return false;
        }
        images.entries.insert(url.to_owned(), ImageEntry::Loading);
        true
    }

    pub(crate) fn finish(&self, url: &str, entry: ImageEntry, loaded: bool) {
        let mut images = self.lock();
        if loaded {
            images.stats.loads += 1;
        }
        images.entries.insert(url.to_owned(), entry);
    }

    pub(crate) fn stats(&self) -> ImageStats {
        self.lock().stats
    }

    /// The entry for `url` without counting a lookup, for labels.
    pub(crate) fn peek(&self, url: &str) -> Option<ImageEntry> {
        self.lock().entries.get(url).cloned()
    }

    /// Forget every image, so each loads again; for a new document folder.
    pub(crate) fn clear(&self) {
        let mut images = self.lock();
        images.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SM-3
    #[test]
    fn a_relative_link_resolves_inside_the_documents_folder() {
        let base = Path::new("/docs/plan");
        assert_eq!(
            locate("img/a%20b.png", Some(base), false).ok(),
            Some(ImageLocation::File(PathBuf::from("/docs/plan/img/a b.png")))
        );
        assert_eq!(
            locate("./x/../y.svg?raw#top", Some(base), false).ok(),
            Some(ImageLocation::File(PathBuf::from("/docs/plan/y.svg")))
        );
    }

    /// SM-3
    #[test]
    fn a_link_out_of_the_folder_is_refused() {
        let base = Path::new("/docs/plan");
        assert!(matches!(
            locate("../../etc/passwd", Some(base), false),
            Err(ImageError::OutsideBase { .. })
        ));
        assert!(matches!(
            locate("/etc/hosts", Some(base), false),
            Err(ImageError::OutsideBase { .. })
        ));
    }

    /// SM-3
    #[test]
    fn remote_images_load_only_when_allowed() {
        assert!(matches!(
            locate("https://example.com/a.png", None, false),
            Err(ImageError::RemoteBlocked { .. })
        ));
        assert_eq!(
            locate("https://example.com/a.png", None, true).ok(),
            Some(ImageLocation::Remote("https://example.com/a.png".into()))
        );
    }

    /// SM-3
    #[test]
    fn a_base64_data_url_decodes_to_its_bytes() {
        assert_eq!(
            locate("data:image/png;base64,iVBORw0KGgo=", None, false).ok(),
            Some(ImageLocation::Data(b"\x89PNG\r\n\x1a\n".to_vec()))
        );
        assert!(matches!(
            locate("data:text/plain,hello", None, false),
            Err(ImageError::DataUrl { .. })
        ));
    }

    /// SM-3
    #[test]
    fn formats_are_told_apart_by_their_first_bytes() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\nrest"), Some(ImageFormat::Png));
        assert_eq!(sniff(b"\xef\xbb\xbf<?xml version=\"1.0\"?><svg>"), Some(ImageFormat::Svg));
        assert_eq!(sniff(b"  <svg xmlns=\"\"></svg>"), Some(ImageFormat::Svg));
        assert_eq!(sniff(b"plain text"), None);
    }
}
