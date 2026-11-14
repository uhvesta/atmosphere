//! Images sent with a message: kept in the session's folder, each by a file
//! name the history records, and read back for the backend and the app.

use std::path::{Path, PathBuf};

use crate::error::AgentError;
use crate::event::{EventId, Image, SessionId};

/// The largest image taken, in bytes; the backends refuse larger ones.
pub(crate) const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;

/// The folder of `session_dir`'s images.
pub(crate) fn folder(session_dir: &Path) -> PathBuf {
    session_dir.join("images")
}

/// The file extension for `media_type`, or None for a kind not taken.
fn extension(media_type: &str) -> Option<&'static str> {
    match media_type {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        _ => None,
    }
}

/// The media type of an image file by its name.
pub(crate) fn media_type(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("jpg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => "image/png",
    }
}

/// Writes `images` into the session's folder, and gives their names.
pub(crate) fn save(
    session: &SessionId,
    session_dir: &Path,
    images: &[Image],
) -> Result<Vec<String>, AgentError> {
    if images.is_empty() {
        return Ok(Vec::new());
    }
    let mut checked = Vec::with_capacity(images.len());
    for image in images {
        let extension =
            extension(&image.media_type).ok_or_else(|| AgentError::UnsupportedImage {
                session: session.clone(),
                media_type: image.media_type.clone(),
            })?;
        if image.data.len() > MAX_IMAGE_BYTES {
            return Err(AgentError::ImageTooLarge {
                session: session.clone(),
                bytes: image.data.len(),
                max: MAX_IMAGE_BYTES,
            });
        }
        checked.push((format!("{}.{extension}", EventId::random()), &image.data));
    }
    let folder = folder(session_dir);
    std::fs::create_dir_all(&folder).map_err(|source| AgentError::SaveImage {
        path: folder.clone(),
        source,
    })?;
    let mut names = Vec::with_capacity(checked.len());
    for (name, data) in checked {
        let path = folder.join(&name);
        std::fs::write(&path, data).map_err(|source| AgentError::SaveImage { path, source })?;
        names.push(name);
    }
    Ok(names)
}

/// Removes the images `names` that [`save`] wrote for a message that wasn't
/// taken after all. One already gone, or that can't be removed, is left: the
/// message's refusal is what the sender needs to hear.
pub(crate) fn discard(session_dir: &Path, names: &[String]) {
    let folder = folder(session_dir);
    for name in names.iter().filter(|name| plain(name)) {
        let _ = std::fs::remove_file(folder.join(name));
    }
}

/// Whether `name` is a plain file name, as [`save`] makes, and so can't
/// reach outside the images folder.
fn plain(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.contains(['/', '\\'])
        && Path::new(name).file_name().is_some_and(|file| file == name)
}

/// The image `name` of the session in `session_dir`.
pub(crate) fn read(
    session: &SessionId,
    session_dir: &Path,
    name: &str,
) -> Result<Image, AgentError> {
    if !plain(name) {
        return Err(AgentError::UnknownImage {
            session: session.clone(),
            name: name.to_owned(),
        });
    }
    let path = folder(session_dir).join(name);
    let data = std::fs::read(&path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            AgentError::UnknownImage {
                session: session.clone(),
                name: name.to_owned(),
            }
        } else {
            AgentError::ReadImage {
                path: path.clone(),
                source,
            }
        }
    })?;
    Ok(Image {
        media_type: media_type(&path).to_owned(),
        data,
    })
}

/// `bytes` as standard base64, padded, as the backends take images.
pub(crate) fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard_encoding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn a_saved_image_reads_back_as_it_was() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let session = SessionId::new("s1");
        let image = Image {
            media_type: "image/png".into(),
            data: vec![1, 2, 3],
        };
        let names = save(&session, dir.path(), std::slice::from_ref(&image)).expect("saved");
        assert_eq!(names.len(), 1);
        let read = read(&session, dir.path(), &names[0]).expect("read");
        assert_eq!(read, image);
    }

    #[test]
    fn discarded_images_are_gone_and_the_others_stay() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let session = SessionId::new("s1");
        let image = |byte| Image {
            media_type: "image/png".into(),
            data: vec![byte],
        };
        let dropped = save(&session, dir.path(), &[image(1)]).expect("saved");
        let kept = save(&session, dir.path(), &[image(2)]).expect("saved");
        discard(dir.path(), &dropped);
        // A second time finds nothing, and is no failure.
        discard(dir.path(), &dropped);
        assert!(matches!(
            read(&session, dir.path(), &dropped[0]),
            Err(AgentError::UnknownImage { .. })
        ));
        assert_eq!(
            read(&session, dir.path(), &kept[0]).expect("read"),
            image(2)
        );
    }

    #[test]
    fn a_kind_the_backends_cant_read_is_refused() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let image = Image {
            media_type: "image/tiff".into(),
            data: vec![1],
        };
        let saved = save(&SessionId::new("s1"), dir.path(), &[image]);
        assert!(matches!(saved, Err(AgentError::UnsupportedImage { .. })));
    }

    #[test]
    fn a_name_reaching_outside_the_folder_is_unknown() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        std::fs::write(dir.path().join("secret.png"), b"x").expect("written");
        let read = read(&SessionId::new("s1"), dir.path(), "../secret.png");
        assert!(matches!(read, Err(AgentError::UnknownImage { .. })));
    }
}
