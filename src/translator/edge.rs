//! Edge / Bing translator: free token from edge.microsoft.com, array in -> array out.

use serde_json::{json, Value};
use std::cell::RefCell;
use std::time::Duration;

#[derive(Default)]
pub struct Edge {
    token: RefCell<Option<String>>,
}

impl Edge {
    fn token(&self) -> Option<String> {
        let mut t = self.token.borrow_mut();
        if t.is_none() {
            *t = ureq::get("https://edge.microsoft.com/translate/auth")
                .timeout(Duration::from_secs(5))
                .call()
                .ok()
                .and_then(|r| r.into_string().ok());
        }
        t.clone()
    }

    pub fn translate(&self, lang: &str, lines: &[String]) -> Option<Vec<String>> {
        let to = match lang {
            "zh-CN" | "zh" => "zh-Hans",
            "zh-TW" => "zh-Hant",
            o => o,
        };
        let url = format!(
            "https://api-edge.cognitive.microsofttranslator.com/translate?to={to}&api-version=3.0&includeSentenceLength=true"
        );
        for _ in 0..2 {
            let token = self.token()?;
            let body: Vec<Value> = lines.iter().map(|l| json!({ "Text": l })).collect();
            match ureq::post(&url)
                .set("Authorization", &format!("Bearer {token}"))
                .timeout(Duration::from_secs(8))
                .send_json(Value::Array(body))
            {
                Ok(resp) => {
                    let v: Value = resp.into_json().ok()?;
                    let arr = v.as_array()?;
                    if arr.len() != lines.len() {
                        return None;
                    }
                    return arr
                        .iter()
                        .map(|x| x["translations"][0]["text"].as_str().map(|s| s.to_string()))
                        .collect();
                }
                Err(_) => *self.token.borrow_mut() = None, // token expired? refetch once
            }
        }
        None
    }
}
