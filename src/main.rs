#[cfg(test)]
mod api_tests;
mod app;
mod auth;
mod client;
mod cmd;
mod config;
mod output;
mod ui;

use std::process::ExitCode;

use anyhow::Result;
use clap::builder::styling::{AnsiColor, Color, RgbColor, Style, Styles};
use clap::{Args, CommandFactory, Parser, Subcommand};

use crate::cmd::apps::{
    AppsCmd, EventsArgs, ReleasesCmd, RunArgs, RunsCmd, TriggersCmd, VersionsCmd,
};
use crate::cmd::deploy::DeployArgs;
use crate::cmd::logs::LogsArgs;
use crate::cmd::project::{
    ArtifactsCmd, ConnectionsCmd, HttpsCmd, OpsCmd, ProjectsCmd, ScheduleCmd, SecretsCmd, UsageArgs,
};
use crate::cmd::session::{AuthCmd, UseArgs};
use crate::output::Format;

fn styles() -> Styles {
    let orange = Style::new().fg_color(Some(Color::Rgb(RgbColor(
        ui::ORANGE.0,
        ui::ORANGE.1,
        ui::ORANGE.2,
    ))));
    Styles::styled()
        .header(orange.bold())
        .usage(orange.bold())
        .literal(Style::new().bold())
        .placeholder(Style::new().fg_color(Some(Color::Rgb(RgbColor(
            ui::MUTED.0,
            ui::MUTED.1,
            ui::MUTED.2,
        )))))
        .error(Style::new().fg_color(Some(AnsiColor::Red.into())).bold())
        .valid(Style::new().fg_color(Some(AnsiColor::Green.into())))
        .invalid(Style::new().fg_color(Some(AnsiColor::Yellow.into())))
}

const AFTER_HELP: &str = "\
Get started (as the operator of a control plane):
  fenders profile add lab --url https://HOST:9090 --ca-cert ca.pem \\
      --token-file STATE/operator.token       Connect with the operator token
  fenders tenants create acme                 Create a tenant (a workspace)
  fenders keys issue acme --save-profile acme Give it a key, saved as a profile
  fenders -P acme deploy app.wasm --app hello Deploy a component as that tenant
  fenders -P acme logs hello -f               Follow its output

Docs: https://fenders.io";

#[derive(Parser, Debug)]
#[command(
    name = "fenders",
    version,
    about = "Deploy and run WebAssembly apps on Fenders — hardware-isolated microVM sandboxes.",
    styles = styles(),
    after_help = AFTER_HELP,
    disable_help_subcommand = true
)]
struct Cli {
    #[command(flatten)]
    global: Global,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Args, Debug)]
struct Global {
    /// Workspace (name or id) for this command
    #[arg(
        short = 'w',
        long,
        global = true,
        env = "FENDERS_WORKSPACE",
        hide_env_values = true
    )]
    workspace: Option<String>,
    /// Project (name or id) for this command
    #[arg(
        short = 'p',
        long,
        global = true,
        env = "FENDERS_PROJECT",
        hide_env_values = true
    )]
    project: Option<String>,
    /// Profile to use for this command (see `fenders profile list`)
    #[arg(
        short = 'P',
        long,
        global = true,
        env = "FENDERS_PROFILE",
        hide_env_values = true,
        value_name = "NAME"
    )]
    profile: Option<String>,
    /// API base URL, instead of the profile's (stored credentials are not sent to it; use FENDERS_TOKEN)
    #[arg(
        long,
        global = true,
        env = "FENDERS_API_URL",
        hide_env_values = true,
        value_name = "URL"
    )]
    api_url: Option<String>,
    /// CA certificate (PEM) to trust, instead of the profile's
    #[arg(
        long,
        global = true,
        env = "FENDERS_CA_CERT",
        hide_env_values = true,
        value_name = "PEM"
    )]
    ca_cert: Option<std::path::PathBuf>,
    /// Output format
    #[arg(short = 'o', long, global = true, value_enum, default_value = "auto")]
    output: Format,
    /// Shorthand for --output json
    #[arg(long, global = true)]
    json: bool,
    /// Disable colours (also honours NO_COLOR)
    #[arg(long, global = true)]
    no_color: bool,
    /// Never prompt; fail instead
    #[arg(long, global = true)]
    no_input: bool,
    /// Log HTTP requests to stderr
    #[arg(short, long, global = true)]
    verbose: bool,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Manage profiles: control planes and the credentials for them
    #[command(subcommand)]
    Profile(cmd::profile::ProfileCmd),
    /// Replace, remove or inspect the current profile's credential
    #[command(subcommand)]
    Auth(AuthCmd),
    /// Choose the workspace/project that commands apply to
    Use(UseArgs),
    /// Show the current profile, workspace, project and auth
    Context,

    /// Upload and deploy a WASI component in one step
    Deploy(Box<DeployArgs>),
    /// Start a run of an app and wait for it
    Run(RunArgs),
    /// Show an app's output
    Logs(LogsArgs),

    /// Manage apps
    #[command(subcommand)]
    Apps(AppsCmd),
    /// Manage app versions
    #[command(subcommand)]
    Versions(VersionsCmd),
    /// Manage app releases
    #[command(subcommand)]
    Releases(ReleasesCmd),
    /// Inspect app runs
    #[command(subcommand)]
    Runs(RunsCmd),
    /// Show or change how an app is triggered
    #[command(subcommand)]
    Triggers(TriggersCmd),
    /// Show an app's events
    Events(EventsArgs),

    /// [operator] Create, change, suspend and delete tenants (workspaces)
    #[command(subcommand, visible_alias = "workspaces")]
    Tenants(cmd::operator::TenantsCmd),
    /// [operator] Issue, list and revoke tenants' API keys
    #[command(subcommand)]
    Keys(cmd::operator::KeysCmd),
    /// [operator] Register, drain and remove nodes
    #[command(subcommand)]
    Nodes(cmd::operator::NodesCmd),
    /// [operator] Register, drain and remove edges; show their routes
    #[command(subcommand)]
    Edges(cmd::operator::EdgesCmd),
    /// [operator] Show or switch the public egress policy
    #[command(subcommand, name = "egress-policy")]
    EgressPolicy(cmd::operator::EgressCmd),
    /// Whether the control plane is up, and its loops (the latter for the operator)
    Status,
    /// What was changed in a workspace, by whom
    Audit(cmd::operator::AuditArgs),
    /// List and create projects
    #[command(subcommand)]
    Projects(ProjectsCmd),
    /// Manage uploaded WASI components
    #[command(subcommand)]
    Artifacts(ArtifactsCmd),
    /// Manage external connections (HTTPS origins, TCP endpoints)
    #[command(subcommand)]
    Connections(ConnectionsCmd),
    /// Manage encrypted secrets
    #[command(subcommand)]
    Secrets(SecretsCmd),
    /// Inspect and wait for background operations
    #[command(subcommand)]
    Ops(OpsCmd),
    /// Show usage for the project or workspace
    Usage(UsageArgs),
    /// Show available app sizes and defaults
    Sizes,
    /// Show or validate the outbound HTTPS policy
    #[command(subcommand, name = "https-access")]
    HttpsAccess(HttpsCmd),
    /// Preview cron schedules
    #[command(subcommand)]
    Schedule(ScheduleCmd),

    /// Make an authenticated request to any API endpoint
    Api(cmd::api::ApiArgs),
    /// Update fenders to the latest release
    #[command(visible_alias = "upgrade")]
    Update(cmd::update::UpdateArgs),
    /// Print a shell completion script
    Completion {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

fn main() -> ExitCode {
    // Exit quietly when stdout is closed early (`fenders apps list | head`)
    // instead of panicking on a broken pipe.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let cli = Cli::parse();
    let g = &cli.global;
    ui::init(g.no_color, g.no_input);
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            report(&e);
            ExitCode::FAILURE
        }
    }
}

fn report(e: &anyhow::Error) {
    if let Some(api) = e.downcast_ref::<client::ApiError>() {
        ui::error(&api.to_string());
        if let Some(h) = api.hint() {
            ui::hint(&h);
        }
        if let Some(r) = &api.request_id {
            ui::hint(&format!("request id: {}", ui::sanitize(r)));
        }
        return;
    }
    if let Some(h) = e.downcast_ref::<app::Hinted>() {
        ui::error(&ui::sanitize(&h.message));
        for hint in &h.hints {
            ui::hint(hint);
        }
        return;
    }
    ui::error(&ui::sanitize(&e.to_string()));
    for cause in e.chain().skip(1) {
        ui::hint(&format!("caused by: {}", ui::sanitize(&cause.to_string())));
    }
}

fn run(cli: Cli) -> Result<()> {
    let Some(command) = cli.cmd else {
        eprint!("{}", ui::banner());
        eprintln!();
        Cli::command().print_help()?;
        return Ok(());
    };
    if let Cmd::Completion { shell } = command {
        clap_complete::generate(
            shell,
            &mut Cli::command(),
            "fenders",
            &mut std::io::stdout(),
        );
        return Ok(());
    }

    if let Cmd::Update(a) = command {
        let json = cli.global.json || cli.global.output == Format::Json;
        return cmd::update::run(a, json);
    }

    let g = cli.global;
    let mut cfg = config::Config::load()?;
    let format = if g.json { Format::Json } else { g.output };
    if let Cmd::Profile(c) = command {
        return cmd::profile::run(cfg, &output::Out::new(format), c, g.verbose);
    }
    cfg.activate(g.profile.as_deref())?;
    let conn = cmd::profile::connect(&cfg, g.api_url.as_deref(), g.ca_cert.as_deref(), g.verbose)?;
    let mut app = app::App::new(
        conn.client,
        output::Out::new(format),
        cfg,
        conn.api_url,
        g.workspace,
        g.project,
    );

    use cmd::*;
    match command {
        Cmd::Auth(c) => session::auth(&mut app, c),
        Cmd::Use(a) => session::use_(&mut app, a),
        Cmd::Context => session::context(&mut app),
        Cmd::Deploy(a) => deploy::run(&mut app, *a),
        Cmd::Run(a) => apps::run(&mut app, a),
        Cmd::Logs(a) => logs::run(&mut app, a),
        Cmd::Apps(c) => apps::apps(&mut app, c),
        Cmd::Versions(c) => apps::versions(&mut app, c),
        Cmd::Releases(c) => apps::releases(&mut app, c),
        Cmd::Runs(c) => apps::runs(&mut app, c),
        Cmd::Triggers(c) => apps::triggers(&mut app, c),
        Cmd::Events(a) => apps::events(&mut app, a),
        Cmd::Tenants(c) => operator::tenants(&mut app, c),
        Cmd::Keys(c) => operator::keys(&mut app, c),
        Cmd::Nodes(c) => operator::nodes(&mut app, c),
        Cmd::Edges(c) => operator::edges(&mut app, c),
        Cmd::EgressPolicy(c) => operator::egress(&mut app, c),
        Cmd::Status => operator::status(&mut app),
        Cmd::Audit(a) => operator::audit(&mut app, a),
        Cmd::Projects(c) => project::projects(&mut app, c),
        Cmd::Artifacts(c) => project::artifacts(&mut app, c),
        Cmd::Connections(c) => project::connections(&mut app, c),
        Cmd::Secrets(c) => project::secrets(&mut app, c),
        Cmd::Ops(c) => project::ops(&mut app, c),
        Cmd::Usage(a) => project::usage(&mut app, a),
        Cmd::Sizes => project::sizes(&mut app),
        Cmd::HttpsAccess(c) => project::https(&mut app, c),
        Cmd::Schedule(c) => project::schedule(&mut app, c),
        Cmd::Api(a) => api::run(&mut app, a),
        Cmd::Completion { .. } | Cmd::Update(_) | Cmd::Profile(_) => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn cli_is_well_formed() {
        use clap::CommandFactory;
        super::Cli::command().debug_assert();
    }
}
