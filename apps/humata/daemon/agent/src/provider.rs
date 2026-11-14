//! Model providers a session pays for with a key of the person's own: the
//! Anthropic API, an OpenAI-compatible server, or OpenCode Go.
//!
//! A session names its provider in its backend config, as JSON:
//! `{"provider": {"kind": "opencode_go", "api_key_variable": "OPENCODE_API_KEY"}}`.
//! That names the host's variable holding the key, never the key: it is
//! read from the host's variables each time it's needed, so it is in no
//! session record and no file.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::Model;
use crate::env::Env;
use crate::local_model::{self, LocalModelError};

/// Where each kind is served unless the provider says otherwise.
const ANTHROPIC_URL: &str = "https://api.anthropic.com";
const OPENAI_URL: &str = "https://api.openai.com/v1";
const OPENCODE_GO_URL: &str = "https://opencode.ai/inference/go";

/// The version of Anthropic's API its models are listed with.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// What Humata calls itself to a provider that asks its clients to say.
const USER_AGENT: &str = "humata/1.0";

/// The header OpenCode Go takes a conversation's id in, to route its
/// requests alike and cache its prompt.
const OPENCODE_SESSION: &str = "x-opencode-session";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderKind {
    /// Anthropic's API, at its `/v1/messages`.
    Anthropic,
    /// Any server with OpenAI's API; its base URL usually ends in `/v1`.
    Openai,
    /// OpenCode Go, which serves OpenAI's and Anthropic's APIs under one
    /// address.
    OpencodeGo,
}

impl ProviderKind {
    fn name(self) -> &'static str {
        match self {
            Self::Anthropic => "the Anthropic API",
            Self::Openai => "an OpenAI-compatible server",
            Self::OpencodeGo => "OpenCode Go",
        }
    }
}

/// Which of OpenAI's two APIs a provider is asked with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WireApi {
    /// `/chat/completions`, which nearly every server has.
    #[default]
    Completions,
    /// `/responses`.
    Responses,
}

/// A provider as a session's backend config names it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Provider {
    pub(crate) kind: ProviderKind,
    /// Where it is served; the kind's own address if not given.
    #[serde(default)]
    pub(crate) base_url: Option<String>,
    /// The host's variable that holds its key.
    pub(crate) api_key_variable: String,
    /// For Copilot, on a kind with OpenAI's API.
    #[serde(default)]
    pub(crate) wire_api: Option<WireApi>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error(
        "the provider's key is read from the variable {variable} of the host this session runs on, and it isn't set there. Set it in Settings › Environments for that host, then send again or list the models again; or start a new session with another provider"
    )]
    NoVariable { variable: String },

    #[error(
        "{backend} can't use {kind} as its provider: it takes {takes}. Start a new session with a provider of that kind, or with another agent"
    )]
    Unsupported {
        backend: &'static str,
        kind: &'static str,
        takes: &'static str,
    },

    #[error(
        "couldn't reach the provider at {url}. Check the provider's base URL in Settings › Environments, and that the host this session runs on is online and can reach it, then send again"
    )]
    Unreachable {
        url: String,
        #[source]
        source: Box<ureq::Error>,
    },

    #[error(
        "the provider at {url} refused the key in the variable {variable} ({status}): {body}. Check that the variable holds a key for this provider that hasn't been revoked, and that the provider's base URL is the one the key is for; set the variable again in Settings › Environments, then send again"
    )]
    KeyRefused {
        url: String,
        variable: String,
        status: u16,
        body: String,
    },

    #[error(
        "the provider at {url} says the account behind the key in {variable} has no credit left (402): {body}. Add credit or raise the spending limit with the provider, then send again; a new session won't get around it"
    )]
    NoCredit {
        url: String,
        variable: String,
        body: String,
    },

    #[error(
        "the provider at {url} is limiting the key in {variable} (429): {body}. Wait for the limit to reset, then send again; sending again at once or starting a new session won't get around it. If it says a quota or the credit is used up, not a rate, add credit with the provider instead"
    )]
    Limited {
        url: String,
        variable: String,
        body: String,
    },
}

impl ProviderError {
    /// What the provider's `status` to a request made with the key in
    /// `variable` means, when it's about the key or its account.
    pub(crate) fn refusal(url: &str, variable: &str, status: u16, body: &str) -> Option<Self> {
        let (url, variable, body) = (url.to_owned(), variable.to_owned(), body.to_owned());
        match status {
            401 | 403 => Some(Self::KeyRefused {
                url,
                variable,
                status,
                body,
            }),
            402 => Some(Self::NoCredit {
                url,
                variable,
                body,
            }),
            429 => Some(Self::Limited {
                url,
                variable,
                body,
            }),
            _ => None,
        }
    }
}

/// How a request to a provider says who asks: the key sent as a bearer
/// token, if it is, and the headers besides.
pub(crate) struct Access {
    pub(crate) bearer: Option<String>,
    pub(crate) headers: Vec<(String, String)>,
}

impl Provider {
    /// Where it is served, without a trailing slash.
    fn base(&self) -> &str {
        let preset = match self.kind {
            ProviderKind::Anthropic => ANTHROPIC_URL,
            ProviderKind::Openai => OPENAI_URL,
            ProviderKind::OpencodeGo => OPENCODE_GO_URL,
        };
        self.base_url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .unwrap_or(preset)
            .trim_end_matches('/')
    }

    /// Its key, from the host's variables.
    pub(crate) fn key(&self, env: &Env) -> Result<String, ProviderError> {
        env.get(&self.api_key_variable)
            .filter(|key| !key.is_empty())
            .ok_or_else(|| ProviderError::NoVariable {
                variable: self.api_key_variable.clone(),
            })
    }

    /// The base of its OpenAI API, which `/chat/completions` goes under,
    /// if it has one.
    fn openai_root(&self) -> Option<String> {
        match self.kind {
            ProviderKind::Openai => Some(self.base().to_owned()),
            ProviderKind::OpencodeGo => Some(format!("{}/openai/v1", self.base())),
            ProviderKind::Anthropic => None,
        }
    }

    /// The base of its OpenAI API; an error for `backend` if it has none.
    pub(crate) fn openai_base(&self, backend: &'static str) -> Result<String, ProviderError> {
        self.openai_root().ok_or(ProviderError::Unsupported {
            backend,
            kind: self.kind.name(),
            takes: "an OpenAI-compatible server or OpenCode Go",
        })
    }

    /// The base of Anthropic's own API, as Claude Code takes it in
    /// `ANTHROPIC_BASE_URL`: None when it's Anthropic's own address. An
    /// error for `backend` if this isn't Anthropic's API.
    pub(crate) fn anthropic_base(
        &self,
        backend: &'static str,
    ) -> Result<Option<String>, ProviderError> {
        match self.kind {
            ProviderKind::Anthropic => {
                Ok(Some(self.base().to_owned()).filter(|base| base != ANTHROPIC_URL))
            }
            ProviderKind::Openai | ProviderKind::OpencodeGo => Err(ProviderError::Unsupported {
                backend,
                kind: self.kind.name(),
                takes: "the Anthropic API",
            }),
        }
    }

    /// Where it lists its models.
    pub(crate) fn models_url(&self) -> String {
        match self.kind {
            ProviderKind::Anthropic => format!("{}/v1/models?limit=1000", self.base()),
            ProviderKind::Openai => format!("{}/models", self.base()),
            ProviderKind::OpencodeGo => format!("{}/v1/models", self.base()),
        }
    }

    /// How a request made with `key` says who asks; `session` is the
    /// conversation it belongs to, for a provider that takes one.
    pub(crate) fn access(&self, key: String, session: Option<&str>) -> Access {
        match self.kind {
            ProviderKind::Anthropic => Access {
                bearer: None,
                headers: vec![
                    ("x-api-key".to_owned(), key),
                    ("anthropic-version".to_owned(), ANTHROPIC_VERSION.to_owned()),
                ],
            },
            ProviderKind::Openai => Access {
                bearer: Some(key),
                headers: Vec::new(),
            },
            ProviderKind::OpencodeGo => {
                let mut headers = vec![("User-Agent".to_owned(), USER_AGENT.to_owned())];
                if let Some(session) = session {
                    headers.push((OPENCODE_SESSION.to_owned(), session.to_owned()));
                }
                Access {
                    bearer: Some(key),
                    headers,
                }
            }
        }
    }

    /// What Copilot takes as a session's provider, with `key`.
    pub(crate) fn copilot(&self, key: &str) -> Value {
        let Some(base) = self.openai_root() else {
            return json!({"type": "anthropic", "baseUrl": self.base(), "apiKey": key});
        };
        let wire_api = match self.wire_api.unwrap_or_default() {
            WireApi::Completions => "completions",
            WireApi::Responses => "responses",
        };
        json!({"type": "openai", "baseUrl": base, "apiKey": key, "wireApi": wire_api})
    }

    /// The models it has, asked with its key from `env`.
    pub(crate) async fn models(
        &self,
        http: ureq::Agent,
        env: &Env,
    ) -> Result<Vec<Model>, LocalModelError> {
        let access = self.access(self.key(env)?, None);
        let url = self.models_url();
        let variable = self.api_key_variable.clone();
        // ureq blocks, so it's asked on its own thread; one that panicked
        // lists nothing.
        tokio::task::spawn_blocking(move || {
            local_model::listing_with(&http, &url, access.bearer.as_deref(), &access.headers)
                .map(|listed| local_model::list_models(&listed))
                .map_err(|error| local_model::provider_fault(error, &variable))
        })
        .await
        .unwrap_or_else(|_| Ok(Vec::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(config: Value) -> Provider {
        serde_json::from_value(config).expect("a provider")
    }

    fn env(key: &str) -> Env {
        Env::new([("MY_KEY".to_owned(), key.to_owned())])
    }

    #[test]
    fn copilot_is_given_anthropics_api_with_the_key_from_the_variable() {
        let provider = provider(json!({"kind": "anthropic", "api_key_variable": "MY_KEY"}));
        let key = provider.key(&env("sk-ant")).expect("a key");
        let params = provider.copilot(&key);
        assert_eq!(params["type"], "anthropic");
        assert_eq!(params["baseUrl"], "https://api.anthropic.com");
        assert_eq!(params["apiKey"], "sk-ant");
    }

    #[test]
    fn copilot_is_given_an_openai_server_with_the_api_picked() {
        let completions = provider(json!({
            "kind": "openai",
            "base_url": "https://models.example/v1/",
            "api_key_variable": "MY_KEY",
        }));
        let params = completions.copilot("sk");
        assert_eq!(params["type"], "openai");
        assert_eq!(params["baseUrl"], "https://models.example/v1");
        assert_eq!(params["wireApi"], "completions");

        let responses = provider(json!({
            "kind": "openai",
            "api_key_variable": "MY_KEY",
            "wire_api": "responses",
        }));
        let params = responses.copilot("sk");
        assert_eq!(params["baseUrl"], "https://api.openai.com/v1");
        assert_eq!(params["wireApi"], "responses");
    }

    #[test]
    fn copilot_is_given_opencode_gos_openai_api() {
        let provider = provider(json!({"kind": "opencode_go", "api_key_variable": "MY_KEY"}));
        let params = provider.copilot("sk");
        assert_eq!(params["type"], "openai");
        assert_eq!(
            params["baseUrl"],
            "https://opencode.ai/inference/go/openai/v1"
        );
        assert_eq!(params["wireApi"], "completions");
        assert_eq!(params["apiKey"], "sk");
        assert_eq!(
            provider.models_url(),
            "https://opencode.ai/inference/go/v1/models"
        );
    }

    #[test]
    fn a_variable_that_isnt_set_or_is_empty_gives_no_key() {
        let provider = provider(json!({"kind": "openai", "api_key_variable": "MY_KEY"}));
        assert!(matches!(
            provider.key(&Env::default()),
            Err(ProviderError::NoVariable { variable }) if variable == "MY_KEY"
        ));
        assert!(matches!(
            provider.key(&env("")),
            Err(ProviderError::NoVariable { .. })
        ));
    }

    #[test]
    fn a_config_holding_a_key_itself_is_refused() {
        let config = json!({"kind": "openai", "api_key_variable": "MY_KEY", "api_key": "sk"});
        assert!(serde_json::from_value::<Provider>(config).is_err());
    }

    #[test]
    fn anthropics_api_has_no_openai_api_and_the_others_arent_anthropics() {
        let anthropic = provider(json!({"kind": "anthropic", "api_key_variable": "MY_KEY"}));
        assert!(matches!(
            anthropic.openai_base("A local model"),
            Err(ProviderError::Unsupported { .. })
        ));
        assert_eq!(
            anthropic.anthropic_base("Claude Code").expect("a base"),
            None
        );
        let go = provider(json!({"kind": "opencode_go", "api_key_variable": "MY_KEY"}));
        assert!(matches!(
            go.anthropic_base("Claude Code"),
            Err(ProviderError::Unsupported { .. })
        ));
    }

    #[test]
    fn a_refused_key_no_credit_and_a_limit_are_told_apart() {
        let refusal = |status| ProviderError::refusal("http://x", "MY_KEY", status, "no");
        assert!(matches!(
            refusal(401),
            Some(ProviderError::KeyRefused { status: 401, .. })
        ));
        assert!(matches!(refusal(402), Some(ProviderError::NoCredit { .. })));
        assert!(matches!(refusal(429), Some(ProviderError::Limited { .. })));
        assert!(refusal(500).is_none());
    }

    #[tokio::test]
    async fn listing_models_without_the_variable_says_which_is_missing() {
        let provider = provider(json!({"kind": "opencode_go", "api_key_variable": "MY_KEY"}));
        let http: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .into();
        let listed = provider.models(http, &Env::default()).await;
        assert!(matches!(
            listed,
            Err(LocalModelError::Provider(ProviderError::NoVariable { .. }))
        ));
    }
}
