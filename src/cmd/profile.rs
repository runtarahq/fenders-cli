//! `fenders profile`: the control planes and credentials this CLI knows.
//!
//! A profile is one control plane (its URL and the CA that signed its
//! certificate) and one credential for it: the operator token, or a tenant's
//! API key. Which it is comes from `GET /v1/me` when the profile is added.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use serde_json::{Value, json};

use crate::app::{id_of, name_of};
use crate::auth;
use crate::client::{ApiError, Client, seg};
use crate::config::{self, Config, Profile, Role};
use crate::output::Out;
use crate::ui::{self, To};

#[derive(Subcommand, Debug)]
pub enum ProfileCmd {
    /// Add a profile: a control plane and a credential for it
    #[command(after_help = "\
Examples:
  # The operator token the control plane wrote at first start:
  fenders profile add lab --url https://control.internal:9090 \\
      --ca-cert pki/ca.pem --token-file /var/lib/fenders/control/operator.token

  # A tenant key, piped in:
  echo \"$KEY\" | fenders profile add acme --url https://control.internal:9090 --ca-cert pki/ca.pem")]
    Add(AddArgs),
    /// List profiles
    #[command(visible_alias = "ls")]
    List,
    /// Make a profile the current one
    Use {
        /// Profile name
        name: String,
    },
    /// Show a profile and check its credential
    Show {
        /// Profile name (default: the current one)
        name: Option<String>,
    },
    /// Remove a profile and its stored credential
    #[command(visible_alias = "rm")]
    Remove {
        /// Profile name
        name: String,
    },
}

#[derive(Args, Debug)]
pub struct AddArgs {
    /// Profile name (letters, digits, '-', '_', '.')
    pub name: String,
    /// The control plane's management API, e.g. https://control.internal:9090
    #[arg(long, value_name = "URL")]
    pub url: String,
    /// PEM file of the CA that signed the control plane's certificate (fenders-pki's ca.pem)
    #[arg(long, value_name = "PEM")]
    pub ca_cert: Option<PathBuf>,
    /// Read the credential from this file, e.g. STATE/operator.token
    #[arg(long, value_name = "FILE", conflicts_with = "with_token")]
    pub token_file: Option<PathBuf>,
    /// Read the credential from stdin (implied when stdin is not a terminal)
    #[arg(long)]
    pub with_token: bool,
    /// Replace a profile of the same name
    #[arg(long)]
    pub force: bool,
    /// Keep the current profile instead of switching to the new one
    #[arg(long)]
    pub keep_current: bool,
}

/// `https://host:9090` → `https://host:9090/v1`; a URL with a path is kept.
pub fn normalize_url(url: &str) -> Result<String> {
    let trimmed = url.trim().trim_end_matches('/');
    let parsed = reqwest::Url::parse(trimmed).with_context(|| format!("{url:?} is not a URL"))?;
    if !matches!(parsed.scheme(), "https" | "http") {
        bail!("the API URL must be https:// (got {url:?})");
    }
    Ok(if parsed.path() == "/" || parsed.path().is_empty() {
        format!("{trimmed}/v1")
    } else {
        trimmed.to_string()
    })
}

/// Read a CA file, and the absolute path to remember it by.
pub fn read_ca(path: &Path) -> Result<(String, Vec<u8>)> {
    let abs =
        std::path::absolute(path).with_context(|| format!("cannot resolve {}", path.display()))?;
    let pem = std::fs::read(&abs).with_context(|| format!("cannot read {}", abs.display()))?;
    Ok((abs.display().to_string(), pem))
}

/// What an invocation talks to and with which credential.
pub struct Connection {
    pub client: Client,
    pub api_url: String,
}

/// Connect with the active profile, overridden by `--api-url`/`--ca-cert`.
///
/// A stored credential is only ever sent to its own profile's URL: with
/// `--api-url` pointing elsewhere, only `FENDERS_TOKEN` is used.
pub fn connect(
    cfg: &Config,
    api_url_flag: Option<&str>,
    ca_flag: Option<&Path>,
    verbose: bool,
) -> Result<Connection> {
    let profile = cfg.profile();
    let api_url = match api_url_flag {
        Some(u) => normalize_url(u)?,
        None if !profile.api_url.is_empty() => profile.api_url.clone(),
        None => {
            return Err(crate::app::hinted(
                "no profile selected",
                &[
                    "Add one: fenders profile add NAME --url https://HOST:9090 --ca-cert ca.pem --token-file operator.token".into(),
                    "Or name the API for one command: --api-url URL with FENDERS_TOKEN set.".into(),
                ],
            ));
        }
    };
    let ca = match ca_flag {
        Some(p) => Some(read_ca(p)?.1),
        None => match &profile.ca_cert {
            Some(p) => Some(
                std::fs::read(p)
                    .with_context(|| format!("cannot read the profile's CA certificate {p}"))?,
            ),
            None => None,
        },
    };
    let own_url = cfg.active.is_some() && api_url == profile.api_url;
    let token = auth::env_token().or_else(|| {
        own_url
            .then(|| auth::token(cfg.active.as_deref()).map(|(t, _)| t))
            .flatten()
    });
    let client = Client::with_ca(&api_url, token, ca.as_deref(), verbose)?;
    Ok(Connection { client, api_url })
}

/// Explain a failed first contact: TLS trouble usually means a missing CA.
fn first_contact(e: anyhow::Error, url: &str, has_ca: bool) -> anyhow::Error {
    if let Some(api) = e.downcast_ref::<ApiError>()
        && api.status == 401
    {
        return anyhow::anyhow!("{url} rejected that credential");
    }
    let text = format!("{e:#}").to_ascii_lowercase();
    if text.contains("certificate") || text.contains("unknownissuer") || text.contains("tls") {
        return crate::app::hinted(
            format!("could not verify {url}'s certificate"),
            &[if has_ca {
                "Check that --ca-cert is the CA that signed the control plane's certificate.".into()
            } else {
                "Pass the CA that signed it: --ca-cert pki/ca.pem (fenders-pki's output).".into()
            }],
        );
    }
    e
}

/// Who a credential is, from `GET /v1/me`.
pub struct Identity {
    pub me: Value,
    pub role: Role,
}

pub fn identify(client: &Client) -> Result<Identity> {
    let me = client.get("/me", &[])?;
    let role = match me.get("role").and_then(Value::as_str) {
        Some("operator") => Role::Operator,
        _ => Role::Tenant,
    };
    Ok(Identity { me, role })
}

/// One line saying who a credential is.
pub fn describe(id: &Identity) -> String {
    match id.role {
        Role::Operator => "the operator".into(),
        Role::Tenant => {
            let label = id
                .me
                .get("label")
                .and_then(Value::as_str)
                .map(|l| format!(" ({})", ui::sanitize(l)))
                .unwrap_or_default();
            format!(
                "a key of workspace {}{label}",
                id.me
                    .get("workspace_id")
                    .or_else(|| id.me.get("tenant_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("?")
            )
        }
    }
}

/// A new profile for `client`'s credential: its role and, for a tenant key,
/// the workspace (and project) the key is scoped to, with their names.
pub fn profile_for(
    client: &Client,
    api_url: &str,
    ca_cert: Option<String>,
    id: &Identity,
) -> Profile {
    let mut p = Profile {
        api_url: api_url.to_string(),
        ca_cert,
        role: Some(id.role),
        ..Profile::default()
    };
    if id.role == Role::Tenant {
        let ws = id
            .me
            .get("workspace_id")
            .or_else(|| id.me.get("tenant_id"))
            .and_then(Value::as_str);
        if let Some(ws) = ws {
            p.workspace = Some(ws.to_string());
            p.workspace_name = client
                .get(&format!("/workspaces/{}", seg(ws)), &[])
                .ok()
                .as_ref()
                .and_then(name_of);
            let proj = id
                .me
                .get("project_id")
                .and_then(Value::as_str)
                .map(str::to_string);
            let projects = client
                .get(&format!("/workspaces/{}/projects", seg(ws)), &[])
                .ok()
                .map(|v| crate::app::items_of(&v))
                .unwrap_or_default();
            // A key of one project uses that project; a key of the whole
            // workspace starts in its only (default) project, if it has one.
            let chosen = match &proj {
                Some(pid) => projects.iter().find(|x| id_of(x).as_deref() == Some(pid)),
                None if projects.len() == 1 => projects.first(),
                None => None,
            };
            p.project = proj.or_else(|| chosen.and_then(id_of));
            p.project_name = chosen.and_then(name_of);
        }
    }
    p
}

/// Save `profile` under `name` with its credential. Makes it current when
/// asked, or when there is no current profile.
pub fn save(
    cfg: &mut Config,
    name: &str,
    profile: Profile,
    token: &str,
    make_current: bool,
) -> Result<auth::Source> {
    let source = auth::store(name, token)?;
    cfg.profiles.insert(name.to_string(), profile);
    if make_current || cfg.current.is_none() {
        cfg.current = Some(name.to_string());
    }
    cfg.save()?;
    Ok(source)
}

fn read_token(args: &AddArgs) -> Result<String> {
    use std::io::IsTerminal;
    let token = if let Some(f) = &args.token_file {
        std::fs::read_to_string(f).with_context(|| format!("cannot read {}", f.display()))?
    } else if args.with_token || !std::io::stdin().is_terminal() {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?;
        s
    } else if ui::interactive() {
        crate::cmd::read_secret("Operator token or API key", false)?
    } else {
        bail!("no credential: pass --token-file FILE, or pipe it in with --with-token");
    };
    let token = token.trim().to_string();
    if token.is_empty() {
        bail!("the credential is empty");
    }
    Ok(token)
}

pub fn run(mut cfg: Config, out: &Out, cmd: ProfileCmd, verbose: bool) -> Result<()> {
    match cmd {
        ProfileCmd::Add(args) => add(&mut cfg, out, args, verbose),
        ProfileCmd::List => list(&cfg, out),
        ProfileCmd::Use { name } => {
            cfg.activate(Some(&name))?;
            cfg.current = Some(name.clone());
            cfg.save()?;
            ui::success(&format!("Now using profile {}", ui::accent(&name, To::Err)));
            Ok(())
        }
        ProfileCmd::Show { name } => show(&mut cfg, out, name, verbose),
        ProfileCmd::Remove { name } => {
            if cfg.profiles.remove(&name).is_none() {
                bail!("no profile named {name:?}");
            }
            auth::clear(&name)?;
            if cfg.current.as_deref() == Some(name.as_str()) {
                cfg.current = cfg.profiles.keys().next().cloned();
            }
            cfg.save()?;
            ui::success(&format!("Removed profile {name}"));
            if let Some(c) = &cfg.current {
                ui::hint(&format!("Current profile: {c}"));
            }
            Ok(())
        }
    }
}

fn add(cfg: &mut Config, out: &Out, args: AddArgs, verbose: bool) -> Result<()> {
    config::valid_name(&args.name)?;
    if cfg.profiles.contains_key(&args.name) && !args.force {
        bail!(
            "a profile named {:?} exists — pass --force to replace it",
            args.name
        );
    }
    let api_url = normalize_url(&args.url)?;
    let ca = args.ca_cert.as_deref().map(read_ca).transpose()?;
    let token = read_token(&args)?;
    let client = Client::with_ca(
        &api_url,
        Some(token.clone()),
        ca.as_ref().map(|(_, pem)| pem.as_slice()),
        verbose,
    )?;
    let pb = ui::spinner("Checking the credential");
    let id = identify(&client);
    pb.finish_and_clear();
    let id = id.map_err(|e| first_contact(e, &api_url, ca.is_some()))?;
    let profile = profile_for(&client, &api_url, ca.map(|(p, _)| p), &id);
    let source = save(cfg, &args.name, profile.clone(), &token, !args.keep_current)?;
    if out.json {
        out.print_json(&json!({
            "name": args.name,
            "profile": profile,
            "identity": id.me,
            "current": cfg.current.as_deref() == Some(args.name.as_str()),
        }));
        return Ok(());
    }
    ui::success(&format!(
        "Added profile {} for {} at {} {}",
        ui::accent(&args.name, To::Err),
        describe(&id),
        ui::bold(&api_url, To::Err),
        ui::dim(&format!("(credential in the {source})"), To::Err)
    ));
    match id.role {
        Role::Operator => ui::hint("Next: `fenders tenants create NAME` and `fenders keys issue`."),
        Role::Tenant if profile.project.is_none() => {
            ui::hint("Next: pick a project with `fenders use`.")
        }
        Role::Tenant => ui::hint("Next: `fenders deploy app.wasm --app NAME`."),
    }
    Ok(())
}

fn list(cfg: &Config, out: &Out) -> Result<()> {
    let rows: Vec<Value> = cfg
        .profiles
        .iter()
        .map(|(name, p)| {
            let current = cfg.current.as_deref() == Some(name.as_str());
            json!({
                "name": if out.json || !current { name.clone() } else { format!("{name} *") },
                "current": if out.json { json!(current) } else { Value::Null },
                "role": p.role.map(|r| r.to_string()),
                "api_url": p.api_url,
                "workspace": p.workspace_name.clone().or(p.workspace.clone()),
                "project": p.project_name.clone().or(p.project.clone()),
            })
        })
        .collect();
    if rows.is_empty() && !out.json {
        ui::info("No profiles yet.");
        ui::hint(
            "Add one: fenders profile add NAME --url https://HOST:9090 --ca-cert ca.pem --token-file operator.token",
        );
        return Ok(());
    }
    out.list(
        &rows,
        &["name", "role", "api_url", "workspace", "project"],
        "profiles",
    );
    Ok(())
}

fn show(cfg: &mut Config, out: &Out, name: Option<String>, verbose: bool) -> Result<()> {
    cfg.activate(name.as_deref())?;
    let Some(name) = cfg.active.clone() else {
        bail!("no current profile — add one with `fenders profile add`");
    };
    let conn = connect(cfg, None, None, verbose)?;
    let source = auth::token(Some(&name)).map(|(_, s)| s.to_string());
    let id = identify(&conn.client);
    let profile = cfg.profile().clone();
    if out.json {
        let (identity, error) = match &id {
            Ok(i) => (i.me.clone(), Value::Null),
            Err(e) => (Value::Null, json!(format!("{e:#}"))),
        };
        out.print_json(&json!({
            "name": name,
            "profile": profile,
            "credential": source,
            "identity": identity,
            "error": error,
        }));
        return id.map(|_| ());
    }
    let row = |k: &str, v: String| println!("  {}  {v}", ui::dim(&format!("{k:<11}"), To::Out));
    row("profile", ui::bold(&name, To::Out));
    row("api", profile.api_url.clone());
    row(
        "ca",
        profile
            .ca_cert
            .clone()
            .unwrap_or_else(|| ui::dim("system roots", To::Out)),
    );
    row(
        "credential",
        source.unwrap_or_else(|| ui::dim("none stored", To::Out)),
    );
    match &id {
        Ok(i) => row("identity", describe(i)),
        Err(e) => row("identity", ui::state(&format!("unreachable: {e}"), To::Out)),
    }
    if let Some(w) = profile.workspace_name.or(profile.workspace) {
        row("workspace", w);
    }
    if let Some(p) = profile.project_name.or(profile.project) {
        row("project", p);
    }
    id.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::normalize_url;

    #[test]
    fn a_bare_host_gets_the_v1_base() {
        assert_eq!(
            normalize_url("https://cp.internal:9090/").unwrap(),
            "https://cp.internal:9090/v1"
        );
        assert_eq!(
            normalize_url("https://cp.internal:9090/v1/").unwrap(),
            "https://cp.internal:9090/v1"
        );
        assert!(normalize_url("ftp://x").is_err());
        assert!(normalize_url("not a url").is_err());
    }
}
