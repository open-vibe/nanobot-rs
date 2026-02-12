use crate::tools::base::Tool;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use reqwest::Method;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Map, Value, json};
use std::str::FromStr;
use url::Url;

fn validate_url(url: &str) -> Result<()> {
    let parsed = Url::parse(url)?;
    match parsed.scheme() {
        "http" | "https" => {}
        scheme => return Err(anyhow!("Only http/https allowed, got '{scheme}'")),
    }
    if parsed.host_str().is_none() {
        return Err(anyhow!("Missing domain"));
    }
    Ok(())
}

fn parse_method(raw: Option<&str>) -> Result<Method> {
    let method = raw.unwrap_or("GET").trim().to_ascii_uppercase();
    let parsed = match method.as_str() {
        "GET" => Method::GET,
        "POST" => Method::POST,
        "PUT" => Method::PUT,
        "PATCH" => Method::PATCH,
        "DELETE" => Method::DELETE,
        "HEAD" => Method::HEAD,
        "OPTIONS" => Method::OPTIONS,
        _ => return Err(anyhow!("unsupported HTTP method: {method}")),
    };
    Ok(parsed)
}

fn value_to_query_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        _ => serde_json::to_string(value).unwrap_or_default(),
    }
}

fn parse_headers(raw: Option<&Map<String, Value>>) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    let Some(raw) = raw else {
        return Ok(headers);
    };

    for (key, value) in raw {
        let name =
            HeaderName::from_str(key.trim()).map_err(|_| anyhow!("invalid header name: {key}"))?;
        let value_str = match value {
            Value::String(s) => s.trim().to_string(),
            Value::Null => String::new(),
            _ => value.to_string(),
        };
        let header_value = HeaderValue::from_str(&value_str)
            .map_err(|_| anyhow!("invalid header value for {key}"))?;
        headers.insert(name, header_value);
    }

    Ok(headers)
}

pub struct HttpRequestTool {
    default_timeout_s: u64,
    default_max_chars: usize,
}

impl HttpRequestTool {
    pub fn new(default_timeout_s: u64, default_max_chars: usize) -> Self {
        Self {
            default_timeout_s: default_timeout_s.clamp(1, 300),
            default_max_chars: default_max_chars.clamp(100, 500_000),
        }
    }
}

#[async_trait]
impl Tool for HttpRequestTool {
    fn name(&self) -> &str {
        "http_request"
    }

    fn description(&self) -> &str {
        "Send HTTP requests (GET/POST/PUT/PATCH/DELETE/etc.) to APIs, including localhost and LAN services."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "HTTP/HTTPS URL" },
                "method": {
                    "type": "string",
                    "description": "HTTP method",
                    "enum": ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"],
                    "default": "GET"
                },
                "headers": {
                    "type": "object",
                    "description": "Request headers (key-value pairs)"
                },
                "query": {
                    "type": "object",
                    "description": "Query parameters (key-value pairs)"
                },
                "json": {
                    "type": "object",
                    "description": "JSON body object (use with POST/PUT/PATCH)"
                },
                "body": {
                    "type": "string",
                    "description": "Raw text body"
                },
                "timeoutSeconds": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 300,
                    "description": "Request timeout in seconds"
                },
                "maxChars": {
                    "type": "integer",
                    "minimum": 100,
                    "maximum": 500000,
                    "description": "Maximum response body characters"
                },
                "followRedirects": {
                    "type": "boolean",
                    "description": "Whether to follow redirects",
                    "default": true
                },
                "insecureTls": {
                    "type": "boolean",
                    "description": "Allow invalid TLS certificates",
                    "default": false
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, params: &Map<String, Value>) -> Result<String> {
        let url = params
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("missing required string field: url"))?;
        if let Err(err) = validate_url(url) {
            return Ok(
                json!({"error": format!("URL validation failed: {err}"), "url": url}).to_string(),
            );
        }

        if params.contains_key("json") && params.contains_key("body") {
            return Ok(
                json!({"error": "Specify either 'json' or 'body', not both", "url": url})
                    .to_string(),
            );
        }

        let method = parse_method(params.get("method").and_then(Value::as_str))?;
        let timeout_s = params
            .get("timeoutSeconds")
            .and_then(Value::as_u64)
            .unwrap_or(self.default_timeout_s)
            .clamp(1, 300);
        let max_chars = params
            .get("maxChars")
            .and_then(Value::as_u64)
            .map(|v| v as usize)
            .unwrap_or(self.default_max_chars)
            .clamp(100, 500_000);
        let follow_redirects = params
            .get("followRedirects")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let insecure_tls = params
            .get("insecureTls")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let headers = parse_headers(params.get("headers").and_then(Value::as_object))?;
        let query_pairs = params
            .get("query")
            .and_then(Value::as_object)
            .map(|obj| {
                obj.iter()
                    .map(|(k, v)| (k.clone(), value_to_query_string(v)))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_s))
            .danger_accept_invalid_certs(insecure_tls)
            .redirect(if follow_redirects {
                reqwest::redirect::Policy::limited(10)
            } else {
                reqwest::redirect::Policy::none()
            })
            .build()?;

        let mut request = client.request(method.clone(), url);
        if !headers.is_empty() {
            request = request.headers(headers);
        }
        if !query_pairs.is_empty() {
            request = request.query(&query_pairs);
        }

        if let Some(json_body) = params.get("json").and_then(Value::as_object) {
            request = request.json(json_body);
        } else if let Some(raw_body) = params.get("body").and_then(Value::as_str) {
            request = request.body(raw_body.to_string());
        }

        let response = request.send().await?;
        let final_url = response.url().to_string();
        let status = response.status();
        let status_code = status.as_u16();
        let ok = status.is_success();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let mut response_headers = serde_json::Map::new();
        for (name, value) in response.headers() {
            response_headers.insert(
                name.to_string(),
                Value::String(value.to_str().unwrap_or("").to_string()),
            );
        }

        let bytes = response.bytes().await?;
        let text = String::from_utf8_lossy(&bytes).to_string();
        let mut body = text;
        let truncated = body.len() > max_chars;
        if truncated {
            body.truncate(max_chars);
        }

        Ok(json!({
            "method": method.as_str(),
            "url": url,
            "finalUrl": final_url,
            "status": status_code,
            "ok": ok,
            "contentType": content_type,
            "headers": response_headers,
            "truncated": truncated,
            "length": body.len(),
            "body": body
        })
        .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_method, validate_url, value_to_query_string};
    use serde_json::json;

    #[test]
    fn parse_method_defaults_to_get() {
        let method = parse_method(None).expect("method");
        assert_eq!(method.as_str(), "GET");
    }

    #[test]
    fn parse_method_rejects_invalid() {
        let err = parse_method(Some("NOT_A_METHOD")).expect_err("should fail");
        assert!(err.to_string().contains("unsupported HTTP method"));
    }

    #[test]
    fn validate_url_allows_localhost_http() {
        validate_url("http://127.0.0.1:8080/health").expect("localhost should be valid");
    }

    #[test]
    fn validate_url_rejects_non_http() {
        let err = validate_url("file:///etc/passwd").expect_err("should fail");
        assert!(err.to_string().contains("Only http/https allowed"));
    }

    #[test]
    fn value_to_query_string_formats_primitives() {
        assert_eq!(value_to_query_string(&json!(null)), "");
        assert_eq!(value_to_query_string(&json!(true)), "true");
        assert_eq!(value_to_query_string(&json!(123)), "123");
        assert_eq!(value_to_query_string(&json!("abc")), "abc");
    }
}
