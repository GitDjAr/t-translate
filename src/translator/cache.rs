//! On-disk translation cache (~/.t-translate/cache.json), entries expire after 30 days.

use std::collections::HashMap;
use std::path::PathBuf;

const TTL_SECS: u64 = 30 * 24 * 3600;

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub struct Cache {
    map: HashMap<String, (String, u64)>, // key -> (translation, unix ts)
    path: Option<PathBuf>,
    dirty: bool,
    enabled: bool,
}

impl Cache {
    pub fn new(enabled: bool) -> Self {
        let path = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .ok()
            .map(|h| PathBuf::from(h).join(".t-translate").join("cache.json"));
        let mut map: HashMap<String, (String, u64)> = HashMap::new();
        if enabled {
            if let Some(s) = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()) {
                if let Ok(m) = serde_json::from_str::<HashMap<String, (String, u64)>>(&s) {
                    let now = now_secs();
                    map = m
                        .into_iter()
                        .filter(|(_, (_, ts))| now.saturating_sub(*ts) < TTL_SECS)
                        .collect();
                }
            }
        }
        Cache { map, path, dirty: false, enabled }
    }

    pub fn key(lang: &str, text: &str) -> String {
        format!("{}\u{0}{}", lang, text.trim())
    }

    pub fn get(&self, key: &str) -> Option<String> {
        self.map.get(key).map(|(v, _)| v.clone())
    }

    pub fn insert(&mut self, key: String, value: String) {
        self.map.insert(key, (value, now_secs()));
        self.dirty = true;
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn path(&self) -> Option<&PathBuf> {
        self.path.as_ref()
    }

    /// Delete the cache file. Returns true if something was removed.
    pub fn clear(&mut self) -> bool {
        self.map.clear();
        self.dirty = false;
        self.path
            .as_ref()
            .map(|p| std::fs::remove_file(p).is_ok())
            .unwrap_or(false)
    }

    pub fn save(&mut self) {
        if !self.dirty || !self.enabled {
            return;
        }
        self.dirty = false;
        if let Some(p) = &self.path {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(s) = serde_json::to_string(&self.map) {
                let _ = std::fs::write(p, s);
            }
        }
    }
}
