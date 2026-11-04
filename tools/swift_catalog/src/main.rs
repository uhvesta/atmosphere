//! Regenerates `bazel/toolchains/swift/swift_versions.json` and `macos_sdks.json`.
//!
//! ```text
//! bazel run //tools/swift_catalog
//! bazel run //tools/swift_catalog -- --only sdk
//! bazel run //tools/swift_catalog -- --refresh --jobs 6
//! ```
//!
//! Entries already in the files are kept unless `--refresh` is given, so a
//! normal run only downloads what is new.

mod catalog;
mod sdk;
mod swift;
mod version;

use std::path::PathBuf;

use bootstrap::{Http, download, print_error};
use clap::{Parser, ValueEnum};
use rayon::prelude::*;

use crate::version::Version;

const TARGET: &str = "//tools/swift_catalog";

#[derive(Parser)]
#[command(about = "Regenerate the Swift release and macOS SDK catalogs in bazel/toolchains/swift.")]
struct Args {
    /// Update only one of the two files.
    #[arg(long)]
    only: Option<Which>,
    /// Downloads to run in parallel.
    #[arg(long, default_value_t = 4)]
    jobs: usize,
    #[command(flatten)]
    options: Options,
}

/// What both updates take from the command line.
#[derive(clap::Args)]
struct Options {
    /// Re-hash entries that are already in the files.
    #[arg(long)]
    refresh: bool,
    /// Oldest Swift release to include.
    #[arg(long, default_value = "6.2", value_parser = parse_version)]
    min_swift: Version,
    /// macOS SDK major versions to include.
    #[arg(long, value_delimiter = ',', default_value = "15,26")]
    macos_sdk_majors: Vec<u32>,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum Which {
    Swift,
    Sdk,
}

/// Why a catalog could not be regenerated. The fields record what went wrong;
/// the message tells the user what to do.
#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)]
    Bootstrap(#[from] bootstrap::Error),

    #[error("could not start {jobs} download threads; retry with a lower --jobs")]
    Threads {
        jobs: usize,
        #[source]
        source: rayon::ThreadPoolBuildError,
    },

    #[error(
        "{} is not a valid catalog; restore it from git, or rerun with --refresh to rebuild it from scratch",
        path.display()
    )]
    InvalidCatalog {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error(
        "Apple's software update catalog is not in the format this tool expects; Apple may have changed it, so update tools/swift_catalog/src/sdk.rs"
    )]
    UnexpectedSoftwareCatalog {
        #[source]
        source: plist::Error,
    },

    #[error(
        "could not hash {what} {}; the errors above say why. Rerun to retry: entries already hashed are kept",
        versions.join(", ")
    )]
    Unhashed {
        what: &'static str,
        versions: Vec<String>,
    },
}

fn main() {
    let args = Args::parse();
    if let Err(error) = setup(args.jobs) {
        bootstrap::exit_with(&error);
    }
    let directory = match bootstrap::workspace(TARGET) {
        Ok(workspace) => workspace.join("bazel/toolchains/swift"),
        Err(error) => bootstrap::exit_with(&error),
    };
    let http = Http::default();

    // Run both, so a failure in one doesn't hide the other's progress.
    let mut ok = true;
    if args.only != Some(Which::Sdk) {
        let path = directory.join("swift_versions.json");
        ok &= report(swift::update(&http, &path, &args.options));
    }
    if args.only != Some(Which::Swift) {
        let path = directory.join("macos_sdks.json");
        ok &= report(sdk::update(&http, &path, &args.options));
    }
    std::process::exit(if ok { 0 } else { 1 });
}

/// Sizes the thread pool that runs downloads in parallel.
fn setup(jobs: usize) -> Result<(), Error> {
    rayon::ThreadPoolBuilder::new()
        .num_threads(jobs.max(1))
        .build_global()
        .map_err(|source| Error::Threads { jobs, source })
}

fn report(result: Result<(), Error>) -> bool {
    result.map_err(|error| print_error(&error)).is_ok()
}

fn parse_version(text: &str) -> Result<Version, String> {
    Version::parse(text).ok_or_else(|| format!("{text:?} is not a dotted version such as 6.2"))
}

/// The SHA-256 of each URL, downloaded in parallel. A failed download is
/// printed and leaves `None`, so it doesn't stop the others.
fn sha256_each(http: &Http, urls: &[String]) -> Vec<Option<String>> {
    urls.par_iter()
        .map(|url| {
            download::sha256(http, url)
                .map_err(|error| print_error(&error))
                .ok()
        })
        .collect()
}
