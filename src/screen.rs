//! Screen mode for redrawing programs (inline TUIs like pi / codex / claude code, alt-screen apps).
//!
//! Those programs erase and repaint their own output with cursor-up / erase-line codes, so
//! inserting translated lines into the stream gets them wiped (or breaks the program's
//! row arithmetic). Instead we:
//!   * pass the program's bytes through untouched,
//!   * feed the same bytes to a virtual terminal (vt100) to know what is on screen,
//!   * once the screen has been stable for STABLE, translate the new text blocks,
//!   * draw the translations in a reserved footer at the bottom of the real terminal
//!     (the child is told the window is `footer` rows shorter, so it never touches it).

use crate::text::should_translate;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthChar;

pub const STABLE: Duration = Duration::from_millis(300);
const MAX_BLOCK_ROWS: usize = 6;
const EDGE_CHARS: &str = "│┃║|╭╮╰╯─━═>›❯•*·▌▐";

/// Footer height for a terminal with `rows` rows (0 = too small, don't use screen mode).
pub fn footer_rows(rows: u16) -> u16 {
    let want: u16 = std::env::var("T_FOOTER").ok().and_then(|v| v.parse().ok()).unwrap_or(6);
    let f = want.min(rows / 3);
    if f < 3 {
        0
    } else {
        f
    }
}

fn clean_row(s: &str) -> String {
    s.trim()
        .trim_matches(|c: char| c.is_whitespace() || EDGE_CHARS.contains(c))
        .to_string()
}

/// Group consecutive translatable screen rows into paragraphs (wrapped text translates
/// much better as one unit than as row fragments).
pub fn blocks_from_rows(rows: &[String], volatile: &mut Volatile) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur: Vec<String> = Vec::new();
    for r in rows {
        let c = clean_row(r);
        if should_translate(&c) && !volatile.is_volatile(&c) {
            cur.push(c);
            if cur.len() >= MAX_BLOCK_ROWS {
                out.push(cur.join(" "));
                cur.clear();
            }
        } else if !cur.is_empty() {
            out.push(cur.join(" "));
            cur.clear();
        }
    }
    if !cur.is_empty() {
        out.push(cur.join(" "));
    }
    out
}

/// Spinners / timers change every frame ("Working (12s)"): a row pattern that shows up with
/// more than 2 different digit values is treated as volatile and never sent for translation
/// (checked per row, before rows are grouped, so it can't poison a paragraph block).
#[derive(Default)]
pub struct Volatile(HashMap<String, HashSet<String>>);

impl Volatile {
    pub fn is_volatile(&mut self, block: &str) -> bool {
        if self.0.len() > 2000 {
            self.0.clear();
        }
        let norm: String = block.chars().map(|c| if c.is_ascii_digit() { '#' } else { c }).collect();
        let set = self.0.entry(norm).or_default();
        set.insert(block.to_string());
        set.len() > 2
    }
}

pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(4);
    let mut lines = Vec::new();
    let mut cur = String::new();
    let mut w = 0;
    for ch in text.chars() {
        let cw = ch.width().unwrap_or(0);
        if w + cw > width {
            lines.push(std::mem::take(&mut cur));
            w = 0;
        }
        cur.push(ch);
        w += cw;
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// `n` footer lines: a header, then the translations of the newest blocks (chronological
/// order) that fit. `blocks` is (original, translation-if-ready) in screen order.
pub fn render_footer(blocks: &[(String, Option<String>)], cols: usize, n: usize) -> Vec<String> {
    let room = n.saturating_sub(1);
    let mut chosen: Vec<Vec<String>> = Vec::new();
    let mut left = room;
    for (_, tr) in blocks.iter().rev() {
        let Some(t) = tr else { continue };
        if left == 0 {
            break;
        }
        let mut lines = wrap(t, cols.saturating_sub(2));
        if lines.len() > left {
            if chosen.is_empty() {
                lines.truncate(left); // newest block alone is too long: show its beginning
            } else {
                break;
            }
        }
        left -= lines.len();
        chosen.push(lines);
    }
    let mut out = vec!["── 翻译 ──".to_string()];
    for b in chosen.into_iter().rev() {
        out.extend(b);
    }
    out.resize(n, String::new());
    out
}

pub struct ScreenState {
    parser: vt100::Parser,
    pub cols: u16,
    pub total_rows: u16,
    pub footer: u16,
    pub by_alt: bool,
    pub region_dirty: bool,
    dirty: bool,
    last_data: Instant,
    trans: HashMap<String, String>,
    requested: HashSet<String>,
    volatile: Volatile,
    blocks: Vec<String>,
}

impl ScreenState {
    pub fn new(cols: u16, total_rows: u16, footer: u16, by_alt: bool) -> Self {
        ScreenState {
            parser: vt100::Parser::new(total_rows - footer, cols, 0),
            cols,
            total_rows,
            footer,
            by_alt,
            region_dirty: true,
            dirty: true,
            last_data: Instant::now(),
            trans: HashMap::new(),
            requested: HashSet::new(),
            volatile: Volatile::default(),
            blocks: Vec::new(),
        }
    }

    pub fn child_rows(&self) -> u16 {
        self.total_rows - self.footer
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
        self.dirty = true;
        self.last_data = Instant::now();
    }

    pub fn resize(&mut self, cols: u16, total_rows: u16, footer: u16) {
        self.cols = cols;
        self.total_rows = total_rows;
        self.footer = footer;
        self.parser.set_size(total_rows - footer, cols);
        self.region_dirty = true;
        self.dirty = true;
        self.last_data = Instant::now();
    }

    /// Time until the screen counts as stable; None if nothing changed since last tick.
    pub fn stable_in(&self) -> Option<Duration> {
        if self.dirty {
            Some(STABLE.saturating_sub(self.last_data.elapsed()))
        } else {
            None
        }
    }

    /// Snapshot the screen; return blocks that still need translating.
    pub fn take_jobs(&mut self) -> Vec<String> {
        self.dirty = false;
        let rows: Vec<String> = self.parser.screen().rows(0, self.cols).collect();
        self.blocks = blocks_from_rows(&rows, &mut self.volatile);
        let mut jobs = Vec::new();
        for b in &self.blocks {
            if self.trans.contains_key(b) || self.requested.contains(b) {
                continue;
            }
            self.requested.insert(b.clone());
            jobs.push(b.clone());
        }
        jobs
    }

    pub fn add_results(&mut self, pairs: Vec<(String, String)>) {
        if self.trans.len() > 5000 {
            self.trans.clear();
            self.requested.clear();
        }
        for (k, v) in pairs {
            self.trans.insert(k, v);
        }
    }

    pub fn footer_lines(&self) -> Vec<String> {
        let blocks: Vec<(String, Option<String>)> =
            self.blocks.iter().map(|b| (b.clone(), self.trans.get(b).cloned())).collect();
        render_footer(&blocks, self.cols as usize, self.footer as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn groups_wrapped_paragraph() {
        let mut v = Volatile::default();
        let b = blocks_from_rows(
            &rows(&[
                "│ I will read the config file and then",
                "│ update the settings accordingly",
                "",
                "$ ls",
                "Running the tests now please wait",
            ]),
            &mut v,
        );
        assert_eq!(b.len(), 2);
        assert_eq!(b[0], "I will read the config file and then update the settings accordingly");
    }

    #[test]
    fn spinner_row_does_not_poison_block() {
        let mut v = Volatile::default();
        let mut last = Vec::new();
        for n in 1..=4 {
            last = blocks_from_rows(
                &rows(&[
                    "I will read the config file and update settings",
                    &format!("Working ({} s) esc to interrupt", n),
                    "Tests are passing and the build finished",
                ]),
                &mut v,
            );
        }
        // by the 3rd frame the timer row is volatile: it splits the text instead of joining it
        assert_eq!(last.len(), 2);
        assert!(last.iter().all(|b| !b.contains("Working")));
    }

    #[test]
    fn volatile_timer() {
        let mut v = Volatile::default();
        let mut flagged = false;
        for i in 0..6 {
            flagged = v.is_volatile(&format!("Working hard ({}s) esc to interrupt", i));
        }
        assert!(flagged);
        assert!(!Volatile::default().is_volatile("Static sentence here"));
    }

    #[test]
    fn footer_tail_and_width() {
        let blocks = vec![
            ("a".to_string(), Some("第一段翻译".to_string())),
            ("b".to_string(), Some("第二段翻译".to_string())),
            ("c".to_string(), None),
        ];
        let f = render_footer(&blocks, 40, 4);
        assert_eq!(f.len(), 4);
        assert_eq!(f[1], "第一段翻译");
        assert_eq!(f[2], "第二段翻译");
        // narrow: only the newest fits
        let f = render_footer(&blocks, 12, 2);
        assert_eq!(f[1], "第二段翻译");
        assert_eq!(wrap("你好世界你好世界", 8), vec!["你好世界", "你好世界"]);
    }
}
