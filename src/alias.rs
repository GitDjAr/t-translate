//! `t --alias [name]`: create an extra command name next to the t binary.
//! Windows: a tiny .cmd shim forwarding to t.exe (stays valid after --update).
//! Unix: a symlink. Refuses names already taken somewhere on PATH.

use std::io::Write;

pub fn make_alias(name: Option<String>) -> Result<(), String> {
    let name = match name {
        Some(n) => n,
        None => {
            print!("Enter alias name (e.g. tt): ");
            let _ = std::io::stdout().flush();
            let mut s = String::new();
            std::io::stdin().read_line(&mut s).map_err(|e| e.to_string())?;
            s.trim().to_string()
        }
    };
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("alias must be letters, digits, '-' or '_'".into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("cannot find install dir")?.to_path_buf();
    let exe_file = exe.file_name().and_then(|s| s.to_str()).unwrap_or("t").to_string();

    let exts: &[&str] = if cfg!(windows) { &["", ".exe", ".cmd", ".bat", ".ps1"] } else { &[""] };
    if let Some(paths) = std::env::var_os("PATH") {
        for p in std::env::split_paths(&paths) {
            for e in exts {
                let cand = p.join(format!("{name}{e}"));
                if cand.exists() {
                    return Err(format!("'{name}' is already taken: {}", cand.display()));
                }
            }
        }
    }

    if cfg!(windows) {
        let target = dir.join(format!("{name}.cmd"));
        std::fs::write(&target, format!("@echo off\r\n\"%~dp0{exe_file}\" %*\r\n"))
            .map_err(|e| e.to_string())?;
        println!("created {}", target.display());
    } else {
        #[cfg(unix)]
        {
            let target = dir.join(&name);
            std::os::unix::fs::symlink(&exe, &target).map_err(|e| e.to_string())?;
            println!("created {}", target.display());
        }
    }
    println!("now you can run: {name} git -h");
    Ok(())
}
