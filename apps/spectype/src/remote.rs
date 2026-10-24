use spectype_core::http::{HttpHeader, request_headers};
use spectype_core::{Host, HostError, LoadedDocument, load_file};

/// Fetches an `http(s)` URL with the given request headers, like Redocly's resolver.
pub type Fetch<'a> = Box<dyn FnMut(&str, &[(String, String)]) -> Result<LoadedDocument, HostError> + 'a>;

/// Loads files from disk and `http(s)` URLs through its [`Fetch`].
pub struct CliHost<'a> {
    fetch: Fetch<'a>,
    pub headers: Vec<HttpHeader>,
    /// The process exits once the CLI returns, so the last document need not be freed.
    pub exiting: bool,
}

impl<'a> CliHost<'a> {
    pub fn new(fetch: Fetch<'a>, exiting: bool) -> Self {
        Self { fetch, headers: Vec::new(), exiting }
    }
}

impl Host for CliHost<'_> {
    fn load(&mut self, absolute_ref: &str) -> Result<Option<LoadedDocument>, HostError> {
        if is_url(absolute_ref) {
            (self.fetch)(absolute_ref, &request_headers(absolute_ref, &self.headers)).map(Some)
        } else {
            load_file(absolute_ref)
        }
    }
}

/// Fetches over the network with `ureq`.
#[cfg(not(target_family = "wasm"))]
pub fn ureq_fetch() -> Fetch<'static> {
    use std::time::Duration;

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(60)))
        .http_status_as_error(false)
        .user_agent(concat!("spectype/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    Box::new(move |url, headers| {
        let failed = |e: ureq::Error| HostError::with_source(format!("Failed to load {url}: {e}"), e);
        let mut request = agent.get(url);
        for (name, value) in headers {
            request = request.header(name, value);
        }
        let mut res = request.call().map_err(failed)?;
        let status = res.status();
        if !status.is_success() {
            let reason = status.canonical_reason().unwrap_or_default();
            return Err(HostError::new(format!("Failed to load {url}: {} {reason}", status.as_u16())));
        }
        let mime_type = res.headers().get("content-type").and_then(|v| v.to_str().ok()).map(str::to_owned);
        let bytes = res.body_mut().with_config().limit(u64::MAX).read_to_vec().map_err(failed)?;
        // `Response.text()` decodes as UTF-8 and drops a byte order mark.
        let text = String::from_utf8_lossy(&bytes);
        Ok(LoadedDocument { text: text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned(), mime_type })
    })
}

pub fn is_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}
