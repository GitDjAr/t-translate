//! Run a command inside a PTY and pump its output through the translator.
//!
//! Two modes:
//!  * line mode (default): complete lines are batched and printed followed by their
//!    translation. Streaming-safe (MAX_WAIT flush), prompts get an inline "(译文)",
//!    progress bars (lone '\r') pass through raw.
//!  * screen mode (see screen.rs): entered when the program repaints itself (cursor-up /
//!    erase codes) or switches to the alternate screen. Output is passed through untouched,
//!    translations go to a footer at the bottom, translated on a worker thread.

use crate::render::{flush_lines, prompt_inline, write_out};
use crate::screen::{footer_rows, ScreenState};
use crate::text::{contains, has_lone_cr, has_redraw, looks_like_prompt, strip_ansi};
use crate::translator::Translator;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

const BATCH_LINES: usize = 30;
const IDLE: Duration = Duration::from_millis(200);
const MAX_WAIT: Duration = Duration::from_millis(500);
const PARTIAL_MAX: Duration = Duration::from_millis(800);
const HL: &str = "\x1b[96m";
const DIM: &str = "\x1b[90m";
const RESET: &str = "\x1b[0m";

enum Msg {
    Data(Vec<u8>),
    Exit(u32),
    Translated(Vec<(String, String)>),
}

struct Pump {
    tr: Arc<Mutex<Translator>>,
    master: Box<dyn MasterPty + Send>,
    job_tx: mpsc::Sender<Vec<String>>,
    lines: Vec<Vec<u8>>,
    buf: Vec<u8>,
    pending_since: Option<Instant>,
    partial_since: Option<Instant>,
    passthrough: bool, // fallback when screen mode isn't possible (no tty / tiny window)
    skip: bool,        // rest of this line goes through raw (prompt echo, partial, progress)
    screen: Option<ScreenState>,
}

fn pty_size(rows: u16, cols: u16) -> PtySize {
    PtySize { rows, cols, pixel_width: 0, pixel_height: 0 }
}

impl Pump {
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
        if let Some(d) = self.screen.as_ref().and_then(|s| s.stable_in()) {
            t = t.min(d);
        }
        t.max(Duration::from_millis(10))
    }

    fn flush_lines(&mut self) {
        {
            let mut tr = self.tr.lock().unwrap();
            flush_lines(&mut tr, &mut self.lines);
        }
        self.pending_since = None;
    }

    fn write_partial_raw(&mut self) {
        if !self.buf.is_empty() {
            write_out(&self.buf);
            self.buf.clear();
        }
        self.partial_since = None;
    }

    // ------------------------------------------------------------ screen mode

    fn enter_screen(&mut self, by_alt: bool) -> bool {
        let Ok((cols, rows)) = crossterm::terminal::size() else { return false };
        let footer = footer_rows(rows);
        if footer == 0 {
            return false;
        }
        let st = ScreenState::new(cols, rows, footer, by_alt);
        let _ = self.master.resize(pty_size(st.child_rows(), cols));
        self.screen = Some(st);
        true
    }

    fn leave_screen(&mut self) {
        if let Some(s) = self.screen.take() {
            let mut o = String::from("\x1b7");
            for i in 0..s.footer {
                o.push_str(&format!("\x1b[{};1H\x1b[2K", s.child_rows() + 1 + i));
            }
            o.push_str("\x1b[r\x1b8");
            write_out(o.as_bytes());
            let _ = self.master.resize(pty_size(s.total_rows, s.cols));
        }
    }

    fn draw_footer(&mut self) {
        let Some(s) = self.screen.as_mut() else { return };
        let lines = s.footer_lines();
        let child_rows = s.child_rows();
        let mut o = String::from("\x1b7"); // save cursor (restored below)
        if s.region_dirty {
            o.push_str(&format!("\x1b[1;{}r", child_rows)); // keep scrolling out of the footer
            s.region_dirty = false;
        }
        for (i, l) in lines.iter().enumerate() {
            let colour = if i == 0 { DIM } else { HL };
            o.push_str(&format!("\x1b[{};1H\x1b[2K{}{}{}", child_rows + 1 + i as u16, colour, l, RESET));
        }
        o.push_str("\x1b8");
        write_out(o.as_bytes());
    }

    /// After every event: follow window resizes, translate a stable screen, redraw footer.
    fn screen_tick(&mut self) {
        let Some(s) = self.screen.as_mut() else { return };
        if let Ok((cols, rows)) = crossterm::terminal::size() {
            if (cols, rows) != (s.cols, s.total_rows) {
                let footer = footer_rows(rows);
                if footer == 0 {
                    self.leave_screen();
                    self.passthrough = true;
                    return;
                }
                s.resize(cols, rows, footer);
                let _ = self.master.resize(pty_size(rows - footer, cols));
            }
        }
        let Some(s) = self.screen.as_mut() else { return };
        if s.stable_in().map_or(false, |d| d.is_zero()) {
            let jobs = s.take_jobs();
            if !jobs.is_empty() {
                let _ = self.job_tx.send(jobs);
            }
            self.draw_footer();
        }
    }

    fn on_translated(&mut self, pairs: Vec<(String, String)>) {
        if let Some(s) = self.screen.as_mut() {
            s.add_results(pairs);
            self.draw_footer();
        }
    }

    // ------------------------------------------------------------ data path

    fn feed(&mut self, chunk: &[u8]) {
        let alt_on = contains(chunk, b"\x1b[?1049h")
            || contains(chunk, b"\x1b[?1047h")
            || contains(chunk, b"\x1b[?47h");
        let alt_off = contains(chunk, b"\x1b[?1049l")
            || contains(chunk, b"\x1b[?1047l")
            || contains(chunk, b"\x1b[?47l");

        if self.screen.is_none() && !self.passthrough && (alt_on || has_redraw(chunk)) {
            self.flush_lines();
            self.write_partial_raw();
            self.skip = false;
            if !self.enter_screen(alt_on) {
                self.passthrough = true;
            }
        }
        if self.screen.is_some() {
            write_out(chunk);
            let by_alt = self.screen.as_ref().map_or(false, |s| s.by_alt);
            if let Some(s) = self.screen.as_mut() {
                s.feed(chunk);
            }
            if alt_off && by_alt {
                self.leave_screen();
            }
            return;
        }
        if self.passthrough {
            write_out(chunk);
            if alt_off {
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
        if self.passthrough || self.screen.is_some() {
            return;
        }
        self.flush_lines();
        if alive && !self.skip && !self.buf.is_empty() && looks_like_prompt(&strip_ansi(&self.buf)) {
            {
                let mut tr = self.tr.lock().unwrap();
                prompt_inline(&mut tr, &mut self.buf);
            }
            self.partial_since = None;
            self.skip = true;
        }
    }

    /// Called after every event so continuous streams still flush on time.
    fn flush_if_due(&mut self) {
        if self.passthrough || self.screen.is_some() {
            return;
        }
        let now = Instant::now();
        if self.lines.len() >= BATCH_LINES || self.pending_since.map_or(false, |s| now - s >= MAX_WAIT) {
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
        self.leave_screen();
        self.flush_lines();
        self.write_partial_raw();
        self.tr.lock().unwrap().save();
    }
}

pub fn run(args: &[String], lang: String, use_cache: bool) -> i32 {
    let (cols, rows) = crossterm::terminal::size().unwrap_or((120, 30));
    let pair = match native_pty_system().openpty(pty_size(rows, cols)) {
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

    // translation worker for screen mode: the UI never waits on the network
    let tr = Arc::new(Mutex::new(Translator::new(lang, use_cache)));
    let (job_tx, job_rx) = mpsc::channel::<Vec<String>>();
    {
        let tr = Arc::clone(&tr);
        let tx = tx.clone();
        std::thread::spawn(move || {
            for job in job_rx {
                let res = tr.lock().unwrap().translate_batch(&job);
                let pairs: Vec<(String, String)> = job
                    .into_iter()
                    .zip(res)
                    .filter_map(|(k, v)| v.map(|v| (k, v)))
                    .collect();
                if tx.send(Msg::Translated(pairs)).is_err() {
                    break;
                }
            }
        });
    }
    drop(tx);

    let mut pump = Pump {
        tr,
        master: pair.master,
        job_tx,
        lines: Vec::new(),
        buf: Vec::new(),
        pending_since: None,
        partial_since: None,
        passthrough: false,
        skip: false,
        screen: None,
    };

    let mut exit_code: Option<u32> = None;
    loop {
        match rx.recv_timeout(pump.next_timeout(exit_code.is_some())) {
            Ok(Msg::Exit(c)) => exit_code = Some(c),
            Ok(Msg::Data(chunk)) => pump.feed(&chunk),
            Ok(Msg::Translated(p)) => pump.on_translated(p),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                pump.on_idle(exit_code.is_none());
                if exit_code.is_some() {
                    break; // child exited and output drained
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        pump.flush_if_due();
        pump.screen_tick();
    }

    pump.finish();
    if raw_ok {
        let _ = crossterm::terminal::disable_raw_mode();
    }
    exit_code.unwrap_or(0) as i32
}
