//! Configuration: the same `/etc/agh-dash/config.toml` the LVGL dashboard reads,
//! plus the `AGH_DASH_PASSWORD` override. Unknown keys (such as the C app's
//! `verify_tls`) are ignored, so one file serves both dashboards.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

/// Default config location; `--config <path>` overrides it.
pub const DEFAULT_PATH: &str = "/etc/agh-dash/config.toml";

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default = "default_username")]
    pub username: String,
    #[serde(default)]
    pub password: String,
    /// How often the statistics are fetched, in seconds.
    #[serde(default = "default_refresh_secs")]
    pub refresh_secs: u64,
    /// Per-request budget in milliseconds.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_base_url() -> String {
    "http://127.0.0.1:8080".to_string()
}

fn default_username() -> String {
    "admin".to_string()
}

fn default_refresh_secs() -> u64 {
    5
}

fn default_timeout_ms() -> u64 {
    10_000
}

impl Config {
    /// Load and validate the config, warning when the file is readable by
    /// anyone but its owner (it holds the AdGuard credentials).
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading {} (is the app installed?)", path.display()))?;
        let mut cfg: Config = toml::from_str(&text)
            .with_context(|| format!("parsing {}", path.display()))?;

        warn_when_world_readable(path);

        if let Ok(password) = std::env::var("AGH_DASH_PASSWORD") {
            cfg.password = password;
        }
        if cfg.password.is_empty() {
            bail!("no password in {} (or set AGH_DASH_PASSWORD)", path.display());
        }
        if cfg.refresh_secs == 0 {
            cfg.refresh_secs = default_refresh_secs();
        }
        Ok(cfg)
    }
}

#[cfg(unix)]
fn warn_when_world_readable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    if let Ok(meta) = std::fs::metadata(path) {
        let mode = meta.permissions().mode();
        if mode & 0o077 != 0 {
            eprintln!(
                "[agh-slint] warning: {} is readable by group/other (mode {:04o}); it holds the AdGuard credentials",
                path.display(),
                mode & 0o777
            );
        }
    }
}

#[cfg(not(unix))]
fn warn_when_world_readable(_path: &Path) {}

/// Resolve the config path from an optional `--config` argument.
pub fn path_from_args(args: &[String]) -> PathBuf {
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == "--config" {
            if let Some(path) = it.next() {
                return PathBuf::from(path);
            }
        }
    }
    PathBuf::from(DEFAULT_PATH)
}
