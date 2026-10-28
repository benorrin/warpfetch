//! Installed package counts.
//!
//! Most package databases can be counted straight from disk in a millisecond
//! or two. rpm, xbps and nix can only be queried through their own tools,
//! which can take seconds (`rpm -qa` takes over 3 s on a typical openSUSE
//! install), so their counts are cached and recounted in the background
//! whenever their database changes.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

use crate::read;

/// A package manager counted by running a command, one package per line.
struct Slow {
    name: &'static str,
    /// Database paths; the first that exists decides staleness by mtime.
    dbs: &'static [&'static str],
    cmd: &'static [&'static str],
}

const SLOW: &[Slow] = &[
    Slow {
        name: "rpm",
        dbs: &[
            "/usr/lib/sysimage/rpm/Packages.db",
            "/usr/lib/sysimage/rpm/rpmdb.sqlite",
            "/var/lib/rpm/Packages.db",
            "/var/lib/rpm/rpmdb.sqlite",
            "/var/lib/rpm/Packages",
        ],
        cmd: &["rpm", "-qa"],
    },
    Slow {
        name: "xbps",
        dbs: &["/var/db/xbps"],
        cmd: &["xbps-query", "-l"],
    },
    Slow {
        name: "nix",
        dbs: &["/nix/var/nix/db/db.sqlite"],
        cmd: &["nix-store", "-qR", "/run/current-system/sw"],
    },
];

/// e.g. "1834 (pacman), 12 (flatpak)". None if nothing was found.
pub fn summary() -> Option<String> {
    let mut parts = Vec::new();

    let mut stale = false;
    for s in SLOW {
        let Some(db) = s.dbs.iter().find(|p| Path::new(p).exists()) else {
            continue;
        };
        let cache = cache_file(s.name);
        stale |= match (mtime(&cache), mtime(db)) {
            (Some(c), Some(d)) => d > c,
            _ => true,
        };
        match read(&cache).and_then(|s| s.parse::<usize>().ok()) {
            Some(n) => parts.extend(part(s.name, Some(n))),
            None => parts.push(format!("counting… ({})", s.name)),
        }
    }
    if stale {
        spawn_refresh();
    }

    let home = env::var("HOME").unwrap_or_default();
    let flatpak = [
        count_dirs("/var/lib/flatpak/app"),
        count_dirs(format!("{home}/.local/share/flatpak/app")),
    ];
    parts.extend(
        [
            part("pacman", count_dirs("/var/lib/pacman/local")),
            part(
                "dpkg",
                count_lines("/var/lib/dpkg/status", |l| {
                    l == "Status: install ok installed"
                }),
            ),
            part(
                "apk",
                count_lines("/lib/apk/db/installed", |l| l.starts_with("P:")),
            ),
            part("emerge", portage()),
            part(
                "flatpak",
                flatpak.into_iter().flatten().reduce(|a, b| a + b),
            ),
            // /snap holds one directory per snap, plus `bin`.
            part("snap", count_dirs("/snap").map(|n| n.saturating_sub(1))),
        ]
        .into_iter()
        .flatten(),
    );

    (!parts.is_empty()).then(|| parts.join(", "))
}

/// "N (name)", or None when the manager is absent or has nothing installed.
fn part(name: &str, n: Option<usize>) -> Option<String> {
    n.filter(|&n| n > 0).map(|n| format!("{n} ({name})"))
}

/// Recount every slow package manager whose cache is missing or stale.
/// Runs in the background, as `warpfetch --refresh-packages`.
pub fn refresh() {
    let dir = cache_dir();
    let _ = fs::create_dir_all(&dir);
    for s in SLOW {
        let Some(db) = s.dbs.iter().find(|p| Path::new(p).exists()) else {
            continue;
        };
        let cache = cache_file(s.name);
        if matches!((mtime(&cache), mtime(db)), (Some(c), Some(d)) if c >= d) {
            continue;
        }
        let out = Command::new(s.cmd[0])
            .args(&s.cmd[1..])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output();
        // A failed count is stored as 0 (hidden) so it is not retried until
        // the database changes.
        let n = match out {
            Ok(o) if o.status.success() => o
                .stdout
                .split(|&b| b == b'\n')
                .filter(|l| !l.is_empty())
                .count(),
            _ => 0,
        };
        let tmp = dir.join(format!("{}-count.tmp", s.name));
        if fs::write(&tmp, n.to_string()).is_ok() {
            let _ = fs::rename(&tmp, &cache);
        }
    }
    let _ = fs::remove_file(dir.join("refreshing"));
}

fn spawn_refresh() {
    let lock = cache_dir().join("refreshing");
    let busy = mtime(&lock)
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age < Duration::from_secs(60));
    if busy || fs::create_dir_all(cache_dir()).is_err() || fs::write(&lock, "").is_err() {
        return;
    }
    if let Ok(exe) = env::current_exe() {
        let _ = Command::new(exe)
            .arg("--refresh-packages")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}

fn portage() -> Option<usize> {
    let categories = fs::read_dir("/var/db/pkg").ok()?;
    Some(
        categories
            .flatten()
            .filter_map(|c| count_dirs(c.path()))
            .sum(),
    )
}

fn count_dirs(path: impl AsRef<Path>) -> Option<usize> {
    let entries = fs::read_dir(path).ok()?;
    Some(
        entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .count(),
    )
}

fn count_lines(path: &str, matches: impl Fn(&str) -> bool) -> Option<usize> {
    let text = fs::read_to_string(path).ok()?;
    Some(text.lines().filter(|l| matches(l)).count())
}

fn cache_dir() -> PathBuf {
    let base = env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env::var_os("HOME").unwrap_or_default()).join(".cache"));
    base.join("warpfetch")
}

fn cache_file(name: &str) -> PathBuf {
    cache_dir().join(format!("{name}-count"))
}

fn mtime(p: impl AsRef<Path>) -> Option<SystemTime> {
    fs::metadata(p).and_then(|m| m.modified()).ok()
}
