//! GitHub's API and releases, and the SHA-256 GitHub publishes for each
//! asset.

use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::Error;
use crate::download;
use crate::http::Http;
use crate::lockfile::Pin;
use crate::platform::Platform;

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
    fn api<T: DeserializeOwned>(&self, path: &str) -> Result<T, Error> {
        let url = format!("https://api.github.com/{path}");
        let authorization = self.token.as_ref().map(|token| format!("Bearer {token}"));
        let mut headers = vec![("Accept", "application/vnd.github+json")];
        if let Some(authorization) = &authorization {
            headers.push(("Authorization", authorization));
        }
        download::json(self.http, &url, &headers).map_err(|error| match error {
            Error::RateLimited { url, wait } if self.token.is_none() => {
                Error::AnonymousRateLimit { url, wait }
            }
            error => error,
        })
    }

    /// The release with this tag.
    pub(crate) fn release(&self, repo: &str, tag: &str) -> Result<Release, Error> {
        self.api(&format!("repos/{repo}/releases/tags/{tag}"))
    }
}

#[derive(Deserialize)]
pub(crate) struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    /// `sha256:<hex>`, published for assets uploaded since mid-2025.
    digest: Option<String>,
}

impl Release {
    /// Pins the asset whose name matches.
    pub(crate) fn pin(
        &self,
        github: &GitHub,
        component: &str,
        platform: Platform,
        matches: impl Fn(&str) -> bool,
    ) -> Result<Pin, Error> {
        let asset = self
            .assets
            .iter()
            .find(|asset| matches(&asset.name))
            .ok_or_else(|| Error::MissingAsset {
                component: component.to_owned(),
                tag: self.tag_name.clone(),
                platform: platform.name(),
            })?;
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
