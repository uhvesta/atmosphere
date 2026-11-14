//! Writes the Swift for Humata 2's core, and the C header and module map
//! it builds on, from the core's built static library:
//!
//! ```text
//! humata2-bindgen <library.a> <out-dir>
//! ```

use std::process::ExitCode;

use camino::Utf8PathBuf;
use uniffi_bindgen::bindings::{SwiftBindingsOptions, generate_swift_bindings};

#[derive(Debug, thiserror::Error)]
enum BindgenError {
    #[error("usage: humata2-bindgen <library.a> <out-dir>")]
    Usage,
    #[error("{path:?} isn't a UTF-8 path. Build from a folder whose path is UTF-8.")]
    NotUtf8 { path: std::path::PathBuf },
    #[error(
        "couldn't write Swift for {library}: {source}. Check it's the core built with its ffi feature."
    )]
    Generate {
        library: Utf8PathBuf,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

fn utf8(arg: std::ffi::OsString) -> Result<Utf8PathBuf, BindgenError> {
    Utf8PathBuf::from_path_buf(arg.into()).map_err(|path| BindgenError::NotUtf8 { path })
}

fn run() -> Result<(), BindgenError> {
    let mut args = std::env::args_os().skip(1);
    let (Some(library), Some(out_dir), None) = (args.next(), args.next(), args.next()) else {
        return Err(BindgenError::Usage);
    };
    let library = utf8(library)?;
    let out_dir = utf8(out_dir)?;
    generate_swift_bindings(SwiftBindingsOptions {
        generate_swift_sources: true,
        generate_headers: true,
        generate_modulemap: false,
        source: library.clone(),
        out_dir,
        xcframework: false,
        module_name: None,
        modulemap_filename: None,
        metadata_no_deps: true,
        link_frameworks: Vec::new(),
        config: None,
    })
    .map_err(|source| BindgenError::Generate {
        library,
        source: source.into(),
    })
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
