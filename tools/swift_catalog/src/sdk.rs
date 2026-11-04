//! macOS SDKs from Apple's software update catalog. Every Command Line Tools
//! product ships SDK packages, and each package's Bom names the SDK it
//! installs, so a ranged read identifies the SDK before the package is hashed.
//! Apple drops old products from its catalog; their entries stay in
//! `macos_sdks.json`.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::sync::LazyLock;

use bootstrap::{Http, download, print_error};
use rayon::prelude::*;
use regex::bytes::Regex;
use serde::{Deserialize, Serialize};

use crate::catalog::{self, Catalog};
use crate::version::Version;
use crate::{Error, Options, sha256_each};

const CATALOG_URL: &str = "https://swscan.apple.com/content/catalogs/others/index-26-15-14-13-12-10.16-10.15-10.14-10.13-10.12-10.11-10.10-10.9-mountainlion-lion-snowleopard-leopard.merged-1.sucatalog";

/// Packages smaller than this are stubs, not SDKs.
const MIN_SDK_PACKAGE_SIZE: u64 = 1_000_000;

/// One entry of `macos_sdks.json`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct MacosSdk {
    posted: String,
    sha256: String,
    strip_prefix: String,
    urls: Vec<String>,
}

/// Apple's software update catalog, reduced to what the catalog needs.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct SoftwareCatalog {
    products: HashMap<String, Product>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Product {
    post_date: Option<plist::Date>,
    #[serde(default)]
    packages: Vec<Package>,
}

#[derive(Deserialize)]
struct Package {
    #[serde(rename = "URL")]
    url: String,
    #[serde(rename = "Size", default)]
    size: u64,
}

/// A Command Line Tools SDK package and the day Apple posted it.
#[derive(Clone, Debug, PartialEq)]
struct SdkPackage {
    url: String,
    posted: String,
}

pub(crate) fn update(http: &Http, path: &Path, options: &Options) -> Result<(), Error> {
    let mut known: Catalog<MacosSdk> = catalog::read(path, options.refresh)?;
    let software: SoftwareCatalog = plist::from_bytes(&download::bytes(http, CATALOG_URL)?)
        .map_err(|source| Error::UnexpectedSoftwareCatalog { source })?;
    let packages = sdk_packages(software);
    eprintln!(
        "{} Command Line Tools SDK packages in Apple's catalog",
        packages.len()
    );

    let found = packages
        .into_par_iter()
        .filter_map(|package| match sdk_version(http, &package.url) {
            Ok(version) => Some((version?, package)),
            Err(error) => {
                print_error(&error);
                None
            }
        })
        .collect();
    let todo: Vec<(Version, SdkPackage)> = newest_per_version(found, &options.macos_sdk_majors)
        .into_iter()
        .filter(|(version, package)| {
            known.get(version).and_then(|sdk| sdk.urls.first()) != Some(&package.url)
        })
        .collect();
    eprintln!(
        "hashing {} macOS SDK packages: {}",
        todo.len(),
        todo.iter()
            .map(|(version, _)| version.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );

    let urls: Vec<String> = todo
        .iter()
        .map(|(_, package)| package.url.clone())
        .collect();
    let mut failed = vec![];
    for ((version, package), hash) in todo.into_iter().zip(sha256_each(http, &urls)) {
        let Some(sha256) = hash else {
            failed.push(version.to_string());
            continue;
        };
        let strip_prefix =
            format!("Payload/Library/Developer/CommandLineTools/SDKs/MacOSX{version}.sdk");
        known.insert(
            version,
            MacosSdk {
                posted: package.posted,
                sha256,
                strip_prefix,
                urls: vec![package.url],
            },
        );
    }
    catalog::write(path, &known)?;
    eprintln!("{} macOS SDKs in {}", known.len(), path.display());
    if failed.is_empty() {
        Ok(())
    } else {
        Err(Error::Unhashed {
            what: "macOS SDK",
            versions: failed,
        })
    }
}

/// Every real Command Line Tools SDK package in the catalog.
fn sdk_packages(catalog: SoftwareCatalog) -> Vec<SdkPackage> {
    catalog
        .products
        .into_values()
        .flat_map(|product| {
            let posted = product
                .post_date
                .map(|date| date.to_xml_format()[..10].to_owned())
                .unwrap_or_default();
            product
                .packages
                .into_iter()
                .filter(|package| {
                    let file = package.url.rsplit('/').next().unwrap_or_default();
                    file.starts_with("CLTools_macOS")
                        && file.ends_with("SDK.pkg")
                        && package.size > MIN_SDK_PACKAGE_SIZE
                })
                .map(move |package| SdkPackage {
                    url: package.url,
                    posted: posted.clone(),
                })
        })
        .collect()
}

/// The newest package for each SDK version with a major in `majors`, when
/// several products ship the same SDK.
fn newest_per_version(found: Vec<(Version, SdkPackage)>, majors: &[u32]) -> Catalog<SdkPackage> {
    let mut newest = Catalog::new();
    for (version, package) in found {
        if !majors.contains(&version.major()) {
            continue;
        }
        let current: Option<&SdkPackage> = newest.get(&version);
        if current.is_none_or(|current| current.posted < package.posted) {
            newest.insert(version, package);
        }
    }
    newest
}

/// The SDK a Command Line Tools SDK package installs, read from its Bom.
fn sdk_version(http: &Http, url: &str) -> Result<Option<Version>, bootstrap::Error> {
    // Newer packages store the Bom (~5 MB) uncompressed, just before the tiny
    // PackageInfo at the end of the file.
    if let Some(version) = sdk_in_bom(&download::range(http, url, "bytes=-8000000")?) {
        return Ok(Some(version));
    }
    // Older ones store it bzip2-compressed at the start of the xar heap.
    static BZIP2_STREAM: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"BZh[1-9]1AY&SY").expect("valid regex"));
    let head = download::range(http, url, "bytes=0-2999999")?;
    let Some(stream) = BZIP2_STREAM.find(&head) else {
        return Ok(None);
    };
    // The range cuts the stream short, so keep whatever decodes before that.
    let mut bom = vec![];
    let _ = bzip2::read::BzDecoder::new(&head[stream.start()..])
        .take(16_000_000)
        .read_to_end(&mut bom);
    Ok(sdk_in_bom(&bom))
}

/// The SDK version named most often in a Bom as `MacOSX<version>.sdk`; the
/// newest wins a tie.
fn sdk_in_bom(bom: &[u8]) -> Option<Version> {
    static SDK_NAME: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"MacOSX(\d+(?:\.\d+)+)\.sdk").expect("valid regex"));
    let mut counts: HashMap<&[u8], usize> = HashMap::new();
    for name in SDK_NAME.captures_iter(bom) {
        *counts.entry(name.get(1)?.as_bytes()).or_default() += 1;
    }
    counts
        .into_iter()
        .filter_map(|(text, count)| Some((count, Version::parse(str::from_utf8(text).ok()?)?)))
        .max()
        .map(|(_, version)| version)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(url: &str, posted: &str) -> SdkPackage {
        SdkPackage {
            url: url.to_owned(),
            posted: posted.to_owned(),
        }
    }

    #[test]
    fn the_most_named_sdk_in_a_bom_wins() {
        let bom =
            b"..MacOSX15.4.sdk/usr..MacOSX15.4.sdk/lib..MacOSX15.sdk..MacOSX.sdk..MacOSX26.5.sdk";
        assert_eq!(sdk_in_bom(bom), Version::parse("15.4"));
        assert_eq!(
            sdk_in_bom(b"MacOSX15.4.sdk MacOSX15.5.sdk"),
            Version::parse("15.5")
        );
        assert_eq!(sdk_in_bom(b"no sdk here"), None);
    }

    #[test]
    fn keeps_the_newest_package_of_each_wanted_sdk() {
        let v = |text| Version::parse(text).unwrap();
        let found = vec![
            (v("15.4"), package("old", "2025-01-01")),
            (v("15.4"), package("new", "2025-03-01")),
            (v("15.4"), package("older", "2024-12-01")),
            (v("14.5"), package("unwanted", "2025-01-01")),
            (v("26.5"), package("tahoe", "2026-05-01")),
        ];
        let newest = newest_per_version(found, &[15, 26]);
        assert_eq!(
            newest.into_iter().collect::<Vec<_>>(),
            [
                (v("15.4"), package("new", "2025-03-01")),
                (v("26.5"), package("tahoe", "2026-05-01")),
            ]
        );
    }

    #[test]
    fn picks_real_sdk_packages_from_the_catalog() {
        let plist = br#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>Products</key><dict>
  <key>052-1</key><dict>
    <key>PostDate</key><date>2025-03-31T17:00:00Z</date>
    <key>Packages</key><array>
      <dict><key>URL</key><string>https://swcdn.example/CLTools_macOS_SDK.pkg</string><key>Size</key><integer>50000000</integer></dict>
      <dict><key>URL</key><string>https://swcdn.example/CLTools_macOSLMOS_SDK.pkg</string><key>Size</key><integer>5000</integer></dict>
      <dict><key>URL</key><string>https://swcdn.example/CLTools_Executables.pkg</string><key>Size</key><integer>90000000</integer></dict>
    </array>
  </dict>
  <key>041-2</key><dict><key>ServerMetadataURL</key><string>x</string></dict>
</dict></dict></plist>"#;
        let catalog: SoftwareCatalog = plist::from_bytes(plist).unwrap();
        assert_eq!(
            sdk_packages(catalog),
            [package(
                "https://swcdn.example/CLTools_macOS_SDK.pkg",
                "2025-03-31"
            )]
        );
    }
}
