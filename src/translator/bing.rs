//! Bing Translator web API (no key). Microsoft retired the Edge token endpoint
//! (edge.microsoft.com/translate/auth now 404s), so we do what the web page does:
//! load /translator once, read key/token/IG from the page, POST to /ttranslatev3.
//! Tokens last about an hour; we refresh after 30 minutes or on any failure.
//! Bing redirects by region (www.bing.com <-> cn.bing.com); a redirected POST must stay a
//! POST, so redirects are followed by hand and the regional host is remembered.

use serde_json::Value;
use std::cell::RefCell;
use std::time::{Duration, Instant};

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
const REFRESH: Duration = Duration::from_secs(30 * 60);

struct Session {
    base: String, // e.g. https://www.bing.com or https://cn.bing.com
    key: String,
    token: String,
    ig: String,
    iid: String,
    at: Instant,
    n: u32,
}

#[derive(Default)]
pub struct Bing {
    session: RefCell<Option<Session>>,
}

fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let i = s.find(start)? + start.len();
    let j = s[i..].find(end)? + i;
    Some(&s[i..j])
}

/// "https://cn.bing.com/path?x" -> "https://cn.bing.com"
fn origin(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let host = rest.split('/').next()?;
    Some(format!("{}://{}", url.split_once("://")?.0, host))
}

fn load_session() -> Option<Session> {
    let resp = ureq::get("https://www.bing.com/translator")
        .set("User-Agent", UA)
        .timeout(Duration::from_secs(8))
        .call()
        .ok()?;
    let base = origin(resp.get_url()).unwrap_or_else(|| "https://www.bing.com".into());
    let html = resp.into_string().ok()?;
    let helper = between(&html, "params_AbusePreventionHelper = [", "]")?;
    let mut parts = helper.split(',');
    let key = parts.next()?.trim().to_string();
    let token = parts.next()?.trim().trim_matches('"').to_string();
    let ig = between(&html, "IG:\"", "\"")?.to_string();
    let iid = between(&html, "data-iid=\"", "\"").unwrap_or("translator.5023").to_string();
    Some(Session { base, key, token, ig, iid, at: Instant::now(), n: 0 })
}

fn post(s: &mut Session, to: &str, text: &str) -> Option<String> {
    let agent = ureq::AgentBuilder::new().redirects(0).timeout(Duration::from_secs(8)).build();
    s.n += 1;
    let mut url = format!(
        "{}/ttranslatev3?isVertical=1&IG={}&IID={}.{}",
        s.base, s.ig, s.iid, s.n
    );
    for _ in 0..3 {
        let resp = agent
            .post(&url)
            .set("User-Agent", UA)
            .set("Accept", "*/*")
            .set("Referer", &format!("{}/translator", s.base))
            .send_form(&[
                ("fromLang", "auto-detect"),
                ("to", to),
                ("text", text),
                ("token", &s.token),
                ("key", &s.key),
            ])
            .ok()?;
        if (300..400).contains(&resp.status()) {
            let loc = resp.header("location")?.to_string();
            if let Some(o) = origin(&loc) {
                s.base = o;
            }
            url = loc;
            continue;
        }
        let v: Value = serde_json::from_str(&resp.into_string().ok()?).ok()?;
        return v[0]["translations"][0]["text"].as_str().map(|t| t.to_string());
    }
    None
}

impl Bing {
    /// One request; lines are joined with \n (Bing keeps line breaks).
    pub fn translate(&self, lang: &str, lines: &[String]) -> Option<Vec<String>> {
        let to = match lang {
            "zh-CN" | "zh" => "zh-Hans",
            "zh-TW" => "zh-Hant",
            o => o,
        };
        for _ in 0..2 {
            let mut guard = self.session.borrow_mut();
            if guard.as_ref().map_or(true, |s| s.at.elapsed() > REFRESH) {
                *guard = load_session();
            }
            let s = guard.as_mut()?;
            match post(s, to, &lines.join("\n")) {
                Some(out) => {
                    let mut parts: Vec<String> = out.split('\n').map(|x| x.to_string()).collect();
                    while parts.len() > lines.len() && parts.last().map_or(false, |x| x.trim().is_empty()) {
                        parts.pop();
                    }
                    return if parts.len() == lines.len() { Some(parts) } else { None };
                }
                None => *guard = None, // token expired / rate limited -> reload once
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_origin_and_page_params() {
        assert_eq!(origin("https://cn.bing.com/translator?x=1").as_deref(), Some("https://cn.bing.com"));
        let html = r#"var params_AbusePreventionHelper = [1791184277307,"abcTOKEN",3600000]; _G={IG:"63AF04"}"#;
        assert_eq!(between(html, "IG:\"", "\""), Some("63AF04"));
        assert!(between(html, "params_AbusePreventionHelper = [", "]").unwrap().starts_with("1791184277307,"));
    }
}
