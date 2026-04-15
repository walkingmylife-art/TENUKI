//! LLMクライアント（HTTP専用）

use std::sync::Arc;
use std::time::Duration;
use serde_json::json;

/// LLMクライアントトレイト
pub trait LlmClient: Send + Sync {
    fn translate_sync(&self, text: &str, prefix: &str) -> Option<String>;
}

/// llama-server へ HTTP でリクエストする実装（ureq ベース）
#[derive(Clone)]
pub struct HttpLlmClient {
    endpoint: Arc<String>,
    timeout_connect: Duration,
    timeout_read: Duration,
}

fn build_translation_message(prefix: &str, text: &str) -> String {
    format!("{}\n\n{}", prefix.trim(), text.trim())
}

impl HttpLlmClient {
    pub fn new(endpoint: String) -> Self {
        Self {
            endpoint: Arc::new(endpoint),
            timeout_connect: Duration::from_secs(10),
            timeout_read: Duration::from_secs(60),
        }
    }

    pub fn with_timeouts(mut self, connect: Duration, read: Duration) -> Self {
        self.timeout_connect = connect;
        self.timeout_read = read;
        self
    }

    fn call_internal(&self, text: &str, prefix: &str) -> Option<String> {
        if text.trim().is_empty() {
            return Some(text.to_string());
        }

        let prompt = build_translation_message(prefix, text);

        let client = ureq::AgentBuilder::new()
            .timeout_connect(self.timeout_connect)
            .timeout_read(self.timeout_read)
            .build();

        let payload = json!({
            "messages": [
                { "role": "user", "content": prompt }
            ],
            "max_tokens": 512,
            "temperature": 0.7,
            "top_k": 20,
            "top_p": 0.6,
            "repetition_penalty": 1.05,
            "cache_prompt": false
        });

        match client.post(self.endpoint.as_str()).send_json(payload) {
            Ok(resp) => {
                let body = resp.into_string().unwrap_or_default();
                serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|j| {
                        j["choices"][0]["message"]["content"]
                            .as_str()
                            .or_else(|| j["choices"][0]["text"].as_str())
                            .or_else(|| j["content"].as_str())
                            .map(|s| s.to_string())
                    })
            }
            Err(_) => None,
        }
    }
}

impl LlmClient for HttpLlmClient {
    fn translate_sync(&self, text: &str, prefix: &str) -> Option<String> {
        self.call_internal(text, prefix)
    }
}

#[cfg(test)]
mod tests {
    use super::build_translation_message;

    #[test]
    fn builds_single_translation_message() {
        assert_eq!(
            build_translation_message(
                "Translate the following segment into Japanese, without additional explanation.",
                "Hello world"
            ),
            "Translate the following segment into Japanese, without additional explanation.\n\nHello world",
        );
    }
}
