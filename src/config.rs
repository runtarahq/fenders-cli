//! Persistent CLI configuration: named profiles, each one control plane and
//! one credential, and the workspace/project commands apply to within it.
//!
//! ```toml
//! current = "lab"
//! [profiles.lab]
//! api_url = "https://control.internal:9090/v1"
//! ca_cert = "/etc/fenders/pki/ca.pem"
//! role = "operator"
//! ```

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// What a profile's credential is: the control plane's operator token, or a
/// tenant's API key. Taken from `GET /v1/me` when the profile is added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Operator,
    #[serde(alias = "key")]
    Tenant,
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Role::Operator => "operator",
            Role::Tenant => "tenant",
        })
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct Profile {
    pub api_url: String,
    /// PEM file of the CA that signed the control plane's certificate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ca_cert: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_name: Option<String>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub profiles: BTreeMap<String, Profile>,
    /// The profile this invocation acts in (from `--profile`, the
    /// environment or `current`); `None` when flags alone name the API.
    #[serde(skip)]
    pub active: Option<String>,
    /// A profile that exists only for this invocation (`--api-url` with no
    /// saved profile). Never saved.
    #[serde(skip)]
    pub ephemeral: Profile,
}

/// The shape written by fenders-cli before profiles existed.
#[derive(Deserialize)]
struct Legacy {
    api_url: Option<String>,
    workspace: Option<String>,
    workspace_name: Option<String>,
    project: Option<String>,
    project_name: Option<String>,
}

/// `$FENDERS_CONFIG_DIR`, else `~/.config/fenders` (Unix) or `%APPDATA%\fenders`.
pub fn dir() -> PathBuf {
    if let Some(d) = std::env::var_os("FENDERS_CONFIG_DIR") {
        return PathBuf::from(d);
    }
    #[cfg(windows)]
    {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("fenders")
    }
    #[cfg(not(windows))]
    {
        if let Some(x) = std::env::var_os("XDG_CONFIG_HOME") {
            return PathBuf::from(x).join("fenders");
        }
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".config")
            .join("fenders")
    }
}

pub fn path() -> PathBuf {
    dir().join("config.toml")
}

/// A profile name: what a shell, a TOML key and a keychain account all take
/// without quoting.
pub fn valid_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        bail!("a profile name is 1 to 64 letters, digits, '-', '_' or '.' (got {name:?})");
    }
    Ok(())
}

impl Config {
    pub fn load() -> Result<Self> {
        let p = path();
        match std::fs::read_to_string(&p) {
            Ok(s) => {
                Self::parse(&s).with_context(|| format!("invalid config file {}", p.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("cannot read {}", p.display())),
        }
    }

    fn parse(s: &str) -> Result<Self> {
        let mut cfg: Config = toml::from_str(s)?;
        if cfg.profiles.is_empty() {
            // A config from before profiles: keep what it said as `default`.
            let old: Legacy = toml::from_str(s)?;
            if let Some(api_url) = old.api_url {
                cfg.profiles.insert(
                    "default".into(),
                    Profile {
                        api_url,
                        workspace: old.workspace,
                        workspace_name: old.workspace_name,
                        project: old.project,
                        project_name: old.project_name,
                        ..Profile::default()
                    },
                );
                cfg.current = Some("default".into());
            }
        }
        Ok(cfg)
    }

    pub fn save(&self) -> Result<()> {
        std::fs::create_dir_all(dir()).context("cannot create config directory")?;
        let s = toml::to_string_pretty(self)?;
        std::fs::write(path(), s).with_context(|| format!("cannot write {}", path().display()))
    }

    /// Choose the profile this invocation acts in: the one named, else
    /// `current`. A name that does not exist is an error; no name and no
    /// `current` leaves only the ephemeral profile.
    pub fn activate(&mut self, named: Option<&str>) -> Result<()> {
        let name = named.map(str::to_string).or_else(|| self.current.clone());
        if let Some(n) = &name
            && !self.profiles.contains_key(n)
        {
            let known: Vec<_> = self.profiles.keys().cloned().collect();
            bail!(
                "no profile named {n:?}{}",
                if known.is_empty() {
                    " — add one with `fenders profile add`".to_string()
                } else {
                    format!(" (profiles: {})", known.join(", "))
                }
            );
        }
        self.active = name;
        Ok(())
    }

    pub fn profile(&self) -> &Profile {
        match &self.active {
            Some(n) => &self.profiles[n],
            None => &self.ephemeral,
        }
    }

    pub fn profile_mut(&mut self) -> &mut Profile {
        match &self.active {
            Some(n) => self.profiles.get_mut(n).expect("active profile exists"),
            None => &mut self.ephemeral,
        }
    }

    /// Save, unless this invocation runs on flags alone.
    pub fn save_active(&self) -> Result<()> {
        if self.active.is_some() {
            self.save()
        } else {
            Ok(())
        }
    }

    pub fn set_workspace(&mut self, id: String, name: Option<String>) {
        let p = self.profile_mut();
        if p.workspace.as_deref() != Some(id.as_str()) {
            p.project = None;
            p.project_name = None;
        }
        p.workspace = Some(id);
        p.workspace_name = name;
    }

    pub fn set_project(&mut self, id: String, name: Option<String>) {
        let p = self.profile_mut();
        p.project = Some(id);
        p.project_name = name;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_config_from_before_profiles_becomes_the_default_profile() {
        let cfg = Config::parse(
            "api_url = \"https://cp.example/v1\"\nworkspace = \"w1\"\nproject = \"p1\"\n",
        )
        .unwrap();
        assert_eq!(cfg.current.as_deref(), Some("default"));
        let p = &cfg.profiles["default"];
        assert_eq!(p.api_url, "https://cp.example/v1");
        assert_eq!(p.workspace.as_deref(), Some("w1"));
        assert_eq!(p.project.as_deref(), Some("p1"));
    }

    #[test]
    fn profiles_round_trip() {
        let mut cfg = Config::default();
        cfg.profiles.insert(
            "lab".into(),
            Profile {
                api_url: "https://cp/v1".into(),
                ca_cert: Some("/ca.pem".into()),
                role: Some(Role::Operator),
                ..Profile::default()
            },
        );
        cfg.current = Some("lab".into());
        let back = Config::parse(&toml::to_string_pretty(&cfg).unwrap()).unwrap();
        assert_eq!(back.profiles, cfg.profiles);
        assert_eq!(back.current, cfg.current);
    }

    #[test]
    fn activating_an_unknown_profile_is_an_error() {
        let mut cfg = Config::default();
        assert!(cfg.activate(Some("nope")).is_err());
        cfg.activate(None).unwrap();
        assert!(cfg.active.is_none());
    }

    #[test]
    fn changing_workspace_forgets_the_project() {
        let mut cfg = Config::default();
        cfg.set_workspace("w1".into(), None);
        cfg.set_project("p1".into(), None);
        cfg.set_workspace("w2".into(), None);
        assert!(cfg.profile().project.is_none());
    }

    #[test]
    fn profile_names_are_checked() {
        assert!(valid_name("lab-op_1.x").is_ok());
        assert!(valid_name("").is_err());
        assert!(valid_name("a b").is_err());
        assert!(valid_name("a/b").is_err());
    }
}
