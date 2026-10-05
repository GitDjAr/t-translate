//! Translator: cache + backend selection (auto = Google if reachable in 3s, else Edge/Bing).

mod cache;
mod edge;
mod google;
mod openai;

pub use cache::Cache;

const GOOGLE_PROBE_SECS: u64 = 3;

#[derive(Clone, Copy, PartialEq)]
enum Backend {
    Google,
    Edge,
    OpenAi,
}

pub struct Translator {
    lang: String,
    backend: Backend,
    auto: bool,
    resolved: bool,
    edge: edge::Edge,
    openai: openai::OpenAi,
    cache: Cache,
}

fn split_lines(out: String, n: usize) -> Option<Vec<String>> {
    let mut parts: Vec<String> = out.split('\n').map(|s| s.to_string()).collect();
    while parts.len() > n && parts.last().map(|s| s.trim().is_empty()).unwrap_or(false) {
        parts.pop();
    }
    if parts.len() == n {
        Some(parts)
    } else {
        None
    }
}

impl Translator {
    pub fn new(lang: String, use_cache: bool) -> Self {
        let name = std::env::var("T_BACKEND").unwrap_or_else(|_| "auto".into());
        let (backend, auto) = match name.as_str() {
            "google" => (Backend::Google, false),
            "edge" | "bing" => (Backend::Edge, false),
            "openai" => (Backend::OpenAi, false),
            _ => (Backend::Google, true),
        };
        Translator {
            lang,
            backend,
            auto,
            resolved: !auto,
            edge: edge::Edge::default(),
            openai: openai::OpenAi::from_env(),
            cache: Cache::new(use_cache),
        }
    }

    /// auto mode: probe Google once (3s budget), otherwise use Edge/Bing.
    fn ensure_backend(&mut self) {
        if self.resolved {
            return;
        }
        self.resolved = true;
        if google::translate(&self.lang, "hello", GOOGLE_PROBE_SECS).is_none() {
            self.backend = Backend::Edge;
        }
    }

    pub fn save(&mut self) {
        self.cache.save();
    }

    /// Translate many lines. Returns one Option per input.
    pub fn translate_batch(&mut self, texts: &[String]) -> Vec<Option<String>> {
        let mut result: Vec<Option<String>> = vec![None; texts.len()];
        let mut todo: Vec<usize> = Vec::new();
        for (i, t) in texts.iter().enumerate() {
            match self.cache.get(&Cache::key(&self.lang, t)) {
                Some(v) => result[i] = Some(v),
                None => todo.push(i),
            }
        }
        if !todo.is_empty() {
            self.ensure_backend();
        }
        // chunk by size so GET urls stay short
        let mut start = 0;
        while start < todo.len() {
            let mut end = start;
            let mut size = 0;
            while end < todo.len() && (size < 1500 || end == start) {
                size += texts[todo[end]].len() + 1;
                end += 1;
            }
            let idxs = &todo[start..end];
            let chunk: Vec<String> = idxs.iter().map(|&i| texts[i].trim().to_string()).collect();
            let mut first = self.call_backend(&chunk);
            if first.is_none() && self.auto && self.backend == Backend::Google {
                // google died mid-run -> switch to edge for the rest of the session
                self.backend = Backend::Edge;
                first = self.call_backend(&chunk);
            }
            let translated = first.or_else(|| {
                // fall back to one-by-one
                let singles: Vec<Option<String>> = chunk
                    .iter()
                    .map(|c| self.call_backend(std::slice::from_ref(c)).and_then(|mut v| v.pop()))
                    .collect();
                if singles.iter().all(|s| s.is_none()) {
                    None
                } else {
                    Some(singles.into_iter().map(|s| s.unwrap_or_default()).collect())
                }
            });
            if let Some(tr) = translated {
                for (k, &i) in idxs.iter().enumerate() {
                    let v = tr[k].trim().to_string();
                    if !v.is_empty() {
                        self.cache.insert(Cache::key(&self.lang, &texts[i]), v.clone());
                        result[i] = Some(v);
                    }
                }
            }
            start = end;
        }
        self.cache.save(); // persist right away so Ctrl+C / tail -f don't lose the cache
        result
    }

    /// Some(vec) with exactly lines.len() entries, or None on failure/mismatch.
    fn call_backend(&self, lines: &[String]) -> Option<Vec<String>> {
        match self.backend {
            Backend::Edge => self.edge.translate(&self.lang, lines),
            Backend::Google => {
                split_lines(google::translate(&self.lang, &lines.join("\n"), 8)?, lines.len())
            }
            Backend::OpenAi => {
                split_lines(self.openai.translate(&self.lang, &lines.join("\n"))?, lines.len())
            }
        }
    }
}
