//! Shell integration (T050): opening external URLs through the core.
//!
//! The WebView never opens anything itself and holds no shell capability.
//! `shell.openExternal` validates the URL here (http and https only, bounded
//! length, no credentials, no whitespace or control characters) and hands the
//! normalized form to an [`ExternalOpener`] the composition root provides: the
//! system's default handler in the desktop app, a recording fake in tests.
//! Nothing else about the request is trusted, and the full URL never reaches
//! the logs (query strings carry tokens): only its scheme and host do.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::runtime::Handle;
use ts_rs::TS;
use url::Url;

use crate::protocol::actor::Actor;
use crate::protocol::error::CoreError;
use crate::protocol::ids::CorrelationId;

/// Longest URL accepted, in bytes.
pub const MAX_URL_LENGTH: usize = 2048;

/// `shell.openExternal`: open `url` with the system's default handler.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OpenExternalRequest {
    pub url: String,
}

/// Opens an already validated, normalized URL outside the application.
/// Spawning the handler is platform work that the composition root owns.
pub trait ExternalOpener: Send + Sync {
    fn open(&self, url: &str) -> Result<(), CoreError>;
}

/// The `shell.*` domain: validation in front of the injected opener.
pub struct ShellService {
    handle: Handle,
    opener: Arc<dyn ExternalOpener>,
}

impl ShellService {
    pub fn new(handle: Handle, opener: Arc<dyn ExternalOpener>) -> Arc<Self> {
        Arc::new(Self { handle, opener })
    }

    /// Validate, then open on the blocking pool (the opener spawns a process).
    /// A rejected URL has no side effect.
    pub async fn open_external(
        &self,
        request: OpenExternalRequest,
        actor: Actor,
        correlation: CorrelationId,
    ) -> Result<(), CoreError> {
        let url = validate_external_url(&request.url)?;
        tracing::info!(
            scheme = url.scheme(),
            host = url.host_str().unwrap_or_default(),
            ?actor,
            correlation_id = %correlation,
            "opening external url"
        );
        let opener = Arc::clone(&self.opener);
        let target = url.to_string();
        self.handle
            .spawn_blocking(move || opener.open(&target))
            .await
            .map_err(|error| CoreError::internal(format!("external opener task failed: {error}")))?
    }
}

/// Accept `http` and `https` URLs only. Length, whitespace and control
/// characters are checked *before* parsing: the WHATWG parser silently strips
/// tabs and newlines, so `https://exa\nmple.com` would otherwise pass.
pub fn validate_external_url(raw: &str) -> Result<Url, CoreError> {
    if raw.is_empty() {
        return Err(reject("empty"));
    }
    if raw.len() > MAX_URL_LENGTH {
        return Err(reject(format!("longer than {MAX_URL_LENGTH} bytes")));
    }
    if raw.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(reject("contains whitespace or control characters"));
    }
    let url = Url::parse(raw).map_err(|error| reject(format!("not a valid URL: {error}")))?;
    match url.scheme() {
        "http" | "https" => {}
        other => {
            return Err(reject(format!(
                "scheme '{other}' is not allowed (http and https only)"
            )));
        }
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(reject("no host"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(reject("credentials in the URL are not allowed"));
    }
    Ok(url)
}

fn reject(reason: impl Into<String>) -> CoreError {
    CoreError::invalid_input("url", reason)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::protocol::error::CoreErrorCode;

    #[derive(Default)]
    struct RecordingOpener {
        opened: Mutex<Vec<String>>,
        fail: bool,
    }

    impl ExternalOpener for RecordingOpener {
        fn open(&self, url: &str) -> Result<(), CoreError> {
            if self.fail {
                return Err(std::io::Error::other("no handler").into());
            }
            self.opened.lock().unwrap().push(url.to_owned());
            Ok(())
        }
    }

    fn reason(raw: &str) -> String {
        let error = validate_external_url(raw).unwrap_err();
        assert_eq!(error.code, CoreErrorCode::InvalidInput);
        let details = error.details.expect("details");
        assert_eq!(details["field"], "url");
        details["reason"].as_str().expect("reason").to_owned()
    }

    #[test]
    fn accepts_http_and_https_and_normalizes() {
        assert_eq!(
            validate_external_url("https://example.com/docs?x=1#top")
                .unwrap()
                .as_str(),
            "https://example.com/docs?x=1#top"
        );
        assert_eq!(
            validate_external_url("HTTP://EXAMPLE.com")
                .unwrap()
                .as_str(),
            "http://example.com/"
        );
        assert_eq!(
            validate_external_url("https://bücher.example/ä")
                .unwrap()
                .host_str(),
            Some("xn--bcher-kva.example")
        );
    }

    #[test]
    fn rejects_everything_else_before_any_side_effect() {
        assert_eq!(reason(""), "empty");
        assert!(reason(&format!("https://example.com/{}", "a".repeat(2048))).contains("longer"));
        assert!(reason("https://exa\nmple.com").contains("whitespace"));
        assert!(reason("https://example.com/a b").contains("whitespace"));
        assert!(reason("https://example.com/\u{7}").contains("control"));
        assert!(reason("javascript:alert(1)").contains("'javascript'"));
        assert!(reason("file:///etc/passwd").contains("'file'"));
        assert!(reason("ftp://example.com").contains("'ftp'"));
        assert!(reason("mailto:someone@example.com").contains("'mailto'"));
        assert!(reason("data:text/html,hi").contains("'data'"));
        assert!(reason("example.com").contains("not a valid URL"));
        assert!(reason("https://user:pw@example.com").contains("credentials"));
        assert!(reason("https://user@example.com").contains("credentials"));
        assert!(reason("http://").contains("not a valid URL"));
    }

    #[tokio::test]
    async fn opens_validated_urls_through_the_opener() {
        let opener = Arc::new(RecordingOpener::default());
        let shell = ShellService::new(Handle::current(), opener.clone());
        shell
            .open_external(
                OpenExternalRequest {
                    url: "HTTPS://Example.com/Path".into(),
                },
                Actor::User,
                CorrelationId::new(),
            )
            .await
            .unwrap();
        assert_eq!(
            *opener.opened.lock().unwrap(),
            vec!["https://example.com/Path".to_owned()]
        );
    }

    #[tokio::test]
    async fn a_rejected_url_never_reaches_the_opener() {
        let opener = Arc::new(RecordingOpener::default());
        let shell = ShellService::new(Handle::current(), opener.clone());
        let error = shell
            .open_external(
                OpenExternalRequest {
                    url: "javascript:alert(1)".into(),
                },
                Actor::User,
                CorrelationId::new(),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, CoreErrorCode::InvalidInput);
        assert!(opener.opened.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_opener_failure_is_an_io_error() {
        let opener = Arc::new(RecordingOpener {
            fail: true,
            ..Default::default()
        });
        let shell = ShellService::new(Handle::current(), opener);
        let error = shell
            .open_external(
                OpenExternalRequest {
                    url: "https://example.com".into(),
                },
                Actor::User,
                CorrelationId::new(),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, CoreErrorCode::Io);
        assert!(error.retryable);
    }
}
