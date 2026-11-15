//! What can go wrong showing a document's images and diagrams. Each error is
//! shown inline where the image or diagram would be, so its message tells the
//! reader of the document what to change.

use std::error::Error;
use std::path::PathBuf;

/// An image the document links to could not be shown.
#[derive(Debug, thiserror::Error)]
pub enum ImageError {
    /// The link leaves the document's folder, which images may not do.
    #[error(
        "the image {path} is outside the document's folder {base}; move the image into that folder or link a copy that is inside it"
    )]
    OutsideBase { path: PathBuf, base: PathBuf },
    /// A relative link, in a view that was given no folder to resolve it in.
    #[error(
        "the image {url} is a relative link, but this view has no document folder to find it in; open the document from its file, or link the image by an absolute path"
    )]
    NoBase { url: String },
    /// The file could not be read.
    #[error("could not read the image {path}: {source}. Check that the file exists and is readable, or fix the link")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The bytes are not in a format the view can show.
    #[error(
        "{name} is not an image this view can show; convert it to PNG, JPEG, GIF, WebP, BMP, TIFF, ICO or SVG, or link another file"
    )]
    Unsupported { name: String },
    /// The bytes claim a format but don't decode as it.
    #[error("could not decode the image {name}: {source}. The file may be damaged or cut short; export it again")]
    Decode {
        name: String,
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
    /// A remote image, in a view that only shows local ones.
    #[error(
        "the remote image {url} was not loaded, because this view only shows images stored with the document; download it next to the document and link the local copy"
    )]
    RemoteBlocked { url: String },
    /// A `data:` link that isn't base64 image data.
    #[error(
        "the image link {url} is a data URL this view can't read; it must be `data:image/...;base64,...`, so embed the image again or link its file"
    )]
    DataUrl { url: String },
}

/// A Mermaid diagram could not be drawn.
#[derive(Debug, thiserror::Error)]
pub enum DiagramError {
    /// The source is not valid Mermaid.
    #[error("the diagram is not valid Mermaid: {source}. Fix the source; the diagram is drawn again as soon as it is valid")]
    Syntax {
        /// The 1-based line within the diagram source the parser stopped at.
        line: Option<u32>,
        /// The 1-based column within that line, in characters.
        column: Option<u32>,
        #[source]
        source: mermaid_rs_renderer::ParseError,
    },
    /// The renderer panicked: a bug in it, not in the source.
    #[error(
        "the Mermaid renderer failed on this diagram; this is a renderer bug rather than a mistake in the source, so simplify the diagram to work around it"
    )]
    Crashed,
    /// The renderer's SVG could not be drawn into an image.
    #[error("the diagram was laid out but its picture could not be drawn: {source}. Simplify the diagram, for example by removing unusual characters from labels")]
    Draw {
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
}
