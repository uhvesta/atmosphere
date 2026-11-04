//! GitHub's API and releases, and the SHA-256 GitHub publishes for each
//! asset.

use bootstrap::download;
use bootstrap::{Http, Pin, Platform};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::Error;

pub(crate) struct GitHub<'a> {
    http: &'a Http,
    token: Option<String>,
}

impl<'a> GitHub<'a> {
    /// `token`, if any, is sent with API requests to raise the rate limit.
    pub(crate) fn new(http: &'a Http, token: Option<String>) -> Self {
        GitHub { http, token }
    }

    /// `path` under the API root, parsed as JSON.
    pub(crate) fn api<T: DeserializeOwned>(&self, path: &str) -> Result<T, Error> {
        let url = format!("https://api.github.com/{path}");
        let authorization = self.token.as_ref().map(|token| format!("Bearer {token}"));
        let mut headers = vec![("Accept", "application/vnd.github+json")];
        if let Some(authorization) = &authorization {
            headers.push(("Authorization", authorization));
        }
        download::json(self.http, &url, &headers).map_err(|error| match error {
            bootstrap::Error::RateLimited { url, wait } if self.token.is_none() => {
                Error::AnonymousRateLimit { url, wait }
            }
            error => error.into(),
        })
    }

    /// The release with this tag.
    pub(crate) fn release(&self, repo: &str, tag: &str) -> Result<Release, Error> {
        self.api(&format!("repos/{repo}/releases/tags/{tag}"))
    }
}

#[derive(Deserialize)]
pub(crate) struct Release {
    pub(crate) tag_name: String,
    #[serde(default)]
    pub(crate) draft: bool,
    #[serde(default)]
    pub(crate) prerelease: bool,
    pub(crate) assets: Vec<Asset>,
}

#[derive(Deserialize)]
pub(crate) struct Asset {
    pub(crate) name: String,
    browser_download_url: String,
    /// `sha256:<hex>`, published for assets uploaded since mid-2025.
    digest: Option<String>,
}

impl Release {
    /// Pins the one asset whose name matches.
    pub(crate) fn pin(
        &self,
        github: &GitHub,
        component: &str,
        platform: Platform,
        matches: impl Fn(&str) -> bool,
    ) -> Result<Pin, Error> {
        let found: Vec<&Asset> = self
            .assets
            .iter()
            .filter(|asset| matches(&asset.name))
            .collect();
        let asset = match found[..] {
            [asset] => asset,
            [] => {
                return Err(Error::MissingAsset {
                    component: component.to_owned(),
                    tag: self.tag_name.clone(),
                    platform: platform.name(),
                });
            }
            _ => {
                return Err(Error::AmbiguousAsset {
                    component: component.to_owned(),
                    tag: self.tag_name.clone(),
                    platform: platform.name(),
                    names: found
                        .iter()
                        .map(|a| a.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                });
            }
        };
        let url = asset.browser_download_url.clone();
        let sha256 = match asset
            .digest
            .as_deref()
            .and_then(|d| d.strip_prefix("sha256:"))
        {
            Some(hex) => hex.to_owned(),
            None => download::sha256(github.http, &url)?,
        };
        Ok(Pin::new(component, platform.name(), sha256, url))
    }
}
