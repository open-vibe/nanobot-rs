use anyhow::Result;
use reqwest::multipart::{Form, Part};
use serde_json::Value;
use std::path::Path;

#[derive(Clone)]
pub struct GroqTranscriptionProvider {
    api_key: String,
    api_url: String,
}

impl GroqTranscriptionProvider {
    pub fn new(api_key: Option<String>) -> Self {
        Self {
            api_key: api_key
                .or_else(|| std::env::var("GROQ_API_KEY").ok())
                .unwrap_or_default(),
            api_url: "https://api.groq.com/openai/v1/audio/transcriptions".to_string(),
        }
    }

    pub async fn transcribe(&self, file_path: &Path) -> Result<String> {
        if self.api_key.is_empty() || !file_path.exists() {
            return Ok(String::new());
        }

        let bytes = tokio::fs::read(file_path).await?;
        let part = Part::bytes(bytes).file_name(
            file_path
                .file_name()
                .and_then(|v| v.to_str())
                .unwrap_or("audio.bin")
                .to_string(),
        );
        let form = Form::new()
            .part("file", part)
            .text("model", "whisper-large-v3");

        let client = reqwest::Client::new();
        let response = client
            .post(&self.api_url)
            .bearer_auth(&self.api_key)
            .multipart(form)
            .timeout(std::time::Duration::from_secs(60))
            .send()
            .await?;

        if !response.status().is_success() {
            return Ok(String::new());
        }

        let value: Value = response
            .json()
            .await
            .unwrap_or_else(|_| serde_json::json!({}));
        Ok(value
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string())
    }
}
