use crate::config::WebSearchConfig;
use crate::tools::base::Tool;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use regex::Regex;
use reqwest::header::{ACCEPT, USER_AGENT};
use serde_json::{Map, Value, json};
use url::Url;

const DEFAULT_USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_7_2) AppleWebKit/537.36";
const BRAVE_SEARCH_ENDPOINT: &str = "https://api.search.brave.com/res/v1/web/search";
const DUCKDUCKGO_INSTANT_ENDPOINT: &str = "https://api.duckduckgo.com/";
const PERPLEXITY_DIRECT_BASE_URL: &str = "https://api.perplexity.ai";
const PERPLEXITY_OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
const DEFAULT_PERPLEXITY_MODEL: &str = "perplexity/sonar-pro";
const GROK_RESPONSES_ENDPOINT: &str = "https://api.x.ai/v1/responses";
const DEFAULT_GROK_MODEL: &str = "grok-4-1-fast";

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

#[derive(Debug, Clone, Copy)]
enum WebSearchProvider {
    Brave,
    Perplexity,
    Grok,
}

pub struct WebSearchTool {
    provider: WebSearchProvider,
    brave_api_key: String,
    perplexity_api_key: String,
    perplexity_base_url: String,
    perplexity_model: String,
    grok_api_key: String,
    grok_model: String,
    grok_inline_citations: bool,
    max_results: usize,
}

fn push_duckduckgo_result(
    output: &mut Vec<(String, String, String)>,
    title: &str,
    url: &str,
    snippet: &str,
) {
    let title = title.trim();
    let url = url.trim();
    let snippet = snippet.trim();
    if title.is_empty() || url.is_empty() {
        return;
    }
    if output
        .iter()
        .any(|(_, existing_url, _)| existing_url == url)
    {
        return;
    }
    output.push((title.to_string(), url.to_string(), snippet.to_string()));
}

fn collect_duckduckgo_related_topics(topics: &[Value], output: &mut Vec<(String, String, String)>) {
    for topic in topics {
        if let Some(nested) = topic.get("Topics").and_then(Value::as_array) {
            collect_duckduckgo_related_topics(nested, output);
            continue;
        }

        let text = topic
            .get("Text")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        let url = topic
            .get("FirstURL")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();

        if text.is_empty() || url.is_empty() {
            continue;
        }

        let title = text.split(" - ").next().unwrap_or(text);
        push_duckduckgo_result(output, title, url, text);
    }
}

impl WebSearchTool {
    fn normalize_secret(secret: impl AsRef<str>) -> String {
        secret.as_ref().trim().to_string()
    }

    fn resolve_provider(raw: &str) -> WebSearchProvider {
        match raw.trim().to_ascii_lowercase().as_str() {
            "perplexity" => WebSearchProvider::Perplexity,
            "grok" => WebSearchProvider::Grok,
            _ => WebSearchProvider::Brave,
        }
    }

    fn resolve_perplexity_api_key(config: &WebSearchConfig) -> String {
        let from_config = Self::normalize_secret(&config.perplexity.api_key);
        if !from_config.is_empty() {
            return from_config;
        }
        let from_env = std::env::var("PERPLEXITY_API_KEY").unwrap_or_default();
        let from_env = Self::normalize_secret(from_env);
        if !from_env.is_empty() {
            return from_env;
        }
        Self::normalize_secret(std::env::var("OPENROUTER_API_KEY").unwrap_or_default())
    }

    fn resolve_perplexity_base_url(config: &WebSearchConfig, api_key: &str) -> String {
        if let Some(base_url) = &config.perplexity.base_url {
            let base_url = Self::normalize_secret(base_url);
            if !base_url.is_empty() {
                return base_url;
            }
        }

        let key = api_key.to_ascii_lowercase();
        if key.starts_with("pplx-") {
            PERPLEXITY_DIRECT_BASE_URL.to_string()
        } else {
            PERPLEXITY_OPENROUTER_BASE_URL.to_string()
        }
    }

    fn resolve_perplexity_model(config: &WebSearchConfig) -> String {
        config
            .perplexity
            .model
            .as_deref()
            .map(Self::normalize_secret)
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| DEFAULT_PERPLEXITY_MODEL.to_string())
    }

    fn resolve_perplexity_request_model(base_url: &str, model: &str) -> String {
        let parsed = Url::parse(base_url).ok();
        let host = parsed
            .and_then(|u| u.host_str().map(|s| s.to_ascii_lowercase()))
            .unwrap_or_default();
        if host == "api.perplexity.ai" {
            model
                .strip_prefix("perplexity/")
                .unwrap_or(model)
                .to_string()
        } else {
            model.to_string()
        }
    }

    fn resolve_grok_api_key(config: &WebSearchConfig) -> String {
        let from_config = Self::normalize_secret(&config.grok.api_key);
        if !from_config.is_empty() {
            return from_config;
        }
        Self::normalize_secret(std::env::var("XAI_API_KEY").unwrap_or_default())
    }

    fn resolve_grok_model(config: &WebSearchConfig) -> String {
        config
            .grok
            .model
            .as_deref()
            .map(Self::normalize_secret)
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| DEFAULT_GROK_MODEL.to_string())
    }

    pub fn from_config(config: WebSearchConfig) -> Self {
        let brave_api_key = Self::normalize_secret(&config.api_key);
        let brave_api_key = if brave_api_key.is_empty() {
            Self::normalize_secret(std::env::var("BRAVE_API_KEY").unwrap_or_default())
        } else {
            brave_api_key
        };
        let perplexity_api_key = Self::resolve_perplexity_api_key(&config);
        let perplexity_base_url = Self::resolve_perplexity_base_url(&config, &perplexity_api_key);
        let perplexity_model = Self::resolve_perplexity_model(&config);
        let grok_api_key = Self::resolve_grok_api_key(&config);
        let grok_model = Self::resolve_grok_model(&config);

        Self {
            provider: Self::resolve_provider(&config.provider),
            brave_api_key,
            perplexity_api_key,
            perplexity_base_url,
            perplexity_model,
            grok_api_key,
            grok_model,
            grok_inline_citations: config.grok.inline_citations,
            max_results: config.max_results.clamp(1, 10),
        }
    }

    pub fn new(api_key: Option<String>, max_results: usize) -> Self {
        let mut config = WebSearchConfig::default();
        config.api_key = api_key.unwrap_or_default();
        config.max_results = max_results;
        Self::from_config(config)
    }

    fn format_search_answer(
        query: &str,
        provider_name: &str,
        answer: &str,
        citations: &[String],
        inline_citations: bool,
    ) -> String {
        let mut lines = vec![format!("Results for: {query} ({provider_name})\n")];
        lines.push(answer.trim().to_string());
        if !citations.is_empty() {
            lines.push(String::new());
            if inline_citations {
                lines.push("Citations (inline mode enabled):".to_string());
            } else {
                lines.push("Sources:".to_string());
            }
            for (idx, url) in citations.iter().enumerate() {
                lines.push(format!("{}. {}", idx + 1, url));
            }
        }
        lines.join("\n")
    }

    fn format_results(
        query: &str,
        provider_name: &str,
        results: &[(String, String, String)],
        limit: usize,
    ) -> String {
        if results.is_empty() {
            return format!("No results for: {query} ({provider_name})");
        }

        let mut lines = vec![format!("Results for: {query} ({provider_name})\n")];
        for (idx, (title, url, desc)) in results.iter().take(limit).enumerate() {
            lines.push(format!("{}. {title}\n   {url}", idx + 1));
            if !desc.is_empty() {
                lines.push(format!("   {desc}"));
            }
        }
        lines.join("\n")
    }

    async fn search_brave(&self, query: &str, n: u64) -> Result<Vec<(String, String, String)>> {
        let client = reqwest::Client::new();
        let response = client
            .get(BRAVE_SEARCH_ENDPOINT)
            .query(&[("q", query), ("count", &n.to_string())])
            .header(ACCEPT, "application/json")
            .header("X-Subscription-Token", &self.brave_api_key)
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

        let mut out = Vec::new();
        for item in results {
            let title = item
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            let url = item
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            let desc = item
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            if !title.is_empty() && !url.is_empty() {
                out.push((title.to_string(), url.to_string(), desc.to_string()));
            }
        }
        Ok(out)
    }

    async fn search_duckduckgo(
        &self,
        query: &str,
        n: u64,
    ) -> Result<Vec<(String, String, String)>> {
        let client = reqwest::Client::new();
        let response = client
            .get(DUCKDUCKGO_INSTANT_ENDPOINT)
            .query(&[
                ("q", query),
                ("format", "json"),
                ("no_html", "1"),
                ("skip_disambig", "1"),
                ("no_redirect", "1"),
            ])
            .header(ACCEPT, "application/json")
            .header(USER_AGENT, DEFAULT_USER_AGENT)
            .send()
            .await?;
        let response = response.error_for_status()?;
        let payload: Value = response.json().await?;

        let mut out = Vec::new();
        let abstract_text = payload
            .get("AbstractText")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        let abstract_url = payload
            .get("AbstractURL")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        let heading = payload
            .get("Heading")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        if !abstract_text.is_empty() && !abstract_url.is_empty() {
            let title = if heading.is_empty() {
                abstract_text
                    .split('.')
                    .next()
                    .unwrap_or(abstract_text)
                    .trim()
            } else {
                heading
            };
            push_duckduckgo_result(&mut out, title, abstract_url, abstract_text);
        }

        if let Some(topics) = payload.get("RelatedTopics").and_then(Value::as_array) {
            collect_duckduckgo_related_topics(topics, &mut out);
        }

        out.truncate(n as usize);
        Ok(out)
    }

    async fn search_perplexity(&self, query: &str) -> Result<(String, Vec<String>)> {
        let client = reqwest::Client::new();
        let endpoint = format!(
            "{}/chat/completions",
            self.perplexity_base_url.trim_end_matches('/')
        );
        let model = Self::resolve_perplexity_request_model(
            &self.perplexity_base_url,
            &self.perplexity_model,
        );
        let response = client
            .post(endpoint)
            .header("Content-Type", "application/json")
            .header(
                "Authorization",
                format!("Bearer {}", self.perplexity_api_key),
            )
            .header("HTTP-Referer", "https://github.com/open-vibe/nanobot-rs")
            .header("X-Title", "nanobot-rs web_search")
            .json(&json!({
                "model": model,
                "messages": [{ "role": "user", "content": query }],
            }))
            .send()
            .await?;
        let response = response.error_for_status()?;
        let payload: Value = response.json().await?;
        let answer = payload
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|arr| arr.first())
            .and_then(|v| v.get("message"))
            .and_then(|v| v.get("content"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        let citations = payload
            .get("citations")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Ok((answer, citations))
    }

    fn extract_grok_output_text(payload: &Value) -> Option<String> {
        if let Some(output_text) = payload.get("output_text").and_then(Value::as_str) {
            let output_text = output_text.trim();
            if !output_text.is_empty() {
                return Some(output_text.to_string());
            }
        }
        let output = payload.get("output").and_then(Value::as_array)?;
        for item in output {
            if item.get("type").and_then(Value::as_str) != Some("message") {
                continue;
            }
            let Some(content) = item.get("content").and_then(Value::as_array) else {
                continue;
            };
            for block in content {
                if block.get("type").and_then(Value::as_str) != Some("output_text") {
                    continue;
                }
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    let text = text.trim();
                    if !text.is_empty() {
                        return Some(text.to_string());
                    }
                }
            }
        }
        None
    }

    async fn search_grok(&self, query: &str) -> Result<(String, Vec<String>)> {
        let client = reqwest::Client::new();
        let response = client
            .post(GROK_RESPONSES_ENDPOINT)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", self.grok_api_key))
            .json(&json!({
                "model": self.grok_model,
                "input": [{ "role": "user", "content": query }],
                "tools": [{ "type": "web_search" }],
            }))
            .send()
            .await?;
        let response = response.error_for_status()?;
        let payload: Value = response.json().await?;
        let answer = Self::extract_grok_output_text(&payload).unwrap_or_default();
        let citations = payload
            .get("citations")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Ok((answer, citations))
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
        let query = params
            .get("query")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("missing required string field: query"))?;
        let count = params
            .get("count")
            .and_then(Value::as_u64)
            .unwrap_or(self.max_results as u64);
        let n = count.clamp(1, 10);
        let note = match self.provider {
            WebSearchProvider::Brave => {
                if !self.brave_api_key.is_empty() {
                    match self.search_brave(query, n).await {
                        Ok(results) if !results.is_empty() => {
                            return Ok(Self::format_results(query, "Brave", &results, n as usize));
                        }
                        Ok(_) => Some(
                            "Brave returned no results, switched to DuckDuckGo fallback."
                                .to_string(),
                        ),
                        Err(err) => Some(format!(
                            "Brave search failed ({err}), switched to DuckDuckGo fallback."
                        )),
                    }
                } else {
                    Some(
                        "BRAVE_API_KEY not configured, using keyless DuckDuckGo fallback."
                            .to_string(),
                    )
                }
            }
            WebSearchProvider::Perplexity => {
                if self.perplexity_api_key.is_empty() {
                    Some(
                        "Perplexity API key not configured, using keyless DuckDuckGo fallback."
                            .to_string(),
                    )
                } else {
                    match self.search_perplexity(query).await {
                        Ok((answer, citations)) if !answer.trim().is_empty() => {
                            return Ok(Self::format_search_answer(
                                query,
                                "Perplexity",
                                &answer,
                                &citations,
                                false,
                            ));
                        }
                        Ok(_) => Some(
                            "Perplexity returned an empty answer, switched to DuckDuckGo fallback."
                                .to_string(),
                        ),
                        Err(err) => Some(format!(
                            "Perplexity search failed ({err}), switched to DuckDuckGo fallback."
                        )),
                    }
                }
            }
            WebSearchProvider::Grok => {
                if self.grok_api_key.is_empty() {
                    Some(
                        "XAI_API_KEY not configured, using keyless DuckDuckGo fallback."
                            .to_string(),
                    )
                } else {
                    match self.search_grok(query).await {
                        Ok((answer, citations)) if !answer.trim().is_empty() => {
                            return Ok(Self::format_search_answer(
                                query,
                                "Grok",
                                &answer,
                                &citations,
                                self.grok_inline_citations,
                            ));
                        }
                        Ok(_) => Some(
                            "Grok returned an empty answer, switched to DuckDuckGo fallback."
                                .to_string(),
                        ),
                        Err(err) => Some(format!(
                            "Grok search failed ({err}), switched to DuckDuckGo fallback."
                        )),
                    }
                }
            }
        };

        match self.search_duckduckgo(query, n).await {
            Ok(results) => {
                let content =
                    Self::format_results(query, "DuckDuckGo fallback", &results, n as usize);
                if let Some(note) = note {
                    Ok(format!("{note}\n\n{content}"))
                } else {
                    Ok(content)
                }
            }
            Err(err) => {
                if let Some(note) = note {
                    Ok(format!(
                        "{note}\n\nSearch fallback failed: {err}\nTip: use web_fetch with a concrete URL for direct page access."
                    ))
                } else {
                    Err(err)
                }
            }
        }
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

#[cfg(test)]
mod tests {
    use super::{WebSearchProvider, WebSearchTool, collect_duckduckgo_related_topics};
    use serde_json::json;

    #[test]
    fn collect_duckduckgo_related_topics_handles_nested_topics() {
        let payload = json!([
            {
                "Text": "Rust - Programming language",
                "FirstURL": "https://duckduckgo.com/Rust_(programming_language)"
            },
            {
                "Name": "Nested",
                "Topics": [
                    {
                        "Text": "Tokio - Async runtime",
                        "FirstURL": "https://duckduckgo.com/Tokio"
                    }
                ]
            }
        ]);

        let mut out = Vec::new();
        collect_duckduckgo_related_topics(payload.as_array().expect("array"), &mut out);

        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0, "Rust");
        assert!(out[0].1.contains("Rust_"));
        assert_eq!(out[1].0, "Tokio");
    }

    #[test]
    fn resolve_provider_defaults_to_brave() {
        assert!(matches!(
            WebSearchTool::resolve_provider(""),
            WebSearchProvider::Brave
        ));
        assert!(matches!(
            WebSearchTool::resolve_provider("Perplexity"),
            WebSearchProvider::Perplexity
        ));
        assert!(matches!(
            WebSearchTool::resolve_provider("grok"),
            WebSearchProvider::Grok
        ));
    }

    #[test]
    fn resolve_perplexity_request_model_strips_prefix_for_direct_base() {
        let direct = WebSearchTool::resolve_perplexity_request_model(
            "https://api.perplexity.ai",
            "perplexity/sonar-pro",
        );
        assert_eq!(direct, "sonar-pro");

        let openrouter = WebSearchTool::resolve_perplexity_request_model(
            "https://openrouter.ai/api/v1",
            "perplexity/sonar-pro",
        );
        assert_eq!(openrouter, "perplexity/sonar-pro");
    }

    #[test]
    fn extract_grok_output_text_supports_output_array_shape() {
        let payload = json!({
            "output": [{
                "type": "message",
                "content": [{
                    "type": "output_text",
                    "text": "hello from grok"
                }]
            }]
        });
        let text = WebSearchTool::extract_grok_output_text(&payload).expect("text");
        assert_eq!(text, "hello from grok");
    }
}
