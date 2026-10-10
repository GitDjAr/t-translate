//! Translator: cache + backend selection (auto = Google if reachable in 3s, else Bing).

mod cache;
mod bing;
mod google;
mod openai;

pub use cache::Cache;

const GOOGLE_PROBE_SECS: u64 = 3;

#[derive(Clone, Copy, PartialEq)]
enum Backend {
    Google,
    Bing,
    OpenAi,
}

pub struct Translator {
    lang: String,
    backend: Backend,
    auto: bool,
    resolved: bool,
    bing: bing::Bing,
    openai: openai::OpenAi,
    cache: Cache,
    glossary: Vec<String>,
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
            "edge" | "bing" => (Backend::Bing, false),
            "openai" => (Backend::OpenAi, false),
            _ => (Backend::Google, true),
        };
        Translator {
            lang,
            backend,
            auto,
            resolved: !auto,
            bing: bing::Bing::default(),
            openai: openai::OpenAi::from_env(),
            cache: Cache::new(use_cache),
            glossary: crate::text::load_glossary(),
        }
    }

    /// auto mode: probe Google once (3s budget), otherwise use Bing.
    fn ensure_backend(&mut self) {
        if self.resolved {
            return;
        }
        self.resolved = true;
        if google::translate(&self.lang, "hello", GOOGLE_PROBE_SECS).is_none() {
            self.backend = Backend::Bing;
        }
    }

    pub fn save(&mut self) {
        self.cache.save();
    }

    /// Translate many lines. Returns one Option per input.
    ///
    /// TODO(privacy, 2026-10-10): printed output sometimes contains secrets
    /// (token/key/secret/password/pwd/api_key). Detect such patterns here and
    /// either skip translation or redact them before calling backends.
    /// Deferred per user request.
    pub fn translate_batch(&mut self, texts: &[String]) -> Vec<Option<String>> {
        // Mask terms that must not be translated (`code`, --flags, win10, ...).
        // Cache keys keep using the original text so entries stay stable.
        let masked: Vec<(String, Vec<String>)> = texts
            .iter()
            .map(|t| crate::text::protect_terms(t, &self.glossary))
            .collect();
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
                size += masked[todo[end]].0.len() + 1;
                end += 1;
            }
            let idxs = &todo[start..end];
            let chunk: Vec<String> = idxs.iter().map(|&i| masked[i].0.trim().to_string()).collect();
            let mut first = self.call_backend(&chunk);
            if first.is_none() && self.auto && self.backend == Backend::Google {
                // google died mid-run -> switch to bing for the rest of the session
                self.backend = Backend::Bing;
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
                    let v = crate::text::restore_terms(tr[k].trim(), &masked[i].1);
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
            Backend::Bing => self.bing.translate(&self.lang, lines),
            Backend::Google => {
                split_lines(google::translate(&self.lang, &lines.join("\n"), 8)?, lines.len())
            }
            Backend::OpenAi => {
                split_lines(self.openai.translate(&self.lang, &lines.join("\n"))?, lines.len())
            }
        }
    }
}
