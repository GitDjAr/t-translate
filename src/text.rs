//! Pure text helpers: ANSI stripping and "is this worth translating" heuristics.

/// Remove ANSI escape sequences (CSI, OSC, simple ESC x) and line endings.
pub fn strip_ansi(raw: &[u8]) -> String {
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
            i += 1;
        } else {
            out.push(b);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Heuristic: is this line worth translating?
pub fn should_translate(s: &str) -> bool {
    let t = s.trim();
    if t.len() < 4 {
        return false;
    }
    if t.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)) {
        return false; // already CJK
    }
    let words = t
        .split_whitespace()
        .filter(|w| w.chars().filter(|c| c.is_ascii_alphabetic()).count() >= 3)
        .count();
    words >= 2 || (words >= 1 && (t.contains('?') || t.ends_with(':')))
}

pub fn leading_ws(s: &str) -> &str {
    &s[..s.len() - s.trim_start().len()]
}

/// For columnar help text ("  -h, --help      Show help") return the column where the
/// description starts plus the description itself, so the translation can be aligned
/// under it. Otherwise (indent, whole trimmed line).
pub fn split_desc(s: &str) -> (usize, String) {
    let ws = leading_ws(s).chars().count();
    let body = s.trim_start();
    let b = body.as_bytes();
    let mut i = 0;
    while i + 1 < b.len() {
        if b[i] == b' ' && b[i + 1] == b' ' {
            let head = &body[..i];
            let tail = &body[i..];
            let rest = tail.trim_start();
            let gap = tail.len() - rest.len();
            if !head.is_empty()
                && head.chars().count() <= 40
                && head.split_whitespace().count() <= 4
                && should_translate(rest)
            {
                return (ws + head.chars().count() + gap, rest.trim_end().to_string());
            }
            break;
        }
        i += 1;
    }
    (ws, body.trim_end().to_string())
}

/// Does an unfinished line look like an interactive prompt waiting for input?
pub fn looks_like_prompt(plain: &str) -> bool {
    let t = plain.trim_end();
    t.ends_with(|c| matches!(c, ':' | '?' | ')' | ']' | '>' | '\u{ff1a}'))
}

/// A carriage return that is followed by more text (progress bars, spinners).
pub fn has_lone_cr(p: &[u8]) -> bool {
    p.iter()
        .enumerate()
        .any(|(i, &b)| b == b'\r' && i + 1 < p.len() && p[i + 1] != b'\n')
}

/// Does the output repaint itself (cursor up / erase line / erase display / cursor home)?
/// Such programs can't have lines inserted into their stream.
pub fn has_redraw(b: &[u8]) -> bool {
    let mut i = 0;
    while i + 2 < b.len() {
        if b[i] == 0x1b && b[i + 1] == b'[' {
            let mut j = i + 2;
            while j < b.len() && (b[j].is_ascii_digit() || b[j] == b';' || b[j] == b'?') {
                j += 1;
            }
            if j < b.len() {
                match b[j] {
                    b'A' | b'J' | b'H' | b'f' => return true,
                    b'K' if &b[i + 2..j] == b"2" => return true,
                    _ => {}
                }
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    false
}

pub fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// Path of the user glossary: one never-translate term per line.
/// `~/.t-translate/no_translate.txt` (same dir as the cache).
pub fn glossary_path() -> Option<std::path::PathBuf> {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()
        .map(|h| std::path::PathBuf::from(h).join(".t-translate").join("no_translate.txt"))
}

/// Load the user glossary, longest terms first so they win on overlap.
/// Missing file -> empty vec.
pub fn load_glossary() -> Vec<String> {
    let mut v: Vec<String> = glossary_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| {
            s.lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| b.len().cmp(&a.len()));
    v.dedup();
    v
}

fn push_term(out: &mut String, terms: &mut Vec<String>, term: &str) {
    out.push_str(&format!("__T{}__", terms.len()));
    terms.push(term.to_string());
}

fn boundary_before(b: &[u8], i: usize) -> bool {
    i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_')
}

fn boundary_after(b: &[u8], j: usize) -> bool {
    j >= b.len() || !(b[j].is_ascii_alphanumeric() || b[j] == b'_')
}

/// Mask terms that must survive translation untouched and return
/// `(masked_text, terms)`; restore with [`restore_terms`].
///
/// Protected, in priority order:
/// 1. `` `code` `` spans (single line)
/// 2. user glossary terms (longest first)
/// 3. `--long-flag` options
/// 4. `-f` short flags
/// 5. alphanumeric tokens mixing letters and digits (`win10`, `pshell5`, `utf-8`)
pub fn protect_terms(s: &str, glossary: &[String]) -> (String, Vec<String>) {
    let mut out = String::with_capacity(s.len());
    let mut terms: Vec<String> = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        // 1. `code` span
        if b[i] == b'`' {
            let mut j = i + 1;
            while j < b.len() && b[j] != b'`' && b[j] != b'\n' {
                j += 1;
            }
            if j < b.len() && b[j] == b'`' && j > i + 1 {
                push_term(&mut out, &mut terms, &s[i..=j]);
                i = j + 1;
                continue;
            }
        }
        // 2. user glossary
        let mut gmatched: Option<&str> = None;
        for g in glossary {
            if s[i..].starts_with(g.as_str())
                && boundary_before(b, i)
                && boundary_after(b, i + g.len())
            {
                gmatched = Some(g);
                break;
            }
        }
        if let Some(g) = gmatched {
            push_term(&mut out, &mut terms, g);
            i += g.len();
            continue;
        }
        // 3. --long-flag
        if s[i..].starts_with("--") && boundary_before(b, i) {
            let mut j = i + 2;
            if j < b.len() && b[j].is_ascii_alphanumeric() {
                j += 1;
                while j < b.len()
                    && (b[j].is_ascii_alphanumeric() || b[j] == b'-' || b[j] == b'_')
                {
                    j += 1;
                }
                push_term(&mut out, &mut terms, &s[i..j]);
                i = j;
                continue;
            }
        }
        // 4. -f short flag
        if b[i] == b'-'
            && boundary_before(b, i)
            && i + 1 < b.len()
            && b[i + 1].is_ascii_alphabetic()
        {
            push_term(&mut out, &mut terms, &s[i..i + 2]);
            i += 2;
            continue;
        }
        // 5. letter+digit token (win10, pshell5, ...)
        if b[i].is_ascii_alphanumeric() {
            let mut j = i;
            while j < b.len()
                && (b[j].is_ascii_alphanumeric() || b[j] == b'.' || b[j] == b'-' || b[j] == b'_')
            {
                j += 1;
            }
            while j > i && (b[j - 1] == b'.' || b[j - 1] == b'-' || b[j - 1] == b'_') {
                j -= 1;
            }
            let tok = &s[i..j];
            if tok.bytes().any(|c| c.is_ascii_alphabetic())
                && tok.bytes().any(|c| c.is_ascii_digit())
            {
                push_term(&mut out, &mut terms, tok);
                i = j;
                continue;
            }
        }
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    (out, terms)
}

/// Put protected terms back into translated text.
/// Placeholders the backend mangled are left as-is.
pub fn restore_terms(s: &str, terms: &[String]) -> String {
    let mut out = s.to_string();
    for (i, t) in terms.iter().enumerate() {
        out = out.replace(&format!("__T{i}__"), t);
    }
    out
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
    fn splits_columns() {
        let (c, t) = split_desc("  -h, --help      Show help for a command");
        assert_eq!(c, 18);
        assert_eq!(t, "Show help for a command");
        let (c, t) = split_desc("  Plain sentence here");
        assert_eq!(c, 2);
        assert_eq!(t, "Plain sentence here");
    }

    #[test]
    fn filters() {
        assert!(should_translate("  -h, --help      Show help for a command"));
        assert!(should_translate("Continue? (y/N)"));
        assert!(!should_translate("/usr/local/bin/git"));
        assert!(!should_translate("a1b2c3d4e5f6"));
        assert!(!should_translate("已经是中文的一行"));
    }

    #[test]
    fn redraw_detect() {
        assert!(has_redraw(b"abc\x1b[2A\x1b[2Kdef"));
        assert!(has_redraw(b"\x1b[2K"));
        assert!(has_redraw(b"\x1b[H"));
        assert!(!has_redraw(b"\x1b[31mred\x1b[0m\x1b[K plain"));
        assert!(!has_redraw(b"\x1b[?25l"));
    }

    #[test]
    fn prompt_and_cr() {
        assert!(looks_like_prompt("Choose install location: "));
        assert!(looks_like_prompt("Continue? (y/N)"));
        assert!(!looks_like_prompt("The quick brown fox jumps"));
        assert!(has_lone_cr(b"10%\r20%"));
        assert!(!has_lone_cr(b"line\r\n"));
        assert!(!has_lone_cr(b"line\r"));
    }

    #[test]
    fn protects_mixed_terms() {
        let g = vec!["PowerShell".to_string()];
        let (m, t) = protect_terms("Install PowerShell on win10 with `choco install` --force", &g);
        assert_eq!(t, vec!["PowerShell", "win10", "`choco install`", "--force"]);
        assert_eq!(m, "Install __T0__ on __T1__ with __T2__ __T3__");
        assert_eq!(
            restore_terms(&m, &t),
            "Install PowerShell on win10 with `choco install` --force"
        );
    }

    #[test]
    fn protects_short_flags_and_versions() {
        let (m, t) = protect_terms("use -f to force, needs pshell5 and v1.2.3", &[]);
        assert_eq!(t, vec!["-f", "pshell5", "v1.2.3"]);
        assert_eq!(m, "use __T0__ to force, needs __T1__ and __T2__");
        assert_eq!(restore_terms(&m, &t), "use -f to force, needs pshell5 and v1.2.3");
    }

    #[test]
    fn protect_leaves_plain_text_alone() {
        let (m, t) = protect_terms("version 2 of the tool", &[]);
        assert!(t.is_empty());
        assert_eq!(m, "version 2 of the tool");
        // unterminated backtick is not a code span
        let (m, _) = protect_terms("say `hi", &[]);
        assert_eq!(m, "say `hi");
    }

    #[test]
    fn glossary_needs_boundaries() {
        // "win10x" must not match glossary term "win10"; the whole token is protected instead
        let g = vec!["win10".to_string()];
        let (m, t) = protect_terms("win10x is not win10", &g);
        assert_eq!(t, vec!["win10x", "win10"]);
        assert_eq!(m, "__T0__ is not __T1__");
    }
}
