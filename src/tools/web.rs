use crate::tools::base::Tool;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use regex::Regex;
use reqwest::header::{ACCEPT, USER_AGENT};
use serde_json::{Map, Value, json};
use url::Url;

const DEFAULT_USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_7_2) AppleWebKit/537.36";

fn strip_tags(text: &str) -> String {
    let script_re = Regex::new(r"(?is)<script[\s\S]*?</script>")
        .unwrap_or_else(|_| Regex::new("^$").expect("regex"));
    let style_re = Regex::new(r"(?is)<style[\s\S]*?</style>")
        .unwrap_or_else(|_| Regex::new("^$").expect("regex"));
    let tag_re = Regex::new(r"(?is)<[^>]+>").unwrap_or_else(|_| Regex::new("^$").expect("regex"));
    let no_script = script_re.replace_all(text, "");
    let no_style = style_re.replace_all(&no_script, "");
    let stripped = tag_re.replace_all(&no_style, "");
    html_escape::decode_html_entities(&stripped).to_string()
}

fn normalize_text(text: &str) -> String {
    let whitespace_re = Regex::new(r"[ \t]+").unwrap_or_else(|_| Regex::new("^$").expect("regex"));
    let breaks_re = Regex::new(r"\n{3,}").unwrap_or_else(|_| Regex::new("^$").expect("regex"));
    let collapsed = whitespace_re.replace_all(text, " ");
    breaks_re.replace_all(&collapsed, "\n\n").trim().to_string()
}

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

pub struct WebSearchTool {
    api_key: String,
    max_results: usize,
}

impl WebSearchTool {
    pub fn new(api_key: Option<String>, max_results: usize) -> Self {
        Self {
            api_key: api_key
                .or_else(|| std::env::var("BRAVE_API_KEY").ok())
                .unwrap_or_default(),
            max_results,
        }
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web. Returns titles, URLs, and snippets."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query" },
                "count": { "type": "integer", "description": "Results (1-10)", "minimum": 1, "maximum": 10 }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, params: &Map<String, Value>) -> Result<String> {
        if self.api_key.is_empty() {
            return Ok("Error: BRAVE_API_KEY not configured".to_string());
        }
        let query = params
            .get("query")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("missing required string field: query"))?;
        let count = params
            .get("count")
            .and_then(Value::as_u64)
            .unwrap_or(self.max_results as u64);
        let n = count.clamp(1, 10);

        let client = reqwest::Client::new();
        let response = client
            .get("https://api.search.brave.com/res/v1/web/search")
            .query(&[("q", query), ("count", &n.to_string())])
            .header(ACCEPT, "application/json")
            .header("X-Subscription-Token", &self.api_key)
            .send()
            .await?;
        let response = response.error_for_status()?;
        let payload: Value = response.json().await?;
        let results = payload
            .get("web")
            .and_then(|v| v.get("results"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        if results.is_empty() {
            return Ok(format!("No results for: {query}"));
        }

        let mut lines = vec![format!("Results for: {query}\n")];
        for (idx, item) in results.iter().take(n as usize).enumerate() {
            let title = item.get("title").and_then(Value::as_str).unwrap_or("");
            let url = item.get("url").and_then(Value::as_str).unwrap_or("");
            lines.push(format!("{}. {title}\n   {url}", idx + 1));
            if let Some(desc) = item.get("description").and_then(Value::as_str) {
                lines.push(format!("   {desc}"));
            }
        }
        Ok(lines.join("\n"))
    }
}

pub struct WebFetchTool {
    max_chars: usize,
}

impl WebFetchTool {
    pub fn new(max_chars: usize) -> Self {
        Self { max_chars }
    }

    fn html_to_markdown(&self, html: &str) -> String {
        let mut text = html.to_string();
        let link_re = Regex::new(r#"(?is)<a\s+[^>]*href=["']([^"']+)["'][^>]*>([\s\S]*?)</a>"#)
            .unwrap_or_else(|_| Regex::new("^$").expect("regex"));
        text = link_re
            .replace_all(&text, |caps: &regex::Captures<'_>| {
                format!("[{}]({})", strip_tags(&caps[2]), &caps[1])
            })
            .to_string();
        normalize_text(&strip_tags(&text))
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch URL and extract readable content (HTML -> markdown/text)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "URL to fetch" },
                "extractMode": { "type": "string", "enum": ["markdown", "text"], "default": "markdown" },
                "maxChars": { "type": "integer", "minimum": 100 }
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

        let extract_mode = params
            .get("extractMode")
            .and_then(Value::as_str)
            .unwrap_or("markdown");
        let max_chars = params
            .get("maxChars")
            .and_then(Value::as_u64)
            .map(|v| v as usize)
            .unwrap_or(self.max_chars);

        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(5))
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        let response = client
            .get(url)
            .header(USER_AGENT, DEFAULT_USER_AGENT)
            .send()
            .await?;
        let final_url = response.url().to_string();
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("")
            .to_string();
        let body = response.text().await?;

        let (mut text, extractor) = if content_type.contains("application/json") {
            (
                serde_json::from_str::<Value>(&body)
                    .map(|v| serde_json::to_string_pretty(&v).unwrap_or_else(|_| body.clone()))
                    .unwrap_or(body.clone()),
                "json",
            )
        } else if content_type.contains("text/html")
            || body[..body.len().min(256)].to_lowercase().contains("<html")
            || body[..body.len().min(256)]
                .to_lowercase()
                .contains("<!doctype")
        {
            let extracted = if extract_mode == "text" {
                normalize_text(&strip_tags(&body))
            } else {
                self.html_to_markdown(&body)
            };
            (extracted, "html")
        } else {
            (body, "raw")
        };

        let truncated = text.len() > max_chars;
        if truncated {
            text.truncate(max_chars);
        }

        Ok(json!({
            "url": url,
            "finalUrl": final_url,
            "status": status,
            "extractor": extractor,
            "truncated": truncated,
            "length": text.len(),
            "text": text
        })
        .to_string())
    }
}
