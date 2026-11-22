//! The operator's commands: tenants, their keys, nodes, edges, the public
//! egress policy, the control plane's status, and the audit trail.
//!
//! Every route here but `audit` and `tenants list|get` is the operator's
//! alone; a tenant key is answered 403. The request bodies are built by the
//! `*_body` functions so the TUI sends exactly what these commands send.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use clap::{Args, Subcommand};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::app::{App, id_of, items_of, name_of};
use crate::client::seg;
use crate::cmd::{ListArgs, confirm};
use crate::config::{Profile, Role};
use crate::ui::{self, To};

/// Every permission a tenant key can hold.
pub const PERMISSIONS: [&str; 6] = [
    "read",
    "apps:write",
    "runs:write",
    "projects:write",
    "connections:write",
    "secrets:write",
];

// ---------------------------------------------------------------- parsing

/// `1GiB`, `512MiB`, `10G`, `2048` (bytes) → bytes.
pub fn parse_bytes(s: &str) -> Result<u64> {
    let t = s.trim();
    let split = t
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(t.len());
    let (num, unit) = t.split_at(split);
    let n: f64 = num
        .parse()
        .map_err(|_| anyhow!("{s:?} is not a size (e.g. 512MiB, 1GiB, 10GB)"))?;
    let mult: f64 = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1.0,
        "k" | "kb" => 1e3,
        "m" | "mb" => 1e6,
        "g" | "gb" => 1e9,
        "t" | "tb" => 1e12,
        "ki" | "kib" => 1024.0,
        "mi" | "mib" => 1024.0 * 1024.0,
        "gi" | "gib" => 1024.0 * 1024.0 * 1024.0,
        "ti" | "tib" => 1024.0_f64.powi(4),
        u => bail!("unknown size unit {u:?} in {s:?}"),
    };
    Ok((n * mult).round() as u64)
}

/// A size in MiB, for memory: `512MiB`, `1GiB`, or a bare number of MiB.
pub fn parse_mib(s: &str) -> Result<u64> {
    if s.trim().chars().all(|c| c.is_ascii_digit()) {
        return s.trim().parse().map_err(Into::into);
    }
    Ok(parse_bytes(s)? / (1024 * 1024))
}

/// `30d`, `12h`, `15m`, `900s` or `900` → seconds.
pub fn parse_duration(s: &str) -> Result<u64> {
    let t = s.trim();
    let (num, mult) = match t.chars().last() {
        Some('d') => (&t[..t.len() - 1], 86_400),
        Some('h') => (&t[..t.len() - 1], 3_600),
        Some('m') => (&t[..t.len() - 1], 60),
        Some('s') => (&t[..t.len() - 1], 1),
        _ => (t, 1),
    };
    let n: u64 = num
        .parse()
        .map_err(|_| anyhow!("{s:?} is not a duration (e.g. 30d, 12h, 15m)"))?;
    Ok(n * mult)
}

/// The pin the control plane keeps for a node or an edge: the SHA-256 of
/// its certificate's DER, as `fenders-pki` prints it.
pub fn cert_pin(pem: &str) -> Result<String> {
    let begin = "-----BEGIN CERTIFICATE-----";
    let end = "-----END CERTIFICATE-----";
    let start = pem.find(begin).context("no certificate in the PEM file")? + begin.len();
    let stop = pem[start..].find(end).context("unterminated certificate")? + start;
    let b64: String = pem[start..stop].split_whitespace().collect();
    let der = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .context("the certificate is not valid base64")?;
    Ok(Sha256::digest(&der)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

fn pin_from(cert: &Option<PathBuf>, pin: &Option<String>) -> Result<String> {
    match (cert, pin) {
        (Some(path), _) => cert_pin(
            &std::fs::read_to_string(path)
                .with_context(|| format!("cannot read {}", path.display()))?,
        ),
        (None, Some(p)) => {
            let p = p.trim().to_ascii_lowercase();
            if p.len() != 64 || !p.chars().all(|c| c.is_ascii_hexdigit()) {
                bail!("a pin is 64 hexadecimal characters (fenders-pki's output)");
            }
            Ok(p)
        }
        (None, None) => bail!("give the certificate (--cert FILE) or its pin (--pin HEX)"),
    }
}

// ---------------------------------------------------------------- tenants

#[derive(Args, Debug, Clone, Default)]
pub struct AllowanceArgs {
    /// CPU its runs may hold at once, in hundredths of a core (200 = two cores)
    #[arg(long, value_name = "N")]
    pub cpu_percent: Option<u64>,
    /// Memory its runs may hold at once (e.g. 1GiB, 512MiB)
    #[arg(long, value_name = "SIZE")]
    pub memory: Option<String>,
    /// Storage for its artifacts (e.g. 1GiB)
    #[arg(long, value_name = "SIZE")]
    pub storage: Option<String>,
    /// Executions at once
    #[arg(long, value_name = "N")]
    pub instances: Option<u64>,
    /// Outbound and gateway transfer per UTC month (e.g. 10GiB)
    #[arg(long, value_name = "SIZE")]
    pub transfer: Option<String>,
}

impl AllowanceArgs {
    pub fn is_empty(&self) -> bool {
        self.cpu_percent.is_none()
            && self.memory.is_none()
            && self.storage.is_none()
            && self.instances.is_none()
            && self.transfer.is_none()
    }

    /// The whole allowance: these flags over `base` (a workspace record or
    /// the defaults). The API takes an allowance only whole.
    pub fn over(&self, base: &Value) -> Result<Value> {
        let get = |k: &str| base.get(k).and_then(Value::as_u64);
        let need = |k: &str, v: Option<u64>| {
            v.or_else(|| get(k))
                .ok_or_else(|| anyhow!("the allowance needs {k}"))
        };
        Ok(json!({
            "cpu_percent": need("cpu_percent", self.cpu_percent)?,
            "memory_mib": need("memory_mib", self.memory.as_deref().map(parse_mib).transpose()?)?,
            "storage_bytes": need("storage_bytes", self.storage.as_deref().map(parse_bytes).transpose()?)?,
            "instances": need("instances", self.instances)?,
            "transfer_bytes": need("transfer_bytes", self.transfer.as_deref().map(parse_bytes).transpose()?)?,
        }))
    }
}

/// What a new tenant may use when the command names nothing else.
pub fn default_allowance() -> Value {
    json!({
        "cpu_percent": 100,
        "memory_mib": 512,
        "storage_bytes": 1u64 << 30,
        "instances": 2,
        "transfer_bytes": 1u64 << 30,
    })
}

#[derive(Subcommand, Debug)]
pub enum TenantsCmd {
    /// List tenants (a tenant key sees only its own)
    #[command(visible_alias = "ls")]
    List(ListArgs),
    /// Show a tenant: its allowance, limits and state
    Get {
        /// Tenant name or id
        tenant: String,
    },
    /// Create a tenant (a workspace, with a default project)
    #[command(after_help = "\
Defaults: one core (--cpu-percent 100), 512MiB memory, 1GiB storage,
2 instances, 1GiB transfer a month.

Example:
  fenders tenants create acme --cpu-percent 200 --memory 1GiB --instances 4 --transfer 10GiB")]
    Create {
        /// Its name (a label: two tenants may share one)
        name: String,
        #[command(flatten)]
        allowance: AllowanceArgs,
        /// App size for apps that name none
        #[arg(long, value_name = "SIZE")]
        default_size: Option<String>,
    },
    /// Change a tenant's allowance or default app size
    Set {
        /// Tenant name or id
        tenant: String,
        #[command(flatten)]
        allowance: AllowanceArgs,
        /// App size for apps that name none
        #[arg(long, value_name = "SIZE")]
        default_size: Option<String>,
    },
    /// Change a tenant's HTTP limits
    HttpLimits {
        /// Tenant name or id
        tenant: String,
        /// Requests its HTTP apps serve at once
        #[arg(long, value_name = "N")]
        concurrent: Option<u64>,
        /// Requests a minute its HTTP apps serve
        #[arg(long, value_name = "N")]
        per_minute: Option<u64>,
    },
    /// Suspend a tenant: its runs end, its keys may only read
    Suspend {
        /// Tenant name or id
        tenant: String,
        /// Why (kept in the audit trail)
        #[arg(long)]
        reason: String,
        /// Return without waiting for its runs to end
        #[arg(long)]
        no_wait: bool,
    },
    /// Resume a suspended tenant
    Resume {
        /// Tenant name or id
        tenant: String,
        /// Why (kept in the audit trail)
        #[arg(long, default_value = "resumed")]
        reason: String,
    },
    /// Delete a suspended tenant, its keys, secrets and storage. Cannot be undone
    #[command(visible_alias = "rm")]
    Delete {
        /// Tenant name or id
        tenant: String,
        /// Don't ask for confirmation
        #[arg(long, short = 'y')]
        yes: bool,
    },
}

const TENANT_COLS: &[&str] = &[
    "id",
    "name",
    "lifecycle_state",
    "cpu_percent",
    "memory_mib",
    "instances",
    "created",
];

fn tenant_path(app: &App, tenant: &str) -> Result<(String, String)> {
    let (id, name) = app.resolve_workspace(tenant)?;
    Ok((
        format!("/workspaces/{}", seg(&id)),
        name.unwrap_or_else(|| id.clone()),
    ))
}

pub fn tenants(app: &mut App, cmd: TenantsCmd) -> Result<()> {
    match cmd {
        TenantsCmd::List(l) => {
            let want = l.want(&app.out);
            let (items, more) = app.collect("/workspaces", vec![], "before", Some(100), want)?;
            app.out.list(&items, TENANT_COLS, "tenants");
            l.more_hint(more, items.len());
        }
        TenantsCmd::Get { tenant } => {
            let (path, _) = tenant_path(app, &tenant)?;
            let v = app.client.get(&path, &[])?;
            app.out.record(&v);
        }
        TenantsCmd::Create {
            name,
            allowance,
            default_size,
        } => {
            let body = json!({"name": name, "allowance": allowance.over(&default_allowance())?});
            let v = app.client.post("/workspaces", body)?;
            let id = id_of(&v).context("the new tenant has no id")?;
            if let Some(size) = default_size {
                app.client.patch(
                    &format!("/workspaces/{}", seg(&id)),
                    json!({"default_app_size": size}),
                )?;
            }
            if app.out.json {
                app.out.print_json(&v);
            } else {
                ui::success(&format!(
                    "Created tenant {} {}",
                    ui::accent(&name, To::Err),
                    ui::dim(&id, To::Err)
                ));
                ui::hint(&format!(
                    "Give it a key: fenders keys issue {id} --save-profile {}",
                    profile_name_for(&name)
                ));
            }
        }
        TenantsCmd::Set {
            tenant,
            allowance,
            default_size,
        } => {
            if allowance.is_empty() && default_size.is_none() {
                bail!("nothing to change — give an allowance flag or --default-size");
            }
            let (path, label) = tenant_path(app, &tenant)?;
            let cur = app.client.get(&path, &[])?;
            let mut body = Map::new();
            if !allowance.is_empty() {
                body.insert("allowance".into(), allowance.over(&cur)?);
                if let Some(r) = cur.get("allowance_revision") {
                    body.insert("expected_revision".into(), r.clone());
                }
            }
            if let Some(s) = default_size {
                body.insert("default_app_size".into(), json!(s));
            }
            let v = app.client.patch(&path, Value::Object(body))?;
            done(app, &v, &format!("Updated tenant {label}"));
        }
        TenantsCmd::HttpLimits {
            tenant,
            concurrent,
            per_minute,
        } => {
            let (path, label) = tenant_path(app, &tenant)?;
            let cur = app.client.get(&path, &[])?;
            let pick = |v: Option<u64>, k: &str| {
                v.or_else(|| cur.get(k).and_then(Value::as_u64))
                    .ok_or_else(|| {
                        anyhow!(
                            "give --{}",
                            if k.contains("concurrent") {
                                "concurrent"
                            } else {
                                "per-minute"
                            }
                        )
                    })
            };
            let body = json!({
                "concurrent_requests": pick(concurrent, "http_concurrent_requests")?,
                "requests_per_minute": pick(per_minute, "http_requests_per_minute")?,
            });
            let v = app.client.patch(&format!("{path}/http-limits"), body)?;
            done(app, &v, &format!("Updated the HTTP limits of {label}"));
        }
        TenantsCmd::Suspend {
            tenant,
            reason,
            no_wait,
        } => {
            let v = lifecycle(app, &tenant, "suspend", &reason)?;
            if !no_wait {
                let (path, label) = tenant_path(app, &tenant)?;
                wait_state(app, &path, "suspended", &format!("Suspending {label}"))?;
            }
            done(app, &v, "Suspended");
        }
        TenantsCmd::Resume { tenant, reason } => {
            let v = lifecycle(app, &tenant, "resume", &reason)?;
            done(app, &v, "Resumed");
        }
        TenantsCmd::Delete { tenant, yes } => {
            let (path, label) = tenant_path(app, &tenant)?;
            confirm(
                &format!(
                    "Delete tenant {label} with its keys, secrets and storage? This cannot be undone"
                ),
                yes,
            )?;
            let v = app.client.delete(&path)?;
            done(app, &v, &format!("Deleted tenant {label}"));
        }
    }
    Ok(())
}

fn lifecycle(app: &mut App, tenant: &str, action: &str, reason: &str) -> Result<Value> {
    let (path, _) = tenant_path(app, tenant)?;
    let cur = app.client.get(&path, &[])?;
    let rev = cur
        .get("lifecycle_revision")
        .cloned()
        .context("the tenant record has no lifecycle_revision")?;
    app.client.post(
        &format!("{path}/lifecycle"),
        json!({"action": action, "reason": reason, "expected_revision": rev}),
    )
}

/// Poll a tenant until its `lifecycle_state` is `want`.
fn wait_state(app: &mut App, path: &str, want: &str, label: &str) -> Result<()> {
    let pb = ui::spinner(label.to_string());
    let started = Instant::now();
    loop {
        let v = app.client.get(path, &[])?;
        if v.get("lifecycle_state").and_then(Value::as_str) == Some(want) {
            pb.finish_and_clear();
            return Ok(());
        }
        if started.elapsed() > Duration::from_secs(600) {
            pb.finish_and_clear();
            bail!("still not {want} after 10 minutes; check with `fenders tenants get`");
        }
        std::thread::sleep(Duration::from_millis(700));
    }
}

fn done(app: &App, v: &Value, msg: &str) {
    if app.out.json {
        app.out.print_json(v);
    } else {
        ui::success(msg);
    }
}

/// A profile name made from a tenant's name.
fn profile_name_for(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .take(64)
        .collect();
    if s.is_empty() { "tenant".into() } else { s }
}

// ---------------------------------------------------------------- keys

#[derive(Subcommand, Debug)]
pub enum KeysCmd {
    /// Issue a key for a tenant (its token is shown once)
    #[command(after_help = "\
Permissions: read, apps:write, runs:write, projects:write, connections:write,
secrets:write, or all (the default).

Examples:
  fenders keys issue acme --save-profile acme
  fenders keys issue acme --permissions read,runs:write --ttl 15m --label 'user:42'")]
    Issue {
        /// Tenant name or id
        tenant: String,
        /// The key's name
        #[arg(long, default_value = "fenders-cli")]
        name: String,
        /// Comma-separated permissions, or `all`
        #[arg(long, default_value = "all", value_delimiter = ',')]
        permissions: Vec<String>,
        /// How long it works, at most 30d (e.g. 30d, 12h, 15m)
        #[arg(long, default_value = "30d")]
        ttl: String,
        /// Scope it to one project (name or id)
        #[arg(long)]
        project: Option<String>,
        /// Who it is for; kept with every change it makes
        #[arg(long)]
        label: Option<String>,
        /// Also save it as a tenant profile of this name
        #[arg(long, value_name = "PROFILE")]
        save_profile: Option<String>,
    },
    /// List a tenant's keys (without their tokens)
    #[command(visible_alias = "ls")]
    List {
        /// Tenant name or id
        tenant: String,
        #[command(flatten)]
        list: ListArgs,
    },
    /// Revoke a key; it stops working at once
    Revoke {
        /// Tenant name or id
        tenant: String,
        /// Key id
        key: String,
    },
}

pub fn issue_body(
    name: &str,
    permissions: &[String],
    ttl_seconds: u64,
    project_id: Option<&str>,
    label: Option<&str>,
) -> Result<Value> {
    let perms: Vec<String> = if permissions.iter().any(|p| p == "all") {
        PERMISSIONS.iter().map(|s| s.to_string()).collect()
    } else {
        for p in permissions {
            if !PERMISSIONS.contains(&p.as_str()) {
                bail!(
                    "unknown permission {p:?} (one of: {}, all)",
                    PERMISSIONS.join(", ")
                );
            }
        }
        permissions.to_vec()
    };
    if !(60..=2_592_000).contains(&ttl_seconds) {
        bail!("a key lives between 1 minute and 30 days");
    }
    let mut body = json!({"name": name, "permissions": perms, "ttl_seconds": ttl_seconds});
    if let Some(p) = project_id {
        body["project_id"] = json!(p);
    }
    if let Some(l) = label {
        body["label"] = json!(l);
    }
    Ok(body)
}

pub fn keys(app: &mut App, cmd: KeysCmd) -> Result<()> {
    match cmd {
        KeysCmd::Issue {
            tenant,
            name,
            permissions,
            ttl,
            project,
            label,
            save_profile,
        } => {
            if let Some(p) = &save_profile {
                crate::config::valid_name(p)?;
                if app.cfg.profiles.contains_key(p) {
                    bail!("a profile named {p:?} exists — choose another --save-profile name");
                }
            }
            let (ws, ws_name) = app.resolve_workspace(&tenant)?;
            let project = project.map(|p| app.resolve_project(&ws, &p)).transpose()?;
            let body = issue_body(
                &name,
                &permissions,
                parse_duration(&ttl)?,
                project.as_ref().map(|(id, _)| id.as_str()),
                label.as_deref(),
            )?;
            let v = app
                .client
                .post(&format!("/workspaces/{}/api-keys", seg(&ws)), body)?;
            let token = v.get("token").and_then(Value::as_str);
            let Some(token) = token else {
                bail!(
                    "the key was issued before (a replayed request), and its token is not kept; revoke {} and issue another",
                    id_of(&v).unwrap_or_default()
                );
            };
            let mut saved = None;
            if let Some(pname) = &save_profile {
                let mut profile = Profile {
                    api_url: app.api_url.clone(),
                    ca_cert: app.cfg.profile().ca_cert.clone(),
                    role: Some(Role::Tenant),
                    workspace: Some(ws.clone()),
                    workspace_name: ws_name.clone(),
                    ..Profile::default()
                };
                match &project {
                    Some((id, n)) => {
                        profile.project = Some(id.clone());
                        profile.project_name = n.clone();
                    }
                    None => {
                        let all = items_of(
                            &app.client
                                .get(&format!("/workspaces/{}/projects", seg(&ws)), &[])?,
                        );
                        if let [only] = all.as_slice() {
                            profile.project = id_of(only);
                            profile.project_name = name_of(only);
                        }
                    }
                }
                crate::cmd::profile::save(&mut app.cfg, pname, profile, token, false)?;
                saved = Some(pname.clone());
            }
            if app.out.json {
                let mut out = v.clone();
                out["saved_profile"] = json!(saved);
                app.out.print_json(&out);
                return Ok(());
            }
            ui::success(&format!(
                "Issued key {} for {}",
                ui::accent(&name, To::Err),
                ws_name.unwrap_or(ws)
            ));
            // The token alone on stdout, so `TOKEN=$(fenders keys issue …)` works.
            println!("{token}");
            ui::warn("This is the only time the token is shown.");
            if let Some(p) = saved {
                ui::hint(&format!(
                    "Saved as profile {p}: `fenders -P {p} context`, or `fenders profile use {p}`"
                ));
            }
        }
        KeysCmd::List { tenant, list } => {
            let (path, _) = tenant_path(app, &tenant)?;
            let want = list.want(&app.out);
            let (items, more) = app.collect(
                &format!("{path}/api-keys"),
                vec![],
                "before",
                Some(100),
                want,
            )?;
            app.out.list(
                &items,
                &[
                    "id",
                    "name",
                    "prefix",
                    "permissions",
                    "project_id",
                    "expires",
                    "revoked",
                ],
                "keys",
            );
            list.more_hint(more, items.len());
        }
        KeysCmd::Revoke { tenant, key } => {
            let (path, _) = tenant_path(app, &tenant)?;
            let v = app
                .client
                .delete(&format!("{path}/api-keys/{}", seg(&key)))?;
            done(app, &v, &format!("Revoked key {key}"));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- nodes

#[derive(Subcommand, Debug)]
pub enum NodesCmd {
    /// List nodes
    #[command(visible_alias = "ls")]
    List(ListArgs),
    /// Show a node
    Get {
        /// Node name or id
        node: String,
    },
    /// Register a node by its certificate
    #[command(after_help = "\
Example:
  fenders nodes register node-1 --cert pki/node.pem --endpoint https://node-1.internal:9443 \\
      --runtime wasmtime-50.0.0-fenders-v1 --cpu-percent 200 --memory 1GiB --instances 4")]
    Register(Box<RegisterNode>),
    /// Change a node's capacity or launchers
    Set {
        /// Node name or id
        node: String,
        #[arg(long, value_name = "N")]
        cpu_percent: Option<u64>,
        #[arg(long, value_name = "SIZE")]
        memory: Option<String>,
        #[arg(long, value_name = "N")]
        instances: Option<u64>,
        /// Require the outbound byte limit on this node
        #[arg(long, value_name = "BOOL")]
        egress_budget_required: Option<bool>,
    },
    /// Stop placing runs on a node
    Drain {
        /// Node name or id
        node: String,
    },
    /// Place runs on a drained node again
    Undrain {
        /// Node name or id
        node: String,
    },
    /// Disable a node
    Disable {
        /// Node name or id
        node: String,
    },
    /// Enable a disabled node
    Enable {
        /// Node name or id
        node: String,
    },
    /// Delete a drained node
    #[command(visible_alias = "rm")]
    Delete {
        /// Node name or id
        node: String,
        #[arg(long, short = 'y')]
        yes: bool,
    },
}

#[derive(Args, Debug)]
pub struct RegisterNode {
    /// Its name
    pub name: String,
    /// Its certificate (PEM); the pin is computed from it
    #[arg(long, value_name = "PEM", conflicts_with = "pin")]
    pub cert: Option<PathBuf>,
    /// Its certificate's pin (64 hex characters, as fenders-pki prints)
    #[arg(long)]
    pub pin: Option<String>,
    /// Where the control plane reaches it, https://host:port
    #[arg(long)]
    pub endpoint: String,
    /// The runtime it runs, e.g. wasmtime-50.0.0-fenders-v1
    #[arg(long)]
    pub runtime: String,
    #[arg(long, default_value = "wasm")]
    pub backend: String,
    #[arg(long, default_value = "x86_64")]
    pub architecture: String,
    /// CPU it offers, in hundredths of a core
    #[arg(long, value_name = "N")]
    pub cpu_percent: u64,
    /// Memory it offers (e.g. 4GiB)
    #[arg(long, value_name = "SIZE")]
    pub memory: String,
    /// Runs at once
    #[arg(long, value_name = "N")]
    pub instances: u64,
    /// Require the outbound byte limit (for tenants not trusted with unlimited transfer)
    #[arg(long)]
    pub egress_budget_required: bool,
    /// The approved launcher's digest, when the byte limit is required
    #[arg(long, value_name = "SHA256")]
    pub egress_budget_launcher: Option<String>,
}

fn find_named(app: &App, collection: &str, needle: &str, noun: &str) -> Result<Value> {
    let (items, _) = app.collect(collection, vec![], "before", Some(100), None)?;
    crate::app::find(&items, needle)
        .cloned()
        .ok_or_else(|| anyhow!("no {noun} named \"{needle}\""))
}

const NODE_COLS: &[&str] = &[
    "id",
    "name",
    "endpoint",
    "cpu_percent",
    "memory_mib",
    "instances",
    "draining",
    "disabled",
    "last_seen",
];

fn node_mode(app: &mut App, node: &str, body: Value, msg: &str) -> Result<()> {
    let n = find_named(app, "/nodes", node, "node")?;
    let id = id_of(&n).context("node record has no id")?;
    let v = app.client.patch(&format!("/nodes/{}", seg(&id)), body)?;
    done(app, &v, msg);
    Ok(())
}

pub fn nodes(app: &mut App, cmd: NodesCmd) -> Result<()> {
    match cmd {
        NodesCmd::List(l) => {
            let want = l.want(&app.out);
            let (items, more) = app.collect("/nodes", vec![], "before", Some(100), want)?;
            app.out.list(&items, NODE_COLS, "nodes");
            l.more_hint(more, items.len());
        }
        NodesCmd::Get { node } => {
            let n = find_named(app, "/nodes", &node, "node")?;
            app.out.record(&n);
        }
        NodesCmd::Register(r) => {
            let mut body = json!({
                "name": r.name,
                "cert_sha256": pin_from(&r.cert, &r.pin)?,
                "endpoint": r.endpoint,
                "backend": r.backend,
                "architecture": r.architecture,
                "runtime": r.runtime,
                "cpu_percent": r.cpu_percent,
                "memory_mib": parse_mib(&r.memory)?,
                "instances": r.instances,
                "egress_budget_required": r.egress_budget_required,
            });
            if let Some(l) = &r.egress_budget_launcher {
                body["egress_budget_launcher"] = json!(l);
            }
            let v = app.client.post("/nodes", body)?;
            done(app, &v, &format!("Registered node {}", r.name));
        }
        NodesCmd::Set {
            node,
            cpu_percent,
            memory,
            instances,
            egress_budget_required,
        } => {
            let mut body = Map::new();
            if let Some(c) = cpu_percent {
                body.insert("cpu_percent".into(), json!(c));
            }
            if let Some(m) = memory {
                body.insert("memory_mib".into(), json!(parse_mib(&m)?));
            }
            if let Some(i) = instances {
                body.insert("instances".into(), json!(i));
            }
            if let Some(e) = egress_budget_required {
                body.insert("egress_budget_required".into(), json!(e));
            }
            if body.is_empty() {
                bail!("nothing to change");
            }
            node_mode(
                app,
                &node,
                Value::Object(body),
                &format!("Updated node {node}"),
            )?;
        }
        NodesCmd::Drain { node } => node_mode(
            app,
            &node,
            json!({"draining": true}),
            &format!("Draining node {node}"),
        )?,
        NodesCmd::Undrain { node } => node_mode(
            app,
            &node,
            json!({"draining": false}),
            &format!("Node {node} takes runs again"),
        )?,
        NodesCmd::Disable { node } => node_mode(
            app,
            &node,
            json!({"disabled": true}),
            &format!("Disabled node {node}"),
        )?,
        NodesCmd::Enable { node } => node_mode(
            app,
            &node,
            json!({"disabled": false}),
            &format!("Enabled node {node}"),
        )?,
        NodesCmd::Delete { node, yes } => {
            let n = find_named(app, "/nodes", &node, "node")?;
            let id = id_of(&n).context("node record has no id")?;
            confirm(&format!("Delete node {node}?"), yes)?;
            let v = app.client.delete(&format!("/nodes/{}", seg(&id)))?;
            done(app, &v, &format!("Deleted node {node}"));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- edges

#[derive(Subcommand, Debug)]
pub enum EdgesCmd {
    /// List edges
    #[command(visible_alias = "ls")]
    List(ListArgs),
    /// Register an edge by its certificate
    Register {
        /// Its name
        name: String,
        /// Its certificate (PEM)
        #[arg(long, value_name = "PEM", conflicts_with = "pin")]
        cert: Option<PathBuf>,
        /// Its certificate's pin
        #[arg(long)]
        pin: Option<String>,
        /// Where other edges reach it, https://host:port
        #[arg(long)]
        endpoint: String,
    },
    /// Stop giving an edge new apps
    Drain {
        /// Edge name or id
        edge: String,
    },
    /// Give a drained edge apps again
    Undrain {
        /// Edge name or id
        edge: String,
    },
    /// Delete a drained edge that owns no app
    #[command(visible_alias = "rm")]
    Delete {
        /// Edge name or id
        edge: String,
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Which edge owns each app's host name
    Routes(ListArgs),
}

pub fn edges(app: &mut App, cmd: EdgesCmd) -> Result<()> {
    let edge_id = |app: &App, e: &str| -> Result<String> {
        id_of(&find_named(app, "/edges", e, "edge")?).context("edge record has no id")
    };
    match cmd {
        EdgesCmd::List(l) => {
            let want = l.want(&app.out);
            let (items, more) = app.collect("/edges", vec![], "before", Some(100), want)?;
            app.out.list(
                &items,
                &["id", "name", "endpoint", "draining", "last_seen"],
                "edges",
            );
            l.more_hint(more, items.len());
        }
        EdgesCmd::Register {
            name,
            cert,
            pin,
            endpoint,
        } => {
            let body =
                json!({"name": name, "cert_sha256": pin_from(&cert, &pin)?, "endpoint": endpoint});
            let v = app.client.post("/edges", body)?;
            done(app, &v, &format!("Registered edge {name}"));
        }
        EdgesCmd::Drain { edge } => {
            let id = edge_id(app, &edge)?;
            let v = app
                .client
                .patch(&format!("/edges/{}", seg(&id)), json!({"draining": true}))?;
            done(app, &v, &format!("Draining edge {edge}"));
        }
        EdgesCmd::Undrain { edge } => {
            let id = edge_id(app, &edge)?;
            let v = app
                .client
                .patch(&format!("/edges/{}", seg(&id)), json!({"draining": false}))?;
            done(app, &v, &format!("Edge {edge} takes apps again"));
        }
        EdgesCmd::Delete { edge, yes } => {
            let id = edge_id(app, &edge)?;
            confirm(&format!("Delete edge {edge}?"), yes)?;
            let v = app.client.delete(&format!("/edges/{}", seg(&id)))?;
            done(app, &v, &format!("Deleted edge {edge}"));
        }
        EdgesCmd::Routes(l) => {
            let want = l.want(&app.out);
            let (items, more) = app.collect("/edge-routes", vec![], "before", Some(100), want)?;
            app.out.list(&items, &[], "routes");
            l.more_hint(more, items.len());
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- egress policy

#[derive(Subcommand, Debug)]
pub enum EgressCmd {
    /// Show the public egress policy
    Get,
    /// Turn the public egress policy on
    Enable {
        /// The policy digest to activate (as `get` shows it)
        #[arg(long)]
        digest: Option<String>,
    },
    /// Turn the public egress policy off
    Disable,
}

pub fn egress(app: &mut App, cmd: EgressCmd) -> Result<()> {
    let path = "/public-egress-policy";
    let (enabled, digest) = match cmd {
        EgressCmd::Get => {
            let v = app.client.get(path, &[])?;
            app.out.record(&v);
            return Ok(());
        }
        EgressCmd::Enable { digest } => (true, digest),
        EgressCmd::Disable => (false, None),
    };
    let cur = app.client.get(path, &[])?;
    let rev = crate::app::revision_of(&cur).unwrap_or(0);
    let mut body = json!({"expected_revision": rev, "enabled": enabled});
    if let Some(d) = digest {
        body["digest"] = json!(d);
    }
    let v = app.client.put(path, body)?;
    done(
        app,
        &v,
        if enabled {
            "Public egress policy on"
        } else {
            "Public egress policy off"
        },
    );
    Ok(())
}

// ---------------------------------------------------------------- status, audit

/// `/healthz` and `/readyz` live at the API's origin, outside `/v1`.
fn origin(api_url: &str) -> Result<String> {
    let u = reqwest::Url::parse(api_url)?;
    Ok(u.origin().ascii_serialization())
}

pub fn status(app: &mut App) -> Result<()> {
    let base = origin(&app.api_url)?;
    let probe = |p: &str| -> Value {
        match app.client.get(&format!("{base}{p}"), &[]) {
            Ok(v) => v.get("status").cloned().unwrap_or(v),
            Err(e) => match e.downcast_ref::<crate::client::ApiError>() {
                Some(a) => json!(format!("HTTP {}", a.status)),
                None => json!(format!("unreachable: {e}")),
            },
        }
    };
    let health = probe("/healthz");
    let ready = probe("/readyz");
    let loops = app.client.get("/control-plane/status", &[]);
    if app.out.json {
        let (l, err) = match &loops {
            Ok(v) => (v.clone(), Value::Null),
            Err(e) => (Value::Null, json!(e.to_string())),
        };
        app.out.print_json(&json!({"api_url": app.api_url, "healthz": health, "readyz": ready, "control_plane": l, "error": err}));
        return loops.map(|_| ());
    }
    let row = |k: &str, v: String| println!("  {}  {v}", ui::dim(&format!("{k:<8}"), To::Out));
    let show = |v: &Value| crate::output::scalar(v);
    row("api", app.api_url.clone());
    row("healthz", ui::state(&show(&health), To::Out));
    row("readyz", ui::state(&show(&ready), To::Out));
    println!();
    app.out.record(&loops?);
    Ok(())
}

#[derive(Args, Debug)]
pub struct AuditArgs {
    /// Tenant name or id (default: the current workspace)
    pub tenant: Option<String>,
    /// Only this action
    #[arg(long)]
    pub action: Option<String>,
    /// Only changes by keys with this label
    #[arg(long)]
    pub actor_label: Option<String>,
    /// From this time (RFC 3339)
    #[arg(long)]
    pub since: Option<String>,
    /// Until this time (RFC 3339)
    #[arg(long)]
    pub until: Option<String>,
    #[command(flatten)]
    pub list: ListArgs,
}

pub fn audit(app: &mut App, a: AuditArgs) -> Result<()> {
    let path = match &a.tenant {
        Some(t) => tenant_path(app, t)?.0,
        None => app.ws_path()?,
    };
    let mut q = Vec::new();
    for (k, v) in [
        ("action", &a.action),
        ("actor_label", &a.actor_label),
        ("since", &a.since),
        ("until", &a.until),
    ] {
        if let Some(v) = v {
            q.push((k.to_string(), v.clone()));
        }
    }
    let want = a.list.want(&app.out);
    let (items, more) = app.collect(&format!("{path}/audit"), q, "before", Some(100), want)?;
    app.out.list(
        &items,
        &["created", "actor", "actor_label", "action", "resource_id"],
        "audit records",
    );
    a.list.more_hint(more, items.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_durations_parse() {
        assert_eq!(parse_bytes("1GiB").unwrap(), 1 << 30);
        assert_eq!(parse_bytes("10GB").unwrap(), 10_000_000_000);
        assert_eq!(parse_bytes("2048").unwrap(), 2048);
        assert_eq!(parse_mib("1GiB").unwrap(), 1024);
        assert_eq!(parse_mib("512").unwrap(), 512);
        assert!(parse_bytes("12 parsecs").is_err());
        assert_eq!(parse_duration("30d").unwrap(), 2_592_000);
        assert_eq!(parse_duration("15m").unwrap(), 900);
        assert_eq!(parse_duration("90").unwrap(), 90);
        assert!(parse_duration("soon").is_err());
    }

    #[test]
    fn allowance_flags_override_the_base_whole() {
        let a = AllowanceArgs {
            cpu_percent: Some(300),
            memory: Some("2GiB".into()),
            ..Default::default()
        };
        let v = a.over(&default_allowance()).unwrap();
        assert_eq!(v["cpu_percent"], 300);
        assert_eq!(v["memory_mib"], 2048);
        assert_eq!(v["instances"], 2);
        assert_eq!(v["storage_bytes"], 1u64 << 30);
    }

    #[test]
    fn issue_body_expands_all_and_checks_permissions() {
        let b = issue_body("k", &["all".into()], 3600, Some("p"), Some("u:1")).unwrap();
        assert_eq!(
            b["permissions"].as_array().unwrap().len(),
            PERMISSIONS.len()
        );
        assert_eq!(b["project_id"], "p");
        assert_eq!(b["label"], "u:1");
        assert!(issue_body("k", &["root".into()], 3600, None, None).is_err());
        assert!(issue_body("k", &["read".into()], 10, None, None).is_err());
    }

    #[test]
    fn pin_is_sha256_of_der() {
        // "hello" as a fake DER body.
        let pem = "-----BEGIN CERTIFICATE-----\naGVsbG8=\n-----END CERTIFICATE-----\n";
        assert_eq!(
            cert_pin(pem).unwrap(),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }
}
