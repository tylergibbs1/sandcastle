use std::collections::HashMap;

use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;

use crate::capability::{Capability, MethodSchema};
use crate::error::CapabilityError;

/// An HTTP client capability that makes real HTTP requests using `reqwest`.
///
/// Optionally restricts requests to a set of allowed domains and caps
/// response body size.
pub struct HttpCapability {
    client: Client,
    allowed_domains: Vec<String>,
    max_response_bytes: usize,
}

impl HttpCapability {
    /// Create a new HTTP capability.
    ///
    /// - `allowed_domains`: if non-empty, only URLs whose host contains one of
    ///   these strings will be allowed.
    /// - `max_response_bytes`: maximum number of bytes to read from the response
    ///   body (default 10 MB).
    pub fn new(allowed_domains: Vec<String>, max_response_bytes: usize) -> Self {
        Self {
            client: Client::new(),
            allowed_domains,
            max_response_bytes,
        }
    }

    /// Check whether the URL's host is in the allowed domains list.
    fn check_domain(&self, url: &str) -> Result<(), CapabilityError> {
        if self.allowed_domains.is_empty() {
            return Ok(());
        }

        let parsed = url::Url::parse(url).map_err(|e| CapabilityError::InvocationFailed {
            capability: "http".into(),
            method: "request".into(),
            message: format!("invalid URL: {e}"),
        })?;

        let host = parsed.host_str().unwrap_or("");
        let allowed = self.allowed_domains.iter().any(|d| host.contains(d.as_str()));

        if !allowed {
            return Err(CapabilityError::InvocationFailed {
                capability: "http".into(),
                method: "request".into(),
                message: format!("domain not in allow list: {host}"),
            });
        }

        Ok(())
    }
}

impl Default for HttpCapability {
    fn default() -> Self {
        Self::new(Vec::new(), 10 * 1024 * 1024) // 10 MB
    }
}

#[async_trait]
impl Capability for HttpCapability {
    fn name(&self) -> &str {
        "http"
    }

    fn methods(&self) -> Vec<MethodSchema> {
        vec![MethodSchema::new(
            "request",
            "Make an HTTP request",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "method": {"type": "string"},
                    "url": {"type": "string"},
                    "headers": {"type": "object"},
                    "body": {"type": "string"}
                },
                "required": ["method", "url"]
            }),
            serde_json::json!({
                "type": "object",
                "properties": {
                    "status": {"type": "number"},
                    "headers": {"type": "object"},
                    "body": {"type": "string"}
                }
            }),
        )]
    }

    async fn call(
        &self,
        method: &str,
        input: Value,
    ) -> Result<Value, CapabilityError> {
        if method != "request" {
            return Err(CapabilityError::NotFound {
                capability: "http".into(),
                method: method.into(),
            });
        }

        let http_method = input
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("GET")
            .to_uppercase();

        let url = input
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CapabilityError::InvocationFailed {
                capability: "http".into(),
                method: "request".into(),
                message: "missing required parameter: url".into(),
            })?;

        // Domain check
        self.check_domain(url)?;

        let reqwest_method = http_method
            .parse::<reqwest::Method>()
            .map_err(|e| CapabilityError::InvocationFailed {
                capability: "http".into(),
                method: "request".into(),
                message: format!("invalid HTTP method: {e}"),
            })?;

        let mut request = self.client.request(reqwest_method, url);

        // Apply headers
        if let Some(headers) = input.get("headers").and_then(|v| v.as_object()) {
            for (key, value) in headers {
                if let Some(val_str) = value.as_str() {
                    request = request.header(key.as_str(), val_str);
                }
            }
        }

        // Apply body
        if let Some(body) = input.get("body").and_then(|v| v.as_str()) {
            request = request.body(body.to_owned());
        }

        let response = request.send().await.map_err(|e| {
            CapabilityError::InvocationFailed {
                capability: "http".into(),
                method: "request".into(),
                message: format!("request failed: {e}"),
            }
        })?;

        let status = response.status().as_u16();

        // Collect response headers
        let response_headers: HashMap<String, String> = response
            .headers()
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_owned(),
                    v.to_str().unwrap_or("").to_owned(),
                )
            })
            .collect();

        // Read body, capped at max_response_bytes
        let body_bytes = response.bytes().await.map_err(|e| {
            CapabilityError::InvocationFailed {
                capability: "http".into(),
                method: "request".into(),
                message: format!("failed to read response body: {e}"),
            }
        })?;

        let body = if body_bytes.len() > self.max_response_bytes {
            String::from_utf8_lossy(&body_bytes[..self.max_response_bytes]).into_owned()
        } else {
            String::from_utf8_lossy(&body_bytes).into_owned()
        };

        Ok(serde_json::json!({
            "status": status,
            "headers": response_headers,
            "body": body,
        }))
    }
}
