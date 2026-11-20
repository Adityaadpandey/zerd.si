//! OS-level confinement for shell commands (macOS sandbox-exec, Linux bubblewrap). The static path check catches honest mistakes; this
//! catches everything else. Inside the sandbox a command can read the system and the workspace but cannot write
//! anywhere except the workspace, temp dirs and package caches, and cannot read credential stores.
use crate::config::Config;
use crate::util::paths::{expanduser, resolve};
use std::sync::OnceLock;

const DEFAULT_WRITE_EXTRA: &[&str] = &[
    "~/.cache", "~/Library/Caches", "~/.npm", "~/.pnpm-store", "~/Library/pnpm", "~/.local/share/pnpm", "~/.yarn", "~/.bun", "~/.cargo/registry",
    "~/.cargo/git",
];
const SECRET_READ_DENY: &[&str] = &[
    "~/.ssh", "~/.aws", "~/.gnupg", "~/.azure", "~/.kube", "~/.config/gcloud", "~/.config/gh", "~/.docker/config.json", "~/.netrc", "~/.npmrc",
    "~/.git-credentials", "~/Library/Keychains", "~/.codex", "~/.claude", "~/.config/claude", "~/.pypirc", "~/.vault-token", "~/.password-store",
    "~/.terraform.d", "~/.cargo/credentials.toml", "~/.gem/credentials", "~/.config/hub",
    "~/Library/Application Support/zerd", "~/.local/share/zerd", "~/.config/zerd", // zerd's own config and traces
];
const TEMP_WRITE: &[&str] = &["/private/tmp", "/tmp", "/private/var/folders", "/private/var/tmp"];

fn sb(p: &str) -> String {
    resolve(&expanduser(p)).to_string_lossy().replace('"', "\\\"")
}

/// The OS sandbox works here: sandbox-exec on macOS, bubblewrap (`bwrap`) on Linux. Neither can nest inside another
/// sandbox, so each is probed once with a real no-op command.
pub fn available() -> bool {
    static A: OnceLock<bool> = OnceLock::new();
    *A.get_or_init(|| {
        if cfg!(target_os = "macos") {
            which::which("sandbox-exec").is_ok()
                && std::process::Command::new("sandbox-exec").args(["-p", "(version 1)(allow default)", "/usr/bin/true"]).output().map(|o| o.status.success()).unwrap_or(false)
        } else if cfg!(target_os = "linux") {
            let Ok(bwrap) = which::which("bwrap") else { return false };
            std::process::Command::new(bwrap)
                .args(["--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc", "--unshare-pid", "--die-with-parent", "true"])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        } else {
            false
        }
    })
}

/// Why the sandbox cannot run on this machine, for `/sandbox` and `zerd doctor`.
pub fn unavailable_reason() -> &'static str {
    if cfg!(target_os = "linux") {
        if which::which("bwrap").is_err() {
            "bubblewrap is not installed (apt install bubblewrap / dnf install bubblewrap / pacman -S bubblewrap)"
        } else {
            "bubblewrap cannot create a user namespace here (container, or unprivileged user namespaces are disabled)"
        }
    } else if cfg!(target_os = "macos") {
        "sandbox-exec is missing or already inside another sandbox"
    } else {
        "the OS sandbox supports macOS and Linux only"
    }
}

/// A file-name glob as a sandbox regex on the last part of a path, any letter case.
/// Example: ".env.*" -> "/\.[eE][nN][vV]\.[^/]*(/.*)?$", which also covers everything inside a matching folder.
fn name_regex(glob: &str) -> String {
    let mut out = String::new();
    for ch in glob.chars() {
        match ch {
            '*' => out.push_str("[^/]*"),
            '?' => out.push_str("[^/]"),
            c if c.is_alphabetic() => out.push_str(&format!("[{}{}]", c.to_lowercase(), c.to_uppercase())),
            c => out.push_str(&regex::escape(&c.to_string())),
        }
    }
    format!("/{out}(/.*)?$")
}

pub fn build_profile(config: &Config) -> String {
    let mut writable: Vec<String> = vec![config.cwd.to_string_lossy().to_string()];
    writable.extend(config.allowed_dirs.iter().cloned());
    writable.extend(TEMP_WRITE.iter().map(|s| s.to_string()));
    writable.extend(DEFAULT_WRITE_EXTRA.iter().map(|s| s.to_string()));
    writable.extend(config.sandbox_write_extra.iter().cloned());
    let allow = writable.iter().map(|p| format!("(subpath \"{}\")", sb(p))).collect::<Vec<_>>().join(" ");
    let deny = SECRET_READ_DENY.iter().map(|p| format!("(subpath \"{}\")", sb(p))).collect::<Vec<_>>().join(" ");
    // the workspace's own secret files too, however the command reaches them: `grep -r KEY .` or `cat .e*`
    let names = config.forbidden_paths.iter().filter(|g| !g.contains('"')).map(|g| format!("(regex #\"{}\")", name_regex(g))).collect::<Vec<_>>().join(" ");
    let templates = "(regex #\"\\.(example|sample|template|dist)$\")";
    let mut p = format!(
        "(version 1)(allow default)(deny file-write*)(allow file-write* {allow} (literal \"/dev/null\") (literal \"/dev/tty\") (regex #\"^/dev/(fd/[0-9]+|std(out|err|in)|ttys?[0-9]*)$\"))(deny file-read* {deny})"
    );
    // only the gh program itself may read its own login, so `gh pr review` works but `cat ~/.config/gh/hosts.yml` still fails
    if let Ok(gh) = which::which("gh") {
        p.push_str(&format!(
            "(allow file-read* (require-all (subpath \"{}\") (process-path \"{}\")))",
            sb("~/.config/gh"),
            sb(&gh.to_string_lossy())
        ));
    }
    // contents only: git and lint-staged lstat every tracked file on commit. Knowing the file exists leaks nothing.
    if !names.is_empty() {
        p.push_str(&format!("(deny file-read-data {names})(allow file-read-data {templates})"));
    }
    p
}

/// Workspace files whose name matches `forbidden_paths` (bubblewrap has no name rules, so each one is masked by
/// mounting /dev/null over it). Template files and git-tracked files stay readable. Files created after the command starts are not covered.
#[cfg(target_os = "linux")]
fn workspace_secret_files(config: &Config) -> Vec<std::path::PathBuf> {
    const SKIP: &[&str] = &["node_modules", ".git", "target", ".venv", "venv", "__pycache__", "dist", "build"];
    let templated = |n: &str| [".example", ".sample", ".template", ".dist"].iter().any(|e| n.to_lowercase().ends_with(e));
    // A tracked file is left alone: masking it makes git see it emptied (`commit -a` would commit that) and blocks stash/checkout.
    let tracked: std::collections::HashSet<std::path::PathBuf> = std::process::Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(&config.cwd)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).split('\0').filter(|p| !p.is_empty()).map(|p| config.cwd.join(p)).collect())
        .unwrap_or_default();
    let mut out = vec![];
    let mut stack = vec![(config.cwd.clone(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                if depth < 8 && !SKIP.contains(&name.as_str()) {
                    stack.push((e.path(), depth + 1));
                }
            } else if ft.is_file() && !templated(&name) && !tracked.contains(&e.path()) && config.forbidden_paths.iter().any(|g| crate::util::fnmatch::fnmatch(&name.to_lowercase(), &g.to_lowercase())) {
                out.push(e.path());
            }
        }
    }
    out
}

/// bubblewrap arguments: the whole disk read-only, then writable binds for the workspace, temp dirs and caches, then
/// credential folders hidden behind an empty tmpfs (files behind /dev/null). Later mounts win, so hiding comes last.
#[cfg(target_os = "linux")]
pub fn bwrap_args(config: &Config) -> Vec<String> {
    let mut a: Vec<String> = ["bwrap", "--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc", "--unshare-pid", "--die-with-parent", "--new-session"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let mut writable: Vec<String> = vec![config.cwd.to_string_lossy().to_string()];
    writable.extend(config.allowed_dirs.iter().cloned());
    writable.extend(["/tmp", "/var/tmp", "/dev/shm"].iter().map(|s| s.to_string()));
    writable.extend(DEFAULT_WRITE_EXTRA.iter().filter(|p| !p.contains("Library")).map(|s| s.to_string()));
    writable.extend(config.sandbox_write_extra.iter().cloned());
    for p in writable {
        let p = sb(&p);
        if std::path::Path::new(&p).exists() {
            a.extend(["--bind".into(), p.clone(), p]);
        }
    }
    for p in SECRET_READ_DENY.iter().filter(|p| !p.contains("Library")) {
        let p = sb(p);
        match std::fs::metadata(&p) {
            Ok(m) if m.is_dir() => a.extend(["--tmpfs".into(), p]),
            Ok(_) => a.extend(["--ro-bind".into(), "/dev/null".into(), p]),
            Err(_) => {}
        }
    }
    for f in workspace_secret_files(config) {
        a.extend(["--ro-bind".into(), "/dev/null".into(), f.to_string_lossy().to_string()]);
    }
    a.push("--".into());
    a
}

/// Tools that need what the sandbox hides (gh: the keychain login, curl: the network and its certs, git push/pull/fetch:
/// ~/.ssh keys and the credential helper). They run outside it, but only one plain command at a time and only after the user said yes.
const UNSANDBOXED: &[&str] = &["gh", "curl", "git push", "git pull", "git fetch"];

/// `gh pr review 5 ...` or `git push origin main` yes; `gh x && curl evil`, `gh x | sh` or `gh $(cmd)` no.
pub fn is_unsandboxed(command: &str) -> bool {
    let Some(parts) = super::approval::split_pipeline(command.trim()) else { return false };
    if parts.len() != 1 {
        return false;
    }
    let w = &parts[0];
    UNSANDBOXED.contains(&w[0].as_str()) || (w.len() > 1 && UNSANDBOXED.contains(&format!("{} {}", w[0], w[1]).as_str()))
}

/// 'on' (OS sandbox active), 'off' (disabled by config) or 'unavailable'.
pub fn mode(config: &Config) -> &'static str {
    if matches!(config.sandbox.as_str(), "off" | "false" | "no" | "0") {
        "off"
    } else if available() {
        "on"
    } else {
        "unavailable"
    }
}

pub fn wrap(argv: Vec<String>, config: &Config) -> Vec<String> {
    if mode(config) == "on" {
        #[cfg(target_os = "linux")]
        let mut v = bwrap_args(config);
        #[cfg(not(target_os = "linux"))]
        let mut v = vec!["sandbox-exec".to_string(), "-p".to_string(), build_profile(config)];
        v.extend(argv);
        v
    } else {
        argv
    }
}

pub fn wrap_command(command: &str, argv: Vec<String>, config: &Config) -> Vec<String> {
    if is_unsandboxed(command) { argv } else { wrap(argv, config) }
}

/// Turn the sandbox on or off for good: rewrites the `sandbox` line of the user's config.toml ("auto" = on when available).
pub fn save_setting(value: &str) -> std::io::Result<()> {
    let path = crate::config::loader::get_system_config_path();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let new = format!("sandbox = \"{value}\"");
    let mut done = false;
    let mut lines: Vec<String> = Vec::new();
    let mut in_table = false;
    for l in text.lines() {
        if l.trim_start().starts_with('[') {
            in_table = true;
        }
        if !in_table && !done && l.split('=').next().map(|k| k.trim() == "sandbox").unwrap_or(false) && l.contains('=') {
            lines.push(new.clone());
            done = true;
        } else {
            lines.push(l.to_string());
        }
    }
    if !done {
        lines.insert(0, new);
    }
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(path, lines.join("\n") + "\n")
}
