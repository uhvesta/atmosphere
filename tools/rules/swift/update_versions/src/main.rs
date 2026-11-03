//! Regenerates `tools/rules/swift/swift_versions.json` and `macos_sdks.json`.
//!
//! ```text
//! bazel run //tools/rules/swift:update_versions
//! bazel run //tools/rules/swift:update_versions -- --only sdk
//! bazel run //tools/rules/swift:update_versions -- --refresh --jobs 6
//! ```
//!
//! Swift releases come from swift.org's release list. swift.org publishes only
//! the Static Linux SDK's checksum, so the macOS `.pkg` and the Ubuntu tarballs
//! are hashed by streaming them; nothing is written to disk.
//!
//! macOS SDKs come from Apple's software update catalog. Every Command Line
//! Tools product ships SDK packages, and each package's Bom names the SDK it
//! installs, so a ranged read identifies the SDK before the package is hashed.
//!
//! Entries already in the files are kept unless `--refresh` is given, so a
//! normal run only downloads what is new. Apple drops old products from its
//! catalog; their entries stay in `macos_sdks.json`.

use std::collections::{BTreeMap, HashMap};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use clap::{Parser, ValueEnum};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const RELEASES_URL: &str = "https://www.swift.org/api/v1/install/releases.json";
const CATALOG_URL: &str = "https://swscan.apple.com/content/catalogs/others/index-26-15-14-13-12-10.16-10.15-10.14-10.13-10.12-10.11-10.10-10.9-mountainlion-lion-snowleopard-leopard.merged-1.sucatalog";
const USER_AGENT: &str = "monty-oxide-update-versions";

#[derive(Parser)]
#[command(about = "Regenerate the Swift release and macOS SDK catalogs in tools/rules/swift.")]
struct Args {
    /// Update only one of the two files.
    #[arg(long)]
    only: Option<Catalog>,
    /// Re-hash entries that are already in the files.
    #[arg(long)]
    refresh: bool,
    /// Downloads to run in parallel.
    #[arg(long, default_value_t = 4)]
    jobs: usize,
    /// Oldest Swift release to include.
    #[arg(long, default_value = "6.2")]
    min_swift: String,
    /// macOS SDK major versions to include.
    #[arg(long, value_delimiter = ',', default_value = "15,26")]
    macos_sdk_majors: Vec<u32>,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum Catalog {
    Swift,
    Sdk,
}

type Error = Box<dyn std::error::Error + Send + Sync>;
type Result<T> = std::result::Result<T, Error>;

fn main() {
    let args = Args::parse();
    let workspace = std::env::var_os("BUILD_WORKSPACE_DIRECTORY")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().expect("current directory"));
    let directory = workspace.join("tools/rules/swift");
    let http = Http::new();

    let mut ok = true;
    if args.only != Some(Catalog::Sdk) {
        ok &= report(update_swift(
            &args,
            &http,
            &directory.join("swift_versions.json"),
        ));
    }
    if args.only != Some(Catalog::Swift) {
        ok &= report(update_sdks(
            &args,
            &http,
            &directory.join("macos_sdks.json"),
        ));
    }
    std::process::exit(if ok { 0 } else { 1 });
}

fn report(result: Result<()>) -> bool {
    match result {
        Ok(()) => true,
        Err(error) => {
            eprintln!("error: {error}");
            false
        }
    }
}

// MARK: - HTTP

struct Http {
    agent: ureq::Agent,
}

impl Http {
    fn new() -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(30)))
            .user_agent(USER_AGENT)
            .build()
            .into();
        Http { agent }
    }

    /// The whole body, or one byte range of it (`bytes=-N` for the last N).
    fn get(&self, url: &str, range: Option<&str>) -> Result<Vec<u8>> {
        let mut request = self.agent.get(url);
        if let Some(range) = range {
            request = request.header("Range", range);
        }
        let body = request
            .call()
            .map_err(|e| format!("GET {url}: {e}"))?
            .into_body();
        Ok(body.into_with_config().limit(u64::MAX).read_to_vec()?)
    }

    /// SHA-256 of the body, streamed.
    fn sha256(&self, url: &str) -> Result<String> {
        let body = self
            .agent
            .get(url)
            .call()
            .map_err(|e| format!("GET {url}: {e}"))?
            .into_body();
        let mut reader = body.into_with_config().limit(u64::MAX).reader();
        let mut hasher = Sha256::new();
        let mut buffer = vec![0; 1 << 20];
        loop {
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        Ok(hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect())
    }
}

/// Runs `work` over `items` on `jobs` threads; results keep the input order.
fn parallel_map<T: Sync, R: Send>(
    items: &[T],
    jobs: usize,
    work: impl Fn(&T) -> R + Sync,
) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let results = Mutex::new((0..items.len()).map(|_| None).collect::<Vec<Option<R>>>());
    std::thread::scope(|scope| {
        for _ in 0..jobs.clamp(1, items.len().max(1)) {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(index) else { break };
                    let result = work(item);
                    results.lock().unwrap()[index] = Some(result);
                }
            });
        }
    });
    results
        .into_inner()
        .unwrap()
        .into_iter()
        .map(Option::unwrap)
        .collect()
}

// MARK: - Files

fn version_key(version: &str) -> Vec<u32> {
    version
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

fn read_catalog<T: for<'de> Deserialize<'de>>(
    path: &Path,
    refresh: bool,
) -> Result<BTreeMap<String, T>> {
    if refresh || !path.exists() {
        return Ok(BTreeMap::new());
    }
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}

/// Writes entries in numeric version order, two-space indented, so the file
/// is byte-identical whichever machine regenerates it.
fn write_catalog<T: Serialize>(path: &Path, entries: &BTreeMap<String, T>) -> Result<()> {
    let mut keys: Vec<&String> = entries.keys().collect();
    keys.sort_by_key(|key| version_key(key));
    let mut object = serde_json::Map::new();
    for key in keys {
        object.insert(key.clone(), serde_json::to_value(&entries[key])?);
    }
    let mut text = serde_json::to_string_pretty(&object)?;
    text.push('\n');
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, text)?;
    std::fs::rename(temporary, path)?;
    Ok(())
}

// MARK: - Swift releases

/// Fields are in alphabetical order, which is the order they are written in.
#[derive(Clone, Deserialize, Serialize)]
struct SwiftRelease {
    linux_aarch64: String,
    linux_x86_64: String,
    macos: String,
    static_sdk: String,
    static_sdk_version: String,
    xcode: String,
}

/// Must match the URL templates in extensions.bzl.
fn swift_artifacts(v: &str) -> [(&'static str, String); 3] {
    let base = format!("https://download.swift.org/swift-{v}-release");
    [
        (
            "macos",
            format!("{base}/xcode/swift-{v}-RELEASE/swift-{v}-RELEASE-osx.pkg"),
        ),
        (
            "linux_aarch64",
            format!(
                "{base}/ubuntu2204-aarch64/swift-{v}-RELEASE/swift-{v}-RELEASE-ubuntu22.04-aarch64.tar.gz"
            ),
        ),
        (
            "linux_x86_64",
            format!("{base}/ubuntu2204/swift-{v}-RELEASE/swift-{v}-RELEASE-ubuntu22.04.tar.gz"),
        ),
    ]
}

#[derive(Deserialize)]
struct Release {
    name: String,
    #[serde(default)]
    xcode: String,
    #[serde(default)]
    platforms: Vec<Platform>,
}

#[derive(Deserialize)]
struct Platform {
    #[serde(default)]
    name: String,
    #[serde(default)]
    platform: String,
    checksum: Option<String>,
    version: Option<String>,
}

fn update_swift(args: &Args, http: &Http, path: &Path) -> Result<()> {
    let mut known: BTreeMap<String, SwiftRelease> = read_catalog(path, args.refresh)?;
    let releases: Vec<Release> = serde_json::from_slice(&http.get(RELEASES_URL, None)?)?;
    let mut failed = vec![];

    for release in releases {
        let name = release.name;
        if !name.starts_with("6.")
            || version_key(&name) < version_key(&args.min_swift)
            || known.contains_key(&name)
        {
            continue;
        }
        let static_sdk = release
            .platforms
            .iter()
            .find(|p| p.platform == "static-sdk");
        let has_ubuntu = release.platforms.iter().any(|p| p.name == "Ubuntu 22.04");
        let (Some(static_sha), Some(static_version), true) = (
            static_sdk.and_then(|p| p.checksum.clone()),
            static_sdk.and_then(|p| p.version.clone()),
            has_ubuntu,
        ) else {
            eprintln!("skipping Swift {name}: no Ubuntu 22.04 or Static SDK artifact");
            continue;
        };

        eprintln!("hashing Swift {name}");
        let artifacts = swift_artifacts(&name);
        let hashes = parallel_map(&artifacts, args.jobs, |(_, url)| http.sha256(url));
        let mut digest: HashMap<&str, String> = HashMap::new();
        for ((key, _), hash) in artifacts.iter().zip(hashes) {
            match hash {
                Ok(sha) => {
                    digest.insert(key, sha);
                }
                Err(error) => eprintln!("  {error}"),
            }
        }
        if digest.len() != artifacts.len() {
            failed.push(name);
            continue;
        }
        known.insert(
            name,
            SwiftRelease {
                linux_aarch64: digest.remove("linux_aarch64").unwrap(),
                linux_x86_64: digest.remove("linux_x86_64").unwrap(),
                macos: digest.remove("macos").unwrap(),
                static_sdk: static_sha,
                static_sdk_version: static_version,
                xcode: release.xcode,
            },
        );
        // Save after every release so an interrupted run keeps its progress.
        write_catalog(path, &known)?;
    }
    write_catalog(path, &known)?;
    eprintln!("{} Swift releases in {}", known.len(), path.display());
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!("failed to hash Swift {}; rerun to retry", failed.join(", ")).into())
    }
}

// MARK: - macOS SDKs

#[derive(Clone, Deserialize, Serialize)]
struct MacosSdk {
    posted: String,
    sha256: String,
    strip_prefix: String,
    urls: Vec<String>,
}

/// The SDK version that appears most often in `data` as `MacOSX<version>.sdk`.
fn most_common_sdk(data: &[u8]) -> Option<String> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut rest = data;
    while let Some(start) = find(rest, b"MacOSX") {
        rest = &rest[start + 6..];
        let version: Vec<u8> = rest
            .iter()
            .take_while(|b| b.is_ascii_digit() || **b == b'.')
            .copied()
            .collect();
        if version.len() > 1
            && rest[version.len()..].starts_with(b"sdk")
            && version.last() == Some(&b'.')
        {
            let version = String::from_utf8_lossy(&version[..version.len() - 1]).into_owned();
            if version.contains('.') {
                *counts.entry(version).or_default() += 1;
            }
        }
    }
    counts
        .into_iter()
        .max_by_key(|(version, count)| (*count, version_key(version)))
        .map(|(version, _)| version)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// The SDK a Command Line Tools SDK package installs, read from its Bom.
fn sdk_version(http: &Http, url: &str) -> Result<Option<String>> {
    // Newer packages store the Bom (~5 MB) uncompressed, just before the tiny
    // PackageInfo at the end of the file.
    if let Some(version) = most_common_sdk(&http.get(url, Some("bytes=-8000000"))?) {
        return Ok(Some(version));
    }
    // Older ones store it bzip2-compressed at the start of the xar heap.
    let head = http.get(url, Some("bytes=0-2999999"))?;
    let Some(start) = head
        .windows(10)
        .position(|w| w.starts_with(b"BZh") && &w[4..10] == b"1AY&SY")
    else {
        return Ok(None);
    };
    // The range cuts the stream short, so keep whatever decodes before that.
    let mut bom = vec![];
    let _ = bzip2::read::BzDecoder::new(&head[start..])
        .take(16_000_000)
        .read_to_end(&mut bom);
    Ok(most_common_sdk(&bom))
}

fn update_sdks(args: &Args, http: &Http, path: &Path) -> Result<()> {
    let mut known: BTreeMap<String, MacosSdk> = read_catalog(path, args.refresh)?;
    let catalog = plist::Value::from_reader(io::Cursor::new(http.get(CATALOG_URL, None)?))?;
    let products = catalog
        .as_dictionary()
        .and_then(|c| c.get("Products"))
        .and_then(plist::Value::as_dictionary)
        .ok_or("unexpected software update catalog format")?;

    // Every real SDK package (the 5 KB ones are stubs) with its post date.
    let mut candidates: Vec<(String, String)> = vec![];
    for product in products.values().filter_map(plist::Value::as_dictionary) {
        let posted = product
            .get("PostDate")
            .and_then(plist::Value::as_date)
            .map(|date| date.to_xml_format()[..10].to_string())
            .unwrap_or_default();
        for package in product
            .get("Packages")
            .and_then(plist::Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(package) = package.as_dictionary() else {
                continue;
            };
            let url = package
                .get("URL")
                .and_then(plist::Value::as_string)
                .unwrap_or_default();
            let size = package
                .get("Size")
                .and_then(plist::Value::as_unsigned_integer)
                .unwrap_or(0);
            let file = url.rsplit('/').next().unwrap_or_default();
            if file.starts_with("CLTools_macOS") && file.ends_with("SDK.pkg") && size > 1_000_000 {
                candidates.push((url.to_string(), posted.clone()));
            }
        }
    }
    eprintln!(
        "{} Command Line Tools SDK packages in Apple's catalog",
        candidates.len()
    );

    let versions = parallel_map(&candidates, args.jobs, |(url, _)| sdk_version(http, url));
    // The newest package wins when several products ship the same SDK.
    let mut newest: BTreeMap<String, (String, String)> = BTreeMap::new();
    for ((url, posted), version) in candidates.into_iter().zip(versions) {
        let version = match version {
            Ok(Some(version)) => version,
            Ok(None) => continue,
            Err(error) => {
                eprintln!("  {error}");
                continue;
            }
        };
        if !args.macos_sdk_majors.contains(&version_key(&version)[0]) {
            continue;
        }
        if newest
            .get(&version)
            .is_some_and(|(_, current)| *current >= posted)
        {
            continue;
        }
        newest.insert(version, (url, posted));
    }

    let todo: Vec<(String, String, String)> = newest
        .into_iter()
        .filter(|(version, (url, _))| {
            known.get(version).and_then(|sdk| sdk.urls.first()) != Some(url)
        })
        .map(|(version, (url, posted))| (version, url, posted))
        .collect();
    eprintln!(
        "hashing {} macOS SDK packages: {}",
        todo.len(),
        todo.iter()
            .map(|(v, _, _)| v.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    let hashes = parallel_map(&todo, args.jobs, |(_, url, _)| http.sha256(url));
    let mut failed = vec![];
    for ((version, url, posted), hash) in todo.into_iter().zip(hashes) {
        match hash {
            Ok(sha256) => {
                known.insert(
                    version.clone(),
                    MacosSdk {
                        posted,
                        sha256,
                        strip_prefix: format!(
                            "Payload/Library/Developer/CommandLineTools/SDKs/MacOSX{version}.sdk"
                        ),
                        urls: vec![url],
                    },
                );
            }
            Err(error) => {
                eprintln!("  {error}");
                failed.push(version);
            }
        }
    }
    write_catalog(path, &known)?;
    eprintln!("{} macOS SDKs in {}", known.len(), path.display());
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "failed to hash macOS SDK {}; rerun to retry",
            failed.join(", ")
        )
        .into())
    }
}
