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
}
