//! Credential storage, one per profile. `FENDERS_TOKEN` (or the older
//! `FENDERS_API_KEY`) wins; otherwise the OS keychain on macOS/Windows,
//! falling back to a 0600 credentials file (always used on Linux, which keeps
//! the binary fully static).

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::config;

#[cfg(any(target_os = "macos", target_os = "windows"))]
const SERVICE: &str = "fenders-cli";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Env,
    // Only produced where a system keychain is used (macOS, Windows).
    #[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
    Keychain,
    File,
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Source::Env => "FENDERS_TOKEN environment variable",
            Source::Keychain => "system keychain",
            Source::File => "credentials file",
        })
    }
}

fn file() -> PathBuf {
    config::dir().join("credentials.toml")
}

fn read_file() -> BTreeMap<String, String> {
    std::fs::read_to_string(file())
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_file(map: &BTreeMap<String, String>) -> Result<()> {
    std::fs::create_dir_all(config::dir())?;
    let body = toml::to_string(map)?;
    let p = file();
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&p)
            .with_context(|| format!("cannot write {}", p.display()))?;
        f.write_all(body.as_bytes())?;
    }
    #[cfg(not(unix))]
    std::fs::write(&p, body).with_context(|| format!("cannot write {}", p.display()))?;
    Ok(())
}

/// The variables that override a stored credential, in order.
pub const ENV_VARS: [&str; 2] = ["FENDERS_TOKEN", "FENDERS_API_KEY"];

/// The credential set in the environment, if any.
pub fn env_token() -> Option<String> {
    ENV_VARS.iter().find_map(|v| {
        std::env::var(v)
            .ok()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
    })
}

/// The credential for `profile` (none for an unsaved one), and where it came from.
pub fn token(profile: Option<&str>) -> Option<(String, Source)> {
    if let Some(t) = env_token() {
        return Some((t, Source::Env));
    }
    let profile = profile?;
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    if let Ok(t) = keyring::Entry::new(SERVICE, profile).and_then(|e| e.get_password()) {
        return Some((t, Source::Keychain));
    }
    read_file().remove(profile).map(|t| (t, Source::File))
}

/// Store `profile`'s credential, preferring the keychain.
pub fn store(profile: &str, key: &str) -> Result<Source> {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        match keyring::Entry::new(SERVICE, profile).and_then(|e| e.set_password(key)) {
            Ok(()) => {
                // Don't leave a stale plaintext copy behind.
                let mut m = read_file();
                if m.remove(profile).is_some() {
                    write_file(&m)?;
                }
                return Ok(Source::Keychain);
            }
            Err(e) => crate::ui::warn(&format!(
                "keychain unavailable ({e}); storing the key in {}",
                file().display()
            )),
        }
    }
    let mut m = read_file();
    m.insert(profile.to_string(), key.to_string());
    write_file(&m)?;
    Ok(Source::File)
}

/// Remove any stored credential for `profile`. Returns whether anything was removed.
pub fn clear(profile: &str) -> Result<bool> {
    let mut removed = false;
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    if let Ok(e) = keyring::Entry::new(SERVICE, profile) {
        removed |= e.delete_credential().is_ok();
    }
    let mut m = read_file();
    if m.remove(profile).is_some() {
        write_file(&m)?;
        removed = true;
    }
    Ok(removed)
}
