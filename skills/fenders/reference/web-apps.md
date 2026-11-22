# Building and running web apps on Fenders

## What Fenders runs

A **WebAssembly component** (WASI 0.2, the `wasm32-wasip2` target), not a core module and not a
native binary. When it is uploaded, the control plane inspects it and says what it is:

- **HTTP handler**: exports `wasi:http/incoming-handler`. It gets an address and answers
  requests. `deploy` prints `Component validated: HTTP handler`.
- **Command**: has a `main` (`wasi:cli/run`). It runs on demand or on a schedule.
  `deploy` prints `Component validated: command`.

A file that is neither is refused with `not_validated`; rebuild it, do not retry.

## Building

### Rust (verified)

Start from `templates/http-rust` (or `templates/command-rust`). Needs the target once:

```bash
rustup target add wasm32-wasip2
cargo build --release --target wasm32-wasip2
ls target/wasm32-wasip2/release/*.wasm
```

The template uses the `wasi` crate (`=0.14.4`), the same bindings as Fenders' own examples.
Its handler reads the method, path and body and answers with text; put the app in `respond`.
Bodies of hundreds of kilobytes work both ways. Writes go out in chunks of at most 4096 bytes
(`blocking_write_and_flush`'s limit), which the template's `send` already does.

### Other languages

Any toolchain that produces a WASI 0.2 component exporting `wasi:http/incoming-handler` (or a
command) works, for example `jco componentize` for JavaScript or `componentize-py` for Python.
These are not set up by this skill: check the toolchain is installed, build, and let
`fenders deploy` tell you whether the result is a component Fenders accepts.

## Deploying

```bash
fenders deploy app.wasm --name my-app --no-input -o json            # a new app
fenders deploy app.wasm --app my-app --no-input -o json             # new code for an existing one
```

The JSON has `app` (its record: `id`, `name`, `state`, `desired`, `latest_version`, …) and
`http` (its address). `state: "ready"` means it is serving.

Flags that matter:

| Flag | Use |
|---|---|
| `-e KEY=VALUE` | Environment variable, repeatable |
| `--secret-env KEY=SECRET` | Expose a project secret as an environment variable |
| `--size small\|medium\|large` | Memory and CPU (`fenders sizes`); default `small` |
| `--http` / `--no-http` | Expose over HTTP (default: when it handles HTTP) |
| `--manual` / `--no-manual` | Allow `fenders run` (default: when it is a command) |
| `--schedule CRON --timezone TZ` | Run a command on a schedule |
| `--connection NAME[:PREFIX]` | Allow an outbound connection (see below) |

`fenders apps deploy NAME --artifact FILE` asks before releasing to an existing app, so do not
use it without a terminal; `fenders deploy FILE --app NAME` is the scriptable way.

## Calling it

```bash
URL=$(fenders apps url my-app --no-input -o json | jq -r .url)
curl -s "$URL/path"
```

The address is `https://app-<app id>.<domain>`, served by the control plane's gateway. On a
local control plane the gateway may use a certificate that does not name app hosts and a domain
that does not resolve; then:

```bash
HOST=${URL#https://}
curl -sk --resolve "$HOST:GATEWAY_PORT:127.0.0.1" "https://$HOST:GATEWAY_PORT/path"
```

Ask the user for the gateway port if the URL does not work as is.

- Each request runs in a fresh instance: keep no state in memory between requests.
- Output the handler prints goes to the app's logs, a few seconds later.
- Requests are recorded as the app's **events** (`method`, `path`, `http_status`, `state`,
  `duration_ms`), not as runs.
- The tenant's HTTP limits (requests at once, per minute) apply; beyond them requests are
  refused. The operator sets them.

## Configuration and secrets

```bash
printf %s "$VALUE" | fenders secrets create db-password --no-input -o json
fenders releases create my-app -e MODE=prod --secret-env DB_PASSWORD=db-password --no-input -o json
```

A release is code plus configuration; `releases create` makes one from the latest version
with new configuration, `--version N` goes back to an older version. Secret values are never
shown again and never belong on a command line.

## Schedules

```bash
fenders schedule preview '0 6 * * *' --timezone Europe/Kyiv --no-input -o json
fenders triggers set my-job --schedule '0 6 * * *' --timezone Europe/Kyiv --no-input -o json
fenders triggers set my-job --no-schedule --no-input -o json
```

Each run is given the time it was triggered for in `FENDERS_TRIGGER_TIME`.

## Outbound connections

Nothing leaves an app unless a connection is attached. Only do this when the user asks for the
app to reach that destination.

```bash
fenders connections create https github --origin https://api.github.com --no-input -o json
fenders connections create tcp postgres --host db.example.com --port 5432 --no-input -o json
fenders apps set-connections my-app github postgres:PG --no-input -o json
fenders apps connections my-app --no-input -o json
```

HTTPS origins also need the operator's public egress policy on (`fenders https-access show`
says `available: yes`); otherwise attaching one fails with `Public HTTPS egress is not
configured`, which only the operator can change.

## Lifecycle

```bash
fenders apps stop my-app --no-input -o json
fenders apps start my-app --no-input -o json
fenders apps delete my-app --yes --no-input -o json
```
