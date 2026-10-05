//! `t --update`: fetch the latest GitHub release and replace the running binary.

use serde_json::Value;
use std::io::Read;
use std::time::Duration;

pub const REPO: &str = "GitDjAr/t-translate";

fn asset_name() -> Option<&'static str> {
    if cfg!(all(windows, target_arch = "x86_64")) {
        Some("t-windows-x64.exe")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("t-linux-x64")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("t-macos-arm64")
    } else {
        None
    }
}

pub fn self_update() -> Result<(), String> {
    let name = asset_name().ok_or("no prebuilt binary for this platform")?;
    let api = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let v: Value = ureq::get(&api)
        .set("User-Agent", "t-translate")
        .timeout(Duration::from_secs(15))
        .call()
        .map_err(|e| format!("check failed: {e}"))?
        .into_json()
        .map_err(|e| e.to_string())?;
    let tag = v["tag_name"].as_str().ok_or("no release found")?;
    let latest = tag.trim_start_matches('v');
    let current = env!("CARGO_PKG_VERSION");
    if latest == current {
        println!("t {current} is already the latest version");
        return Ok(());
    }
    let url = v["assets"]
        .as_array()
        .and_then(|a| a.iter().find(|x| x["name"].as_str() == Some(name)))
        .and_then(|x| x["browser_download_url"].as_str())
        .ok_or_else(|| format!("release {tag} has no asset {name}"))?;
    println!("updating {current} -> {latest} ...");
    let mut buf = Vec::new();
    ureq::get(url)
        .set("User-Agent", "t-translate")
        .timeout(Duration::from_secs(180))
        .call()
        .map_err(|e| format!("download failed: {e}"))?
        .into_reader()
        .read_to_end(&mut buf)
        .map_err(|e| e.to_string())?;
    if buf.len() < 100_000 {
        return Err("downloaded file looks wrong".into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let new = exe.with_extension("new");
    std::fs::write(&new, &buf).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755));
    }
    if cfg!(windows) {
        // a running exe can't be overwritten on Windows, but it can be renamed
        let old = exe.with_extension("old");
        let _ = std::fs::remove_file(&old);
        std::fs::rename(&exe, &old).map_err(|e| e.to_string())?;
        if let Err(e) = std::fs::rename(&new, &exe) {
            let _ = std::fs::rename(&old, &exe); // roll back
            return Err(e.to_string());
        }
    } else {
        std::fs::rename(&new, &exe).map_err(|e| e.to_string())?;
    }
    println!("updated to {latest}");
    Ok(())
}
