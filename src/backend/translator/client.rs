//! LLMクライアント（HTTP専用）

use serde_json::json;
use std::sync::Arc;

/// LLMクライアントトレイト
pub trait LlmClient: Send + Sync {
    fn translate_sync(&self, text: &str, prefix: &str) -> Option<String>;
}

/// llama-server へ HTTP でリクエストする実装（ureq ベース）
#[derive(Clone)]
pub struct HttpLlmClient {
    endpoint: Arc<String>,
    agent: ureq::Agent,
}

fn build_translation_message(prefix: &str, text: &str) -> String {
    if prefix.contains("{source_text}") {
        prefix.replace("{source_text}", text.trim())
    } else {
        format!("{}\n\n{}", prefix.trim(), text.trim())
    }
}

impl HttpLlmClient {
    pub fn new(endpoint: String) -> Self {
        Self {
            endpoint: Arc::new(endpoint),
            agent: ureq::Agent::new(),
        }
    }

    fn call_internal(&self, text: &str, prefix: &str) -> Option<String> {
        if text.trim().is_empty() {
            return Some(text.to_string());
        }

        let prompt = build_translation_message(prefix, text);

        let payload = json!({
            "messages": [
                { "role": "user", "content": prompt }
            ],
            "max_tokens": 512,
            "temperature": 0.4,
            "top_k": 20,
            "top_p": 0.6,
            "repetition_penalty": 1.05,
            "cache_prompt": true
        });

        match self.agent.post(self.endpoint.as_str()).send_json(payload) {
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

    #[test]
    fn builds_message_with_source_text_placeholder() {
        assert_eq!(
            build_translation_message(
                "Translate: {source_text}",
                "Hello world"
            ),
            "Translate: Hello world",
        );
    }
}
