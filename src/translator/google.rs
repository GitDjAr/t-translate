//! Google Translate free web endpoint (no key, unofficial).

use serde_json::Value;
use std::time::Duration;

pub fn translate(lang: &str, text: &str, timeout_secs: u64) -> Option<String> {
    let v: Value = ureq::get("https://translate.googleapis.com/translate_a/single")
        .query("client", "gtx")
        .query("sl", "auto")
        .query("tl", lang)
        .query("dt", "t")
        .query("q", text)
        .timeout(Duration::from_secs(timeout_secs))
        .call()
        .ok()?
        .into_json()
        .ok()?;
    let arr = v.get(0)?.as_array()?;
    let mut s = String::new();
    for seg in arr {
        if let Some(t) = seg.get(0).and_then(|x| x.as_str()) {
            s.push_str(t);
        }
    }
    Some(s)
}
