//! Writing original output + aligned, highlighted translation to the terminal.

use crate::text::{split_desc, should_translate, strip_ansi};
use crate::translator::Translator;
use std::collections::HashMap;
use std::io::Write;

// translation is the thing you read -> bright cyan; original stays as-is
const HL: &str = "\x1b[96m";
const RESET: &str = "\x1b[0m";

pub fn write_out(bytes: &[u8]) {
    let mut o = std::io::stdout().lock();
    let _ = o.write_all(bytes);
    let _ = o.flush();
}

/// Print complete lines, each followed by its translation (if any).
pub fn flush_lines(tr: &mut Translator, lines: &mut Vec<Vec<u8>>) {
    if lines.is_empty() {
        return;
    }
    let plains: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
    let mut idx = Vec::new();
    let mut texts = Vec::new();
    let mut cols: HashMap<usize, usize> = HashMap::new();
    for (i, p) in plains.iter().enumerate() {
        let (col, text) = split_desc(p);
        if should_translate(&text) {
            idx.push(i);
            texts.push(text);
            cols.insert(i, col);
        }
    }
    let translated = if texts.is_empty() { Vec::new() } else { tr.translate_batch(&texts) };
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
            let col = cols.get(&i).copied().unwrap_or(0);
            out.extend_from_slice(format!("{}{}{}{}\r\n", " ".repeat(col), HL, t, RESET).as_bytes());
        }
    }
    write_out(&out);
    lines.clear();
}

/// Unfinished line that looks like a prompt: print it and append "(译文)" inline.
pub fn prompt_inline(tr: &mut Translator, buf: &mut Vec<u8>) {
    if buf.is_empty() {
        return;
    }
    let plain = strip_ansi(buf);
    let mut out = buf.clone();
    if should_translate(&plain) {
        if let Some(Some(t)) = tr.translate_batch(&[plain]).into_iter().next() {
            out.extend_from_slice(format!(" {}({}){}", HL, t, RESET).as_bytes());
        }
    }
    write_out(&out);
    buf.clear();
}
