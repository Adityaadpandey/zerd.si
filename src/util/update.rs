//! Keeping zerd up to date. Once a day, on an interactive launch, zerd asks GitHub which release is the latest
//! (one request, two seconds at most) and, if it's newer than this binary, offers to install it right there:
//!
//!   zerd 0.1.3 is out (you have 0.1.2). Update now? [Y/n]
//!
//! Yes runs the same install script the README gives, into the folder this binary lives in, then restarts zerd on
//! the new version. `zerd update` does it on demand, and ZERD_NO_UPDATE_CHECK=1 turns the launch check off.
use crate::ui::style::{bold_paint, paint, ACCENT, CORAL, MUTED};
use std::io::Write;

const LATEST: &str = "https://github.com/Adityaadpandey/zerd.si/releases/latest";
const DAY: u64 = 24 * 60 * 60;

/// "v0.1.10" -> (0, 1, 10), so 0.1.10 sorts after 0.1.9.
fn parse(v: &str) -> Option<(u64, u64, u64)> {
    let mut n = v.trim().trim_start_matches('v').splitn(3, '.').map(|p| p.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok());
    Some((n.next()??, n.next()??, n.next()??))
}

fn newer(latest: &str, current: &str) -> bool {
    matches!((parse(latest), parse(current)), (Some(l), Some(c)) if l > c)
}

/// The latest release tag, read off where /releases/latest redirects (no API token, no rate limit worth worrying about).
async fn latest() -> Option<String> {
    let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).timeout(std::time::Duration::from_secs(2)).build().ok()?;
    let r = client.get(LATEST).header("User-Agent", "zerd").send().await.ok()?;
    let to = r.headers().get("location")?.to_str().ok()?;
    Some(to.rsplit('/').next()?.to_string())
}

fn stamp() -> std::path::PathBuf {
    crate::config::loader::get_data_dir().join("update-check")
}

/// A dev build (`cargo run`, target/debug) isn't something to overwrite with a release.
fn installed_binary() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    (!exe.components().any(|c| c.as_os_str() == "target")).then_some(exe)
}

/// Runs the install script into this binary's folder. True when it worked.
fn install(exe: &std::path::Path) -> bool {
    let dir = exe.parent().unwrap_or(exe);
    let mut cmd = if cfg!(windows) {
        // Windows won't overwrite a running .exe, but it will rename one
        let _ = std::fs::rename(exe, exe.with_extension("exe.old"));
        let mut c = std::process::Command::new("powershell");
        c.args(["-NoProfile", "-Command", "irm https://www.zerd.si/install.ps1 | iex"]);
        c
    } else {
        let mut c = std::process::Command::new("sh");
        c.args(["-c", "curl -fsSL https://www.zerd.si/install.sh | sh"]);
        c
    };
    let ok = cmd.env("ZERD_BIN", dir).status().map_or(false, |s| s.success());
    if !ok && cfg!(windows) {
        let _ = std::fs::rename(exe.with_extension("exe.old"), exe);
    }
    ok
}

/// Starts the freshly installed zerd with the same arguments, in place of this one.
fn restart(exe: &std::path::Path) -> ! {
    let args: Vec<String> = std::env::args().skip(1).filter(|a| a != "update").collect();
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let e = std::process::Command::new(exe).args(&args).exec();
        eprintln!("{}", paint(&format!("updated, but couldn't restart ({e}); run zerd again"), MUTED));
        std::process::exit(0);
    }
    #[cfg(not(unix))]
    std::process::exit(std::process::Command::new(exe).args(&args).status().ok().and_then(|s| s.code()).unwrap_or(0));
}

/// The launch check: at most once a day, and only asks when there is something newer.
pub async fn offer_on_launch() {
    if std::env::var_os("ZERD_NO_UPDATE_CHECK").is_some() {
        return;
    }
    let Some(exe) = installed_binary() else { return };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let last = std::fs::read_to_string(stamp()).ok().and_then(|t| t.trim().parse::<u64>().ok()).unwrap_or(0);
    if now.saturating_sub(last) < DAY {
        return;
    }
    let _ = std::fs::create_dir_all(stamp().parent().unwrap());
    let _ = std::fs::write(stamp(), now.to_string());
    let Some(tag) = latest().await else { return };
    let current = env!("CARGO_PKG_VERSION");
    if !newer(&tag, current) {
        return;
    }
    eprint!("{} {} ", bold_paint(&format!("zerd {} is out", tag.trim_start_matches('v')), ACCENT), paint(&format!("(you have {current}). Update now? [Y/n]"), MUTED));
    let _ = std::io::stderr().flush();
    let mut a = String::new();
    let _ = std::io::stdin().read_line(&mut a);
    if matches!(a.trim().to_lowercase().as_str(), "" | "y" | "yes") {
        if install(&exe) {
            restart(&exe);
        }
        eprintln!("{}", paint("The update didn't go through; carrying on with this version. zerd update tries again.", CORAL));
    } else {
        eprintln!("{}", paint("Run zerd update whenever you're ready.", MUTED));
    }
}

/// `zerd update`: check now and install if there's something newer.
pub async fn main() -> i32 {
    let current = env!("CARGO_PKG_VERSION");
    let Some(tag) = latest().await else {
        eprintln!("couldn't reach GitHub to check for a new release");
        return 1;
    };
    if !newer(&tag, current) {
        println!("zerd {current} is the latest.");
        return 0;
    }
    let Some(exe) = installed_binary() else {
        eprintln!("this is a development build; update it with git pull and cargo build");
        return 1;
    };
    println!("updating zerd {current} -> {}", tag.trim_start_matches('v'));
    if install(&exe) { 0 } else { 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_as_numbers() {
        assert!(newer("v0.1.10", "0.1.9"));
        assert!(newer("v0.2.0", "0.1.2"));
        assert!(!newer("v0.1.2", "0.1.2"));
        assert!(!newer("v0.1.1", "0.1.2"));
        assert!(!newer("nightly", "0.1.2")); // a tag that isn't a version never triggers an update
        assert_eq!(parse("v1.2.3-beta"), Some((1, 2, 3)));
    }
}
