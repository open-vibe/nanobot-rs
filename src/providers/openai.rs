use crate::providers::base::{LLMProvider, LLMResponse, ToolCallRequest};
use anyhow::Context;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{Map, Value, json};

#[derive(Clone)]
pub struct OpenAIProvider {
    api_key: String,
    api_base: String,
    default_model: String,
    client: Client,
}

impl OpenAIProvider {
    pub fn new(
        api_key: impl Into<String>,
        api_base: Option<String>,
        default_model: impl Into<String>,
    ) -> Self {
        Self {
            api_key: api_key.into(),
            api_base: api_base.unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
            default_model: default_model.into(),
            client: Client::new(),
        }
    }
}

#[async_trait]
impl LLMProvider for OpenAIProvider {
    async fn chat(
        &self,
        messages: &[Value],
        tools: Option<&[Value]>,
        model: Option<&str>,
        max_tokens: u32,
        temperature: f32,
    ) -> anyhow::Result<LLMResponse> {
        let model_name = model.unwrap_or(&self.default_model).to_string();
        let mut body = json!({
            "model": model_name,
            "messages": messages,
            "max_tokens": max_tokens,
            "temperature": temperature,
        });

        if let Some(tool_defs) = tools {
            body["tools"] = Value::Array(tool_defs.to_vec());
            body["tool_choice"] = Value::String("auto".to_string());
        }

        let url = format!("{}/chat/completions", self.api_base.trim_end_matches('/'));
        let response = self
            .client
            .post(url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .context("failed to call OpenAI-compatible endpoint")?;

        let status = response.status();
        let payload: Value = response
            .json()
            .await
            .context("failed to parse provider response as JSON")?;

        if !status.is_success() {
            return Ok(LLMResponse {
                content: Some(format!("Error calling LLM: {}", payload)),
                tool_calls: Vec::new(),
                finish_reason: "error".to_string(),
                usage: Map::new(),
            });
        }

        let choice = payload
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|v| v.first())
            .cloned()
            .unwrap_or_else(|| json!({}));

        let message = choice.get("message").cloned().unwrap_or_else(|| json!({}));
        let content = message
            .get("content")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);

        let tool_calls = message
            .get("tool_calls")
            .and_then(Value::as_array)
            .map(|calls| {
                calls
                    .iter()
                    .filter_map(|tc| {
                        let id = tc.get("id")?.as_str()?.to_string();
                        let function = tc.get("function")?;
                        let name = function.get("name")?.as_str()?.to_string();
                        let args_raw = function
                            .get("arguments")
                            .and_then(Value::as_str)
                            .unwrap_or("{}");
                        let args_value: Value = serde_json::from_str(args_raw)
                            .unwrap_or_else(|_| json!({ "raw": args_raw }));
                        let arguments = args_value.as_object().cloned().unwrap_or_default();
                        Some(ToolCallRequest {
                            id,
                            name,
                            arguments,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let finish_reason = choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .unwrap_or("stop")
            .to_string();

        let usage = payload
            .get("usage")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();

        Ok(LLMResponse {
            content,
            tool_calls,
            finish_reason,
            usage,
        })
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }
}
