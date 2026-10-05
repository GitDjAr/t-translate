//! Run a command inside a PTY and pump its output through the translator.
//!
//! Streaming rules (why `tail -f`, `docker logs -f` etc. keep translating):
//!  * complete lines are batched, but a batch is flushed after MAX_WAIT even if output
//!    never goes idle (continuous streams used to wait for 30 lines);
//!  * an unfinished line that looks like a prompt is translated inline after IDLE;
//!  * any other unfinished line is passed through raw after PARTIAL_MAX so nothing hangs;
//!  * progress bars (lone '\r') and full-screen TUIs (alternate screen) pass through untouched.

use crate::render::{flush_lines, prompt_inline, write_out};
use crate::text::{contains, has_lone_cr, looks_like_prompt, strip_ansi};
use crate::translator::Translator;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const BATCH_LINES: usize = 30;
const IDLE: Duration = Duration::from_millis(200);
const MAX_WAIT: Duration = Duration::from_millis(500);
const PARTIAL_MAX: Duration = Duration::from_millis(800);

enum Msg {
    Data(Vec<u8>),
    Exit(u32),
}

struct Pump {
    tr: Translator,
    lines: Vec<Vec<u8>>,
    buf: Vec<u8>,
    pending_since: Option<Instant>,
    partial_since: Option<Instant>,
    passthrough: bool, // inside alternate screen (TUI)
    skip: bool,        // rest of this line goes through raw (prompt echo, partial, progress)
}

impl Pump {
    fn new(tr: Translator) -> Self {
        Pump {
            tr,
            lines: Vec::new(),
            buf: Vec::new(),
            pending_since: None,
            partial_since: None,
            passthrough: false,
            skip: false,
        }
    }

    fn next_timeout(&self, exiting: bool) -> Duration {
        if exiting {
            return Duration::from_millis(150);
        }
        let now = Instant::now();
        let mut t = IDLE;
        if let Some(s) = self.pending_since {
            t = t.min(MAX_WAIT.saturating_sub(now - s));
        }
        if let Some(s) = self.partial_since {
            t = t.min(PARTIAL_MAX.saturating_sub(now - s));
        }
        t.max(Duration::from_millis(10))
    }

    fn flush_lines(&mut self) {
        flush_lines(&mut self.tr, &mut self.lines);
        self.pending_since = None;
    }

    fn write_partial_raw(&mut self) {
        if !self.buf.is_empty() {
            write_out(&self.buf);
            self.buf.clear();
        }
        self.partial_since = None;
    }

    fn feed(&mut self, chunk: &[u8]) {
        let enter = contains(chunk, b"\x1b[?1049h")
            || contains(chunk, b"\x1b[?1047h")
            || contains(chunk, b"\x1b[?47h");
        let leave = contains(chunk, b"\x1b[?1049l")
            || contains(chunk, b"\x1b[?1047l")
            || contains(chunk, b"\x1b[?47l");
        if enter && !self.passthrough {
            self.flush_lines();
            self.write_partial_raw();
            self.passthrough = true;
        }
        if self.passthrough {
            write_out(chunk);
            if leave {
                self.passthrough = false;
            }
            return;
        }

        let mut rest = chunk;
        while !rest.is_empty() {
            let (piece, tail) = match rest.iter().position(|&b| b == b'\n') {
                Some(p) => (&rest[..=p], &rest[p + 1..]),
                None => (rest, &rest[rest.len()..]),
            };
            rest = tail;
            let has_nl = piece.ends_with(b"\n");

            if self.skip {
                write_out(piece);
                if has_nl {
                    self.skip = false;
                }
                continue;
            }
            let lone_cr = has_lone_cr(piece) || (self.buf.ends_with(b"\r") && !piece.starts_with(b"\n"));
            if lone_cr {
                self.flush_lines();
                self.write_partial_raw();
                write_out(piece);
                if !has_nl {
                    self.skip = true;
                }
                continue;
            }
            if self.buf.is_empty() && !has_nl {
                self.partial_since = Some(Instant::now());
            }
            self.buf.extend_from_slice(piece);
            if has_nl {
                let line = std::mem::take(&mut self.buf);
                self.partial_since = None;
                if self.lines.is_empty() {
                    self.pending_since = Some(Instant::now());
                }
                self.lines.push(line);
            }
        }
    }

    /// No data for IDLE: flush pending lines; translate a prompt-looking partial line.
    fn on_idle(&mut self, alive: bool) {
        if self.passthrough {
            return;
        }
        self.flush_lines();
        if alive && !self.skip && !self.buf.is_empty() && looks_like_prompt(&strip_ansi(&self.buf)) {
            prompt_inline(&mut self.tr, &mut self.buf);
            self.partial_since = None;
            self.skip = true;
        }
    }

    /// Called after every event so continuous streams still flush on time.
    fn flush_if_due(&mut self) {
        if self.passthrough {
            return;
        }
        let now = Instant::now();
        if self.lines.len() >= BATCH_LINES
            || self.pending_since.map_or(false, |s| now - s >= MAX_WAIT)
        {
            self.flush_lines();
        }
        if !self.skip
            && !self.buf.is_empty()
            && self.partial_since.map_or(false, |s| now - s >= PARTIAL_MAX)
        {
            self.flush_lines();
            self.write_partial_raw();
            self.skip = true;
        }
    }

    fn finish(&mut self) {
        self.flush_lines();
        self.write_partial_raw();
        self.tr.save();
    }
}

pub fn run(args: &[String], lang: String, use_cache: bool) -> i32 {
    let (cols, rows) = crossterm::terminal::size().unwrap_or((120, 30));
    let pair = match native_pty_system().openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 }) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("t: cannot open pty: {e}");
            return 1;
        }
    };

    // On Windows, resolve .cmd/.bat shims (npm-installed tools like codex) via cmd.exe.
    let mut cmd = if cfg!(windows) {
        let mut c = CommandBuilder::new("cmd");
        c.arg("/c");
        c.args(args);
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
            return 127;
        }
    };
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("pty reader");
    let mut writer = pair.master.take_writer().expect("pty writer");
    let raw_ok = crossterm::terminal::enable_raw_mode().is_ok();

    let (tx, rx) = mpsc::channel::<Msg>();

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

    let mut pump = Pump::new(Translator::new(lang, use_cache));
    let mut exit_code: Option<u32> = None;
    loop {
        match rx.recv_timeout(pump.next_timeout(exit_code.is_some())) {
            Ok(Msg::Exit(c)) => exit_code = Some(c),
            Ok(Msg::Data(chunk)) => pump.feed(&chunk),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                pump.on_idle(exit_code.is_none());
                if exit_code.is_some() {
                    break; // child exited and output drained
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        pump.flush_if_due();
    }

    pump.finish();
    if raw_ok {
        let _ = crossterm::terminal::disable_raw_mode();
    }
    drop(pair.master);
    exit_code.unwrap_or(0) as i32
}
