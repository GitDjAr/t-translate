//! OpenAI-compatible chat completions (OpenAI, DeepSeek, Ollama, ...).

use serde_json::{json, Value};
use std::time::Duration;

pub struct OpenAi {
    base: String,
    key: String,
    model: String,
}

impl OpenAi {
    pub fn from_env() -> Self {
        OpenAi {
            base: std::env::var("T_API_BASE").unwrap_or_else(|_| "https://api.openai.com/v1".into()),
            key: std::env::var("T_API_KEY").unwrap_or_default(),
            model: std::env::var("T_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into()),
        }
    }

    pub fn translate(&self, lang: &str, text: &str) -> Option<String> {
        let body = json!({
            "model": self.model,
            "temperature": 0,
            "messages": [
                {"role": "system", "content": format!(
                    "Translate each input line into {lang}. Output exactly the same number of lines, \
                     one translation per line, no numbering, no commentary. Keep code, flags, \
                     paths, URLs and placeholders unchanged.")},
                {"role": "user", "content": text}
            ]
        });
        let v: Value = ureq::post(&format!("{}/chat/completions", self.base.trim_end_matches('/')))
            .set("Authorization", &format!("Bearer {}", self.key))
            .timeout(Duration::from_secs(30))
            .send_json(body)
            .ok()?
            .into_json()
            .ok()?;
        v["choices"][0]["message"]["content"].as_str().map(|s| s.to_string())
    }
}
