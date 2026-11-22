//! `auth`, `use` and `context`: who you are and where commands apply.

use anyhow::{Result, bail};
use clap::{Args, Subcommand};
use serde_json::{Value, json};

use crate::app::{App, id_of, name_of};
use crate::auth;
use crate::client::{ApiError, Client};
use crate::ui::{self, To};

#[derive(Subcommand, Debug)]
pub enum AuthCmd {
    /// Replace the current profile's credential (to add a profile, use `fenders profile add`)
    Login {
        /// Read the credential from stdin instead of prompting
        #[arg(long)]
        with_token: bool,
    },
    /// Forget the current profile's stored credential
    Logout,
    /// Show the current identity and where the credential comes from
    Status,
    /// Print the credential (for use in scripts)
    Token,
}

#[derive(Args, Debug)]
pub struct UseArgs {
    /// WORKSPACE or WORKSPACE/PROJECT, by name or id. Prompts when omitted.
    pub target: Option<String>,
}

fn identity(me: &Value) -> String {
    ["email", "name", "key_name", "label"]
        .iter()
        .find_map(|k| me.get(*k).and_then(Value::as_str))
        .map(|s| ui::sanitize(s).into_owned())
        .or_else(|| me.get("key").and_then(name_of))
        .or_else(|| id_of(me))
        .unwrap_or_else(|| "API key".into())
}

/// The active profile's name, for commands that act on its credential.
fn profile_name(app: &App) -> Result<String> {
    app.cfg
        .active
        .clone()
        .ok_or_else(|| anyhow::anyhow!("no profile selected — add one with `fenders profile add`"))
}

pub fn auth(app: &mut App, cmd: AuthCmd) -> Result<()> {
    match cmd {
        AuthCmd::Login { with_token } => login(app, with_token),
        AuthCmd::Logout => {
            let name = profile_name(app)?;
            if auth::clear(&name)? {
                ui::success(&format!("Forgot the credential of profile {name}"));
            } else {
                ui::info("No stored credential to remove.");
            }
            if auth::env_token().is_some() {
                ui::warn(
                    "FENDERS_TOKEN is still set in your environment and will keep being used.",
                );
            }
            Ok(())
        }
        AuthCmd::Status => {
            let Some((token, source)) = auth::token(app.cfg.active.as_deref()) else {
                bail!(ApiError {
                    status: 401,
                    code: "not_logged_in".into(),
                    message: format!("not logged in to {}", app.api_url),
                    request_id: None,
                });
            };
            let me = app.me()?;
            if app.out.json {
                app.out.print_json(&json!({
                    "profile": app.cfg.active,
                    "api_url": app.api_url,
                    "source": source.to_string(),
                    "identity": me,
                }));
                return Ok(());
            }
            ui::success(&format!(
                "Logged in to {} as {}",
                ui::bold(&app.api_url, To::Err),
                ui::accent(&identity(&me), To::Err)
            ));
            ui::hint(&format!("Key {} from the {source}", mask(&token)));
            println!();
            app.out.record(&me);
            Ok(())
        }
        AuthCmd::Token => match auth::token(app.cfg.active.as_deref()) {
            Some((t, _)) => {
                println!("{t}");
                Ok(())
            }
            None => bail!("no credential — add a profile with `fenders profile add`"),
        },
    }
}

fn mask(t: &str) -> String {
    let head: String = t.chars().take(6).collect();
    format!("{head}…")
}

fn login(app: &mut App, with_token: bool) -> Result<()> {
    let name = profile_name(app)?;
    let key = if with_token || !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?;
        s.trim().to_string()
    } else {
        if !ui::interactive() {
            bail!("no terminal to prompt on — pipe the credential in with `--with-token`");
        }
        crate::cmd::read_secret("Operator token or API key", false)?
            .trim()
            .to_string()
    };
    if key.is_empty() {
        bail!("no credential given");
    }

    let ca = match &app.cfg.profile().ca_cert {
        Some(p) => Some(std::fs::read(p)?),
        None => None,
    };
    let client = Client::with_ca(&app.api_url, Some(key.clone()), ca.as_deref(), false)?;
    let pb = ui::spinner("Checking the credential");
    let id = crate::cmd::profile::identify(&client);
    pb.finish_and_clear();
    let id = match id {
        Ok(i) => i,
        Err(e) => match e.downcast_ref::<ApiError>() {
            Some(api) if api.status == 401 => bail!("{} rejected that credential", app.api_url),
            _ => return Err(e),
        },
    };
    let fresh = crate::cmd::profile::profile_for(
        &client,
        &app.api_url,
        app.cfg.profile().ca_cert.clone(),
        &id,
    );
    let p = app.cfg.profile_mut();
    if p.role != fresh.role || fresh.workspace.is_some() {
        // Another kind of credential, or another tenant's: start from what it opens.
        let api_url = std::mem::take(&mut p.api_url);
        *p = fresh;
        p.api_url = api_url;
    }
    let source = auth::store(&name, &key)?;
    app.cfg.save()?;
    ui::success(&format!(
        "Profile {} now uses {} {}",
        ui::accent(&name, To::Err),
        crate::cmd::profile::describe(&id),
        ui::dim(&format!("(credential in the {source})"), To::Err)
    ));
    if auth::env_token().is_some() {
        ui::warn("FENDERS_TOKEN is set and takes precedence over the saved credential.");
    }
    app.client = client;
    Ok(())
}

pub fn use_(app: &mut App, args: UseArgs) -> Result<()> {
    choose(app, args.target.as_deref())
}

fn choose(app: &mut App, target: Option<&str>) -> Result<()> {
    let (ws_part, proj_part) = match target {
        Some(t) => match t.split_once('/') {
            Some((w, p)) => (Some(w), Some(p).filter(|p| !p.is_empty())),
            None => (Some(t), None),
        },
        None => (None, None),
    };
    let (ws, ws_name) = match ws_part {
        Some(w) => app.resolve_workspace(w)?,
        None if ui::interactive() => app.pick_workspace()?,
        None => bail!("give a target, e.g. `fenders use my-workspace/my-project`"),
    };
    let proj = match proj_part {
        Some(p) => Some(app.resolve_project(&ws, p)?),
        None if ui::interactive() => match app.pick_project(&ws) {
            Ok(p) => Some(p),
            Err(e) => {
                ui::warn(&e.to_string());
                None
            }
        },
        None => None,
    };
    app.cfg.set_workspace(ws.clone(), ws_name.clone());
    if let Some((id, name)) = &proj {
        app.cfg.set_project(id.clone(), name.clone());
    }
    app.cfg.save_active()?;
    let show = |name: &Option<String>, id: &str| name.clone().unwrap_or_else(|| id.to_string());
    let mut msg = format!(
        "Now using workspace {}",
        ui::accent(&show(&ws_name, &ws), To::Err)
    );
    if let Some((id, name)) = &proj {
        msg.push_str(&format!(
            " · project {}",
            ui::accent(&show(name, id), To::Err)
        ));
    }
    ui::success(&msg);
    Ok(())
}

pub fn context(app: &mut App) -> Result<()> {
    let source = auth::token(app.cfg.active.as_deref()).map(|(_, s)| s.to_string());
    let (ws, proj) = app.effective_context();
    let as_json = |c: &Option<(String, Option<String>, bool)>| match c {
        Some((id, name, overridden)) => json!({
            "id": id,
            "name": name,
            "source": if *overridden { "flag or environment" } else { "config" },
        }),
        None => Value::Null,
    };
    if app.out.json {
        app.out.print_json(&json!({
            "profile": app.cfg.active,
            "role": app.cfg.profile().role.map(|r| r.to_string()),
            "api_url": app.api_url,
            "workspace": as_json(&ws),
            "project": as_json(&proj),
            "auth": source.clone().unwrap_or_else(|| "not logged in".into()),
            "config": crate::config::path().display().to_string(),
        }));
        return Ok(());
    }
    let row = |k: &str, v: String| println!("  {}  {v}", ui::dim(&format!("{k:<10}"), To::Out));
    let named = |c: &Option<(String, Option<String>, bool)>| match c {
        Some((id, name, overridden)) => {
            let mut s = match name {
                Some(n) => format!("{} {}", ui::bold(n, To::Out), ui::dim(id, To::Out)),
                None => id.clone(),
            };
            if *overridden {
                s.push_str(&format!(" {}", ui::accent("(override)", To::Out)));
            }
            s
        }
        None => ui::dim("not set — run `fenders use`", To::Out),
    };
    row(
        "profile",
        match (&app.cfg.active, app.cfg.profile().role) {
            (Some(n), Some(r)) => format!(
                "{} {}",
                ui::bold(n, To::Out),
                ui::dim(&format!("({r})"), To::Out)
            ),
            (Some(n), None) => ui::bold(n, To::Out),
            (None, _) => ui::dim("none — run `fenders profile add`", To::Out),
        },
    );
    row("api", app.api_url.clone());
    row(
        "auth",
        source.unwrap_or_else(|| ui::dim("no credential — run `fenders profile add`", To::Out)),
    );
    row("workspace", named(&ws));
    row("project", named(&proj));
    row("config", crate::config::path().display().to_string());
    Ok(())
}
