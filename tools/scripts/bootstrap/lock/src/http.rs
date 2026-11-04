//! The transport: sends the request it is given and reports what came back,
//! whatever the status. `download` decides what a status means.

use std::io::Read;
use std::time::Duration;

use crate::Error;

const USER_AGENT: &str = "monty-oxide-bootstrap";

pub(crate) struct Http {
    agent: ureq::Agent,
}

impl Default for Http {
    fn default() -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(30)))
            .user_agent(USER_AGENT)
            // Every status comes back as a response for the caller to judge.
            .http_status_as_error(false)
            .build()
            .into();
        Http { agent }
    }
}

impl Http {
    /// GETs `url` with `headers`. Fails only when no response arrives.
    pub(crate) fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<Response, Error> {
        let mut request = self.agent.get(url);
        for &(name, value) in headers {
            request = request.header(name, value);
        }
        request
            .call()
            .map(Response)
            .map_err(|source| Error::Download {
                url: url.to_owned(),
                source,
            })
    }
}

/// A response with any status.
pub(crate) struct Response(ureq::http::Response<ureq::Body>);

impl Response {
    pub(crate) fn status(&self) -> u16 {
        self.0.status().as_u16()
    }

    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.0
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
    }

    pub(crate) fn into_reader(self) -> impl Read + use<> {
        self.0
            .into_body()
            .into_with_config()
            .limit(u64::MAX)
            .reader()
    }
}
