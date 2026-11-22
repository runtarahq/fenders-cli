<p align="center">
  <a href="https://fenders.io">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="assets/logo-dark.svg">
      <img src="assets/logo-light.svg" alt="fenders.io" width="320">
    </picture>
  </a>
</p>

# fenders-cli

`fenders` is the command line for a self-hosted [Fenders](https://github.com/runtarahq/fenders)
control plane. With the operator's credential it manages tenants, their keys, nodes and edges.
With a tenant's key it deploys and runs WebAssembly (WASI) components in hardware-isolated
microVM sandboxes. It talks to the control plane's management API, whose contract the control
plane serves at `/api/openapi.json`.

It ships as a single native binary with no runtime dependencies.

## Install

macOS and Linux (x86_64 and arm64):

```sh
curl -fsSL https://raw.githubusercontent.com/runtarahq/fenders-cli/main/install.sh | sh
```

The script picks the right binary for your machine, verifies its SHA-256 checksum, and installs
it to `~/.local/bin` (or wherever `fenders` already is). Options go after `sh -s --`:

```sh
curl -fsSL …/install.sh | sh -s -- --version v0.3.0   # a specific release
curl -fsSL …/install.sh | sh -s -- --dir /usr/local/bin  # another directory (uses sudo if needed)
curl -fsSL …/install.sh | sh -s -- --uninstall        # remove (add --purge to drop ~/.config/fenders)
```

**Update** with `fenders update` (`--check` only reports), or re-run the install command.
On Windows, download the `.zip` from [Releases](https://github.com/runtarahq/fenders-cli/releases).
From source: `cargo install --git https://github.com/runtarahq/fenders-cli`.

## Quick start

At first start the control plane writes the operator credential to `STATE/operator.token`, and
`fenders-pki` made the CA that signed its certificate. Those two files are all you need:

```sh
# Connect as the operator.
fenders profile add lab --url https://control.internal:9090 \
    --ca-cert pki/ca.pem --token-file /var/lib/fenders/control/operator.token
fenders status                                    # healthz, readyz and the control plane's loops

# A tenant for a customer, and a key for it saved as a second profile.
fenders tenants create acme --cpu-percent 200 --memory 1GiB --instances 4 --transfer 10GiB
fenders keys issue acme --label 'team:acme' --save-profile acme

# Act as that tenant.
fenders -P acme deploy hello.wasm --app hello     # upload → inspect → deploy → release
fenders -P acme run hello --logs                  # start a run and print its output
fenders -P acme logs hello -f                     # follow its output
fenders -P acme audit                             # what was changed, and by which key
```

## Commands

| Area | Commands |
|---|---|
| Profiles | `profile add \| list \| use \| show \| remove`, `auth login \| logout \| status \| token`, `use [WORKSPACE[/PROJECT]]`, `context` |
| Operator | `tenants list \| get \| create \| set \| http-limits \| suspend \| resume \| delete`, `keys issue \| list \| revoke`, `nodes list \| get \| register \| set \| drain \| undrain \| disable \| enable \| delete`, `edges list \| register \| drain \| undrain \| delete \| routes`, `egress-policy get \| enable \| disable` |
| Either | `status`, `audit [TENANT]` |
| Ship | `deploy FILE.wasm`, `run APP`, `logs APP [-f]` |
| Apps | `apps deploy \| list \| get \| create \| start [--release] \| stop \| url \| connections \| set-connections` |
| App history | `versions …`, `releases …`, `runs …`, `events APP`, `triggers get \| set` |
| Project | `projects list \| create`, `artifacts list \| get \| inspect \| upload`, `connections list \| get \| create https\|tcp \| revoke`, `secrets list \| get \| create`, `ops list \| get \| wait`, `usage`, `sizes`, `https-access show \| validate`, `schedule preview` |
| Anything else | `api PATH [-X METHOD] [-d JSON\|@file]` with `{workspace}`/`{project}` placeholders |
| Shell | `completion bash\|zsh\|fish\|powershell\|elvish`, `update [--check]` |

Run `fenders <command> --help` for flags and examples. Operator commands need an operator
profile; with a tenant key the control plane answers 403 and the CLI says so.

## How it behaves

- **Profiles.** A profile is one control plane (URL and CA) and one credential. Whether the
  credential is the operator's or a tenant key's is read from `GET /v1/me` when the profile is
  added; a tenant key's profile starts in the workspace (and project) it is scoped to. Pick one
  per command with `-P NAME` or `FENDERS_PROFILE`, or switch with `fenders profile use`.
- **Private CA.** The control plane's certificate is signed by the CA `fenders-pki` made, not a
  public one. Give it once with `--ca-cert` when adding the profile.
- **Credentials stay with their profile.** `--api-url` overrides the profile's URL for one
  command, but then only `FENDERS_TOKEN` is sent: a stored credential never goes to another host.
- **Names or ids.** Tenants, projects, apps, nodes, edges, connections and secrets can be given
  by name; the CLI turns names into the API's 32-character ids.
- **Waiting.** Deploys, releases, runs, lifecycle changes and suspensions are background
  operations. The CLI waits for them with a spinner by default. `--no-wait` prints the operation
  instead; pick it up later with `fenders ops wait ID`.
- **Safe retries.** Each invocation sends one `Idempotency-Key`, reused when it retries on
  network errors, 429 or 5xx responses, so a retried deploy never runs twice.
- **Revisions.** Mutations that need `expected_revision` read the current revision
  automatically. Pass `--expected-revision N` to pin one.
- **Output.** Tables and colour on a terminal; JSON when piped, or with `--json` / `-o json`.
  `fenders logs` writes one JSON record per line, with base64 output decoded into `text`.
  `NO_COLOR` and `--no-color` are honoured. Progress and errors go to stderr.
- **Untrusted text.** App log output and API strings have terminal control characters
  escaped before printing, so they can't inject escape sequences into your terminal.
- **Secrets.** Values are never accepted as flags: they're read from stdin, `--from-file`,
  or a hidden prompt. A new key's token is printed once, alone on stdout.
- **Errors.** API errors show the message, a hint keyed to the control plane's error `code`,
  and the `request_id`.
- **Scripting.** `--no-input` disables prompts, and exit status is non-zero on any failure
  (including a failed operation).

## Configuration

| Setting | Flag | Environment | Stored in |
|---|---|---|---|
| Profile | `-P` / `--profile` | `FENDERS_PROFILE` | `config.toml` (`current`) |
| Credential | — | `FENDERS_TOKEN` (or `FENDERS_API_KEY`) | macOS Keychain / Windows Credential Manager; elsewhere `credentials.toml` (mode 0600) |
| API URL | `--api-url` | `FENDERS_API_URL` | `config.toml` (`profiles.NAME.api_url`) |
| CA certificate | `--ca-cert` | `FENDERS_CA_CERT` | `config.toml` (`profiles.NAME.ca_cert`, a path) |
| Workspace / project | `-w` / `-p` | `FENDERS_WORKSPACE` / `FENDERS_PROJECT` | `config.toml` (per profile) |
| Config directory | — | `FENDERS_CONFIG_DIR` | default `~/.config/fenders` (`%APPDATA%\fenders` on Windows) |

A `config.toml` from before profiles becomes a profile named `default`.

## Building

```sh
cargo build --release          # → target/release/fenders (~4 MB, stripped, LTO)
cargo test                     # unit tests + HTTP behaviour against a mock server
```

Release binaries are built natively per platform by `.github/workflows/release.yml` when a
`v*` tag is pushed: macOS (arm64, x86_64), Linux (static musl, x86_64 and arm64) and Windows,
published with a `SHA256SUMS` file that `install.sh` verifies. TLS uses rustls, so there is
no OpenSSL dependency. CI uses only `actions/*`, `actions-rust-lang/setup-rust-toolchain` and
`Swatinem/rust-cache`, each pinned to a commit SHA.

To cut a release, bump `version` in `Cargo.toml`, then `git tag v0.1.0 && git push --tags`.

## Layout

```
src/
  main.rs        command tree (clap), global flags, error reporting
  client.rs      HTTP client: auth, private CA, idempotency, retries, error mapping, wasm upload
  app.rs         context resolution, name → id lookup, pagination, operation polling
  output.rs      tables / records / JSON rendering of untyped API records
  ui.rs          brand palette, banner, spinners, prompts theme, output sanitising
  auth.rs        credential storage per profile (keychain or 0600 file)
  config.rs      profiles (URL, CA, role, workspace/project) in config.toml
  cmd/           one module per command group (profile.rs, operator.rs, …)
install.sh       installer / updater / uninstaller for macOS and Linux
```
