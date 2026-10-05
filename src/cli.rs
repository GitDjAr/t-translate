//! Argument parsing.

pub enum Action {
    Version,
    Update,
    Alias(Option<String>),
    Cache { clear: bool },
    Help { code: i32 },
    Run { args: Vec<String>, lang: String, use_cache: bool },
}

pub fn usage() {
    eprintln!(
        "t - bilingual command output\n\n\
         usage: t [--lang <code>] [--no-cache] <command> [args...]\n\
         \x20      t --update | --version | --alias [name] | --cache [clear]\n\
         \n\
         example: t git -h\n\
         env: T_LANG, T_BACKEND(auto|google|bing|openai), T_API_BASE, T_API_KEY, T_MODEL"
    );
}

pub fn parse(mut args: Vec<String>) -> Action {
    match args.first().map(|s| s.as_str()) {
        Some("--version") | Some("-V") => return Action::Version,
        Some("--update") => return Action::Update,
        Some("--alias") => return Action::Alias(args.get(1).cloned()),
        Some("--cache") => return Action::Cache { clear: args.get(1).map(|s| s == "clear").unwrap_or(false) },
        _ => {}
    }
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
    if args.is_empty() {
        return Action::Help { code: 2 };
    }
    if args[0] == "-h" || args[0] == "--help" {
        return Action::Help { code: 0 };
    }
    Action::Run { args, lang, use_cache }
}
