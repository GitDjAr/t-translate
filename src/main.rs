//! t — run any command through a PTY and print its output bilingual.
//!
//!   t git -h
//!   t --lang ja docker logs -f web
//!
//! Layout: cli (args) · runner (PTY + streaming pump) · render (output) ·
//! translator/ (cache + google/bing/openai) · text (heuristics) · update · alias.

mod alias;
mod cli;
mod render;
mod runner;
mod screen;
mod text;
mod translator;
mod update;

use cli::Action;

fn main() {
    // Invoked as `tt` (e.g. via `t --alias tt`) -> replace mode: the
    // translation takes the original's place instead of being appended.
    // Note: use argv[0], not current_exe(), because the latter resolves symlinks.
    let invoked_as_tt = std::env::args()
        .next()
        .map(|a| {
            std::path::Path::new(&a).file_stem().and_then(|s| s.to_str()) == Some("tt")
        })
        .unwrap_or(false);
    let code = match cli::parse(std::env::args().skip(1).collect()) {
        Action::Version => {
            println!("t {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Action::Update => report("update", update::self_update()),
        Action::Alias(name) => report("alias", alias::make_alias(name)),
        Action::Cache { clear } => {
            let mut c = translator::Cache::new(true);
            if clear {
                println!("{}", if c.clear() { "cache cleared" } else { "nothing to clear" });
            } else {
                match c.path() {
                    Some(p) => println!("cache: {} ({} entries)", p.display(), c.len()),
                    None => println!("cache: unavailable (no home dir)"),
                }
            }
            0
        }
        Action::Help { code } => {
            cli::usage();
            code
        }
        Action::Run { args, lang, use_cache, replace } => {
            runner::run(&args, lang, use_cache, replace || invoked_as_tt)
        }
    };
    std::process::exit(code);
}

fn report(what: &str, r: Result<(), String>) -> i32 {
    match r {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("t: {what} error: {e}");
            1
        }
    }
}
