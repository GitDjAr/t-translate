//! t — run any command through a PTY and print its output bilingual.
//!
//!   t git -h
//!   t --lang ja docker logs -f web
//!
//! Env:
//!   T_LANG       target language (default zh-CN)
//!   T_BACKEND    google (default) | openai
//!   T_API_BASE   openai-compatible base url (default https://api.openai.com/v1)
//!   T_API_KEY    api key for openai backend
//!   T_MODEL      model name (default gpt-4o-mini)

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

const DIM: &str = "\x1b[90m";
const RESET: &str = "\x1b[0m";
const BATCH_LINES: usize = 30;
const IDLE: Duration = Duration::from_millis(200);

enum Msg {
    Data(Vec<u8>),
    Exit(u32),
}

// ---------------------------------------------------------------- text utils

/// Remove ANSI escape sequences (CSI, OSC, simple ESC x) and decode lossily.
fn strip_ansi(raw: &[u8]) -> String {
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let b = raw[i];
        if b == 0x1b {
            i += 1;
            if i >= raw.len() {
                break;
            }
            match raw[i] {
                b'[' => {
                    i += 1;
                    while i < raw.len() && !(0x40..=0x7e).contains(&raw[i]) {
                        i += 1;
                    }
                    i += 1;
                }
                b']' => {
                    i += 1;
                    while i < raw.len() {
                        if raw[i] == 0x07 {
                            i += 1;
                            break;
                        }
                        if raw[i] == 0x1b && i + 1 < raw.len() && raw[i + 1] == b'\\' {
                            i += 2;
                            break;
                        }
                        i += 1;
                    }
                }
                _ => i += 1,
            }
        } else if b == b'\r' || b == b'\n' {
            i += 1; // drop line endings
        } else {
            out.push(b);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Heuristic: is this line worth translating?
fn should_translate(s: &str) -> bool {
    let t = s.trim();
    if t.len() < 4 {
        return false;
    }
    // already contains CJK -> skip
    if t.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)) {
        return false;
    }
    let words = t
        .split_whitespace()
        .filter(|w| w.chars().filter(|c| c.is_ascii_alphabetic()).count() >= 3)
        .count();
    words >= 2 || (words >= 1 && (t.contains('?') || t.ends_with(':')))
}

fn leading_ws(s: &str) -> &str {
    &s[..s.len() - s.trim_start().len()]
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

// ---------------------------------------------------------------- translator

struct Translator {
    lang: String,
    backend: String,
    api_base: String,
    api_key: String,
    model: String,
    cache: HashMap<String, String>,
    cache_path: Option<PathBuf>,
    dirty: bool,
    use_cache: bool,
}

impl Translator {
    fn new(lang: String, use_cache: bool) -> Self {
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .ok()
            .map(PathBuf::from);
        let cache_path = home.map(|h| h.join(".t-translate").join("cache.json"));
        let mut cache = HashMap::new();
        if use_cache {
            if let Some(p) = &cache_path {
                if let Ok(s) = std::fs::read_to_string(p) {
                    if let Ok(m) = serde_json::from_str::<HashMap<String, String>>(&s) {
                        cache = m;
                    }
                }
            }
        }
        Translator {
            lang,
            backend: std::env::var("T_BACKEND").unwrap_or_else(|_| "google".into()),
            api_base: std::env::var("T_API_BASE")
                .unwrap_or_else(|_| "https://api.openai.com/v1".into()),
            api_key: std::env::var("T_API_KEY").unwrap_or_default(),
            model: std::env::var("T_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into()),
            cache,
            cache_path,
            dirty: false,
            use_cache,
        }
    }

    fn save(&self) {
        if !self.dirty || !self.use_cache {
            return;
        }
        if let Some(p) = &self.cache_path {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(s) = serde_json::to_string(&self.cache) {
                let _ = std::fs::write(p, s);
            }
        }
    }

    fn key(&self, text: &str) -> String {
        format!("{}\u{0}{}", self.lang, text.trim())
    }

    /// Translate many lines. Returns one Option per input.
    fn translate_batch(&mut self, texts: &[String]) -> Vec<Option<String>> {
        let mut result: Vec<Option<String>> = vec![None; texts.len()];
        let mut todo: Vec<usize> = Vec::new();
        for (i, t) in texts.iter().enumerate() {
            if let Some(v) = self.cache.get(&self.key(t)) {
                result[i] = Some(v.clone());
            } else {
                todo.push(i);
            }
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
            let translated = self.call_backend(&chunk).or_else(|| {
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
                        let key = self.key(&texts[i]);
                        self.cache.insert(key, v.clone());
                        self.dirty = true;
                        result[i] = Some(v);
                    }
                }
            }
            start = end;
        }
        result
    }

    /// Returns Some(vec) with exactly lines.len() entries, or None on failure/mismatch.
    fn call_backend(&self, lines: &[String]) -> Option<Vec<String>> {
        let joined = lines.join("\n");
        let out = if self.backend == "openai" {
            self.openai(&joined)?
        } else {
            self.google(&joined)?
        };
        let mut parts: Vec<String> = out.split('\n').map(|s| s.to_string()).collect();
        while parts.len() > lines.len() && parts.last().map(|s| s.trim().is_empty()).unwrap_or(false)
        {
            parts.pop();
        }
        if parts.len() == lines.len() {
            Some(parts)
        } else {
            None
        }
    }

    fn google(&self, text: &str) -> Option<String> {
        let v: Value = ureq::get("https://translate.googleapis.com/translate_a/single")
            .query("client", "gtx")
            .query("sl", "auto")
            .query("tl", &self.lang)
            .query("dt", "t")
            .query("q", text)
            .timeout(Duration::from_secs(8))
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

    fn openai(&self, text: &str) -> Option<String> {
        let body = json!({
            "model": self.model,
            "temperature": 0,
            "messages": [
                {"role": "system", "content": format!(
                    "Translate each input line into {}. Output exactly the same number of lines, \
                     one translation per line, no numbering, no commentary. Keep code, flags, \
                     paths, URLs and placeholders unchanged.", self.lang)},
                {"role": "user", "content": text}
            ]
        });
        let v: Value = ureq::post(&format!("{}/chat/completions", self.api_base.trim_end_matches('/')))
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .timeout(Duration::from_secs(30))
            .send_json(body)
            .ok()?
            .into_json()
            .ok()?;
        v["choices"][0]["message"]["content"]
            .as_str()
            .map(|s| s.to_string())
    }
}

// ---------------------------------------------------------------- output

fn write_out(bytes: &[u8]) {
    let mut o = std::io::stdout().lock();
    let _ = o.write_all(bytes);
    let _ = o.flush();
}

/// Print complete lines, each followed by its translation (if any).
fn flush_lines(tr: &mut Translator, lines: &mut Vec<Vec<u8>>) {
    if lines.is_empty() {
        return;
    }
    let plains: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
    let mut idx = Vec::new();
    let mut texts = Vec::new();
    for (i, p) in plains.iter().enumerate() {
        if should_translate(p) {
            idx.push(i);
            texts.push(p.clone());
        }
    }
    let translated = if texts.is_empty() {
        Vec::new()
    } else {
        tr.translate_batch(&texts)
    };
    let mut map: HashMap<usize, String> = HashMap::new();
    for (k, i) in idx.iter().enumerate() {
        if let Some(t) = &translated[k] {
            map.insert(*i, t.clone());
        }
    }
    let mut out: Vec<u8> = Vec::new();
    for (i, raw) in lines.iter().enumerate() {
        out.extend_from_slice(raw);
        if let Some(t) = map.get(&i) {
            out.extend_from_slice(
                format!("{}{}{}{}\r\n", DIM, leading_ws(&plains[i]), t, RESET).as_bytes(),
            );
        }
    }
    write_out(&out);
    lines.clear();
}

/// Partial line sitting idle = probably an interactive prompt. Translate inline.
fn flush_prompt(tr: &mut Translator, buf: &mut Vec<u8>) -> bool {
    if buf.is_empty() {
        return false;
    }
    let plain = strip_ansi(buf);
    let mut out = buf.clone();
    if should_translate(&plain) {
        if let Some(Some(t)) = tr.translate_batch(&[plain]).into_iter().next() {
            out.extend_from_slice(format!(" {}({}){}", DIM, t, RESET).as_bytes());
        }
    }
    write_out(&out);
    buf.clear();
    true
}

// ---------------------------------------------------------------- main

fn usage() {
    eprintln!(
        "t - bilingual command output\n\n\
         usage: t [--lang <code>] [--no-cache] <command> [args...]\n\
         \n\
         example: t git -h\n\
         env: T_LANG, T_BACKEND(google|openai), T_API_BASE, T_API_KEY, T_MODEL"
    );
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut lang = std::env::var("T_LANG").unwrap_or_else(|_| "zh-CN".into());
    let mut use_cache = true;
    loop {
        match args.first().map(|s| s.as_str()) {
            Some("--lang") if args.len() >= 2 => {
                lang = args[1].clone();
                args.drain(0..2);
            }
            Some("--no-cache") => {
                use_cache = false;
                args.remove(0);
            }
            _ => break,
        }
    }
    if args.is_empty() || args[0] == "-h" || args[0] == "--help" {
        usage();
        std::process::exit(if args.is_empty() { 2 } else { 0 });
    }

    let (cols, rows) = crossterm::terminal::size().unwrap_or((120, 30));
    let pty = native_pty_system();
    let pair = match pty.openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 }) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("t: cannot open pty: {e}");
            std::process::exit(1);
        }
    };

    // On Windows, resolve .cmd/.bat shims (npm-installed tools like codex) via cmd.exe.
    let mut cmd = if cfg!(windows) {
        let mut c = CommandBuilder::new("cmd");
        c.arg("/c");
        c.args(&args);
        c
    } else {
        let mut c = CommandBuilder::new(&args[0]);
        c.args(&args[1..]);
        c
    };
    if let Ok(cwd) = std::env::current_dir() {
        cmd.cwd(cwd);
    }

    let mut child = match pair.slave.spawn_command(cmd) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("t: failed to start '{}': {e}", args[0]);
            std::process::exit(127);
        }
    };
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("pty reader");
    let mut writer = pair.master.take_writer().expect("pty writer");

    let raw_ok = crossterm::terminal::enable_raw_mode().is_ok();

    let (tx, rx) = mpsc::channel::<Msg>();

    // pty -> channel
    let tx_r = tx.clone();
    std::thread::spawn(move || {
        let mut b = [0u8; 4096];
        loop {
            match reader.read(&mut b) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx_r.send(Msg::Data(b[..n].to_vec())).is_err() {
                        break;
                    }
                }
            }
        }
    });

    // child exit -> channel
    let tx_e = tx.clone();
    std::thread::spawn(move || {
        let code = child.wait().map(|s| s.exit_code()).unwrap_or(1);
        let _ = tx_e.send(Msg::Exit(code));
    });
    drop(tx);

    // keyboard -> pty
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        let mut b = [0u8; 1024];
        loop {
            match stdin.read(&mut b) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if writer.write_all(&b[..n]).is_err() {
                        break;
                    }
                    let _ = writer.flush();
                }
            }
        }
    });

    let mut tr = Translator::new(lang, use_cache);
    let mut lines: Vec<Vec<u8>> = Vec::new();
    let mut buf: Vec<u8> = Vec::new();
    let mut passthrough = false; // inside alternate screen (TUI)
    let mut skip_to_newline = false; // after a prompt: user echo, don't translate
    let mut exit_code: Option<u32> = None;

    loop {
        match rx.recv_timeout(if exit_code.is_some() { Duration::from_millis(150) } else { IDLE }) {
            Ok(Msg::Exit(c)) => {
                exit_code = Some(c);
            }
            Ok(Msg::Data(chunk)) => {
                // alternate-screen detection
                let enter = contains(&chunk, b"\x1b[?1049h")
                    || contains(&chunk, b"\x1b[?1047h")
                    || contains(&chunk, b"\x1b[?47h");
                let leave = contains(&chunk, b"\x1b[?1049l")
                    || contains(&chunk, b"\x1b[?1047l")
                    || contains(&chunk, b"\x1b[?47l");
                if enter && !passthrough {
                    flush_lines(&mut tr, &mut lines);
                    if !buf.is_empty() {
                        write_out(&buf);
                        buf.clear();
                    }
                    passthrough = true;
                }
                if passthrough {
                    write_out(&chunk);
                    if leave {
                        passthrough = false;
                    }
                    continue;
                }

                let mut rest: &[u8] = &chunk;
                while !rest.is_empty() {
                    if skip_to_newline {
                        match rest.iter().position(|&b| b == b'\n') {
                            Some(p) => {
                                write_out(&rest[..=p]);
                                skip_to_newline = false;
                                rest = &rest[p + 1..];
                            }
                            None => {
                                write_out(rest);
                                rest = &[];
                            }
                        }
                        continue;
                    }
                    match rest.iter().position(|&b| b == b'\n') {
                        Some(p) => {
                            buf.extend_from_slice(&rest[..=p]);
                            lines.push(std::mem::take(&mut buf));
                            rest = &rest[p + 1..];
                        }
                        None => {
                            buf.extend_from_slice(rest);
                            rest = &[];
                        }
                    }
                }
                if lines.len() >= BATCH_LINES {
                    flush_lines(&mut tr, &mut lines);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if !passthrough {
                    flush_lines(&mut tr, &mut lines);
                    if exit_code.is_none() && flush_prompt(&mut tr, &mut buf) {
                        skip_to_newline = true;
                    }
                }
                if exit_code.is_some() {
                    break; // child exited and output drained
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    flush_lines(&mut tr, &mut lines);
    if !buf.is_empty() {
        write_out(&buf);
    }
    tr.save();
    if raw_ok {
        let _ = crossterm::terminal::disable_raw_mode();
    }
    drop(pair.master);
    std::process::exit(exit_code.unwrap_or(0) as i32);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_ansi() {
        assert_eq!(strip_ansi(b"\x1b[31mhello\x1b[0m world\r\n"), "hello world");
        assert_eq!(strip_ansi(b"\x1b]0;title\x07abc"), "abc");
    }

    #[test]
    fn filters() {
        assert!(should_translate("  -h, --help      Show help for a command"));
        assert!(should_translate("Continue? (y/N)"));
        assert!(!should_translate("/usr/local/bin/git"));
        assert!(!should_translate("a1b2c3d4e5f6"));
        assert!(!should_translate("已经是中文的一行"));
    }
}
