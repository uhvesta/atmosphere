//! What a pasted image must look like before it's saved.

use humata2_grpc::humata::v1 as pb;
use pb::invalid_field::Problem;
use tonic::Status;

use crate::status;

/// The largest image PasteImage takes.
pub(crate) const PASTE_MAX_BYTES: usize = 32 << 20;

/// Checks `data` is an image of `format` and returns the file extension it
/// is saved with.
pub(crate) fn check(format: i32, data: &[u8]) -> Result<&'static str, Status> {
    let format = match pb::ImageFormat::try_from(format) {
        Ok(pb::ImageFormat::Unspecified) | Err(_) => {
            return Err(status::invalid_field(
                "format",
                Problem::UnknownValue,
                "the pasted image has no format this daemon knows",
                "send PNG, JPEG, GIF, WebP, TIFF or HEIC; convert other formats before pasting",
            ));
        }
        Ok(format) => format,
    };
    if data.is_empty() {
        return Err(status::missing("data"));
    }
    if data.len() > PASTE_MAX_BYTES {
        return Err(status::invalid_field(
            "data",
            Problem::TooLong,
            format!(
                "the pasted image is {} MiB; at most {} MiB can be pasted",
                data.len().div_ceil(1 << 20),
                PASTE_MAX_BYTES >> 20
            ),
            "paste a smaller image, or copy the file to the machine and type its path",
        ));
    }
    let (extension, matches) = match format {
        pb::ImageFormat::Png => ("png", data.starts_with(b"\x89PNG\r\n\x1a\n")),
        pb::ImageFormat::Jpeg => ("jpg", data.starts_with(b"\xff\xd8\xff")),
        pb::ImageFormat::Gif => (
            "gif",
            data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a"),
        ),
        pb::ImageFormat::Webp => (
            "webp",
            data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP".as_slice()),
        ),
        pb::ImageFormat::Tiff => (
            "tiff",
            data.starts_with(b"II*\0") || data.starts_with(b"MM\0*"),
        ),
        pb::ImageFormat::Heic => ("heic", is_heif(data)),
        pb::ImageFormat::Unspecified => ("", false),
    };
    if !matches {
        return Err(status::invalid_field(
            "data",
            Problem::BadFormat,
            format!(
                "the pasted data isn't a {} image",
                format.as_str_name().trim_start_matches("IMAGE_FORMAT_")
            ),
            "send the format the image really is; if the copy came out damaged, copy it again",
        ));
    }
    Ok(extension)
}

/// An ISO base media file whose brand is one of HEIF's.
fn is_heif(data: &[u8]) -> bool {
    const BRANDS: [&[u8]; 8] = [
        b"heic", b"heix", b"hevc", b"hevx", b"heim", b"heis", b"mif1", b"msf1",
    ];
    data.get(4..8) == Some(b"ftyp".as_slice())
        && data
            .get(8..12)
            .is_some_and(|brand| BRANDS.contains(&brand))
}
