---
name: fenders
description: Deploy WebAssembly (.wasm) web apps and run untrusted or generated code in hardware-isolated sandboxes on a Fenders control plane, with the `fenders` CLI. Use when the user wants to deploy, host, run, test or debug a .wasm/WASI component or HTTP app on Fenders, get a URL for one, run code in a sandbox (including through a JS or Python runner app), or check its logs, runs, secrets or usage.
---

# Fenders

Fenders runs WebAssembly components, each app in a microVM of its own with no network access
unless it is given some. The `fenders` CLI talks to a Fenders control plane. Two things it does
for you:

1. **Host a web app.** Build an HTTP component (`.wasm`), `fenders deploy` it, get a URL.
2. **Run code in a sandbox.** Send code to a runner app (an interpreter deployed as an HTTP
   app), or build the code into a command component and run it once. See
   [reference/sandbox.md](reference/sandbox.md).

## Before anything: check where you are

```bash
fenders context -o json
```

- Fails with `no profile selected`: stop and ask the user for a profile. They add one with
  `fenders profile add NAME --url https://HOST:9090 --ca-cert ca.pem` and a tenant key on
  stdin. Never guess a URL or look for tokens on disk.
- `"role": "operator"`: do not deploy with it. The operator credential controls every
  tenant. Ask the user for a tenant profile (the operator makes one with
  `fenders keys issue TENANT --permissions read,apps:write,runs:write --ttl 1d --save-profile NAME`),
  then use `-P NAME` on every command.
- `"role": "tenant"` with a `workspace` and `project`: go ahead. Without a project, ask which
  one (`fenders projects list -o json`) and pass `-p PROJECT`.

## Rules

- Pass `--no-input -o json` on every command, read stdout as JSON, and check the exit status.
  Progress and errors go to stderr. Never parse the table output.
- Never print, echo, log or put in a file a token (`fnd_tok_…`) or a secret value. Secret
  values go on stdin only: `printf %s "$VALUE" | fenders secrets create NAME --no-input`.
- Changes wait for their operation by default. Keep it that way, so the command's exit status
  tells you whether the change worked.
- Do not change triggers, connections, allowances or keys the user did not ask about. Do not
  attach a connection (outbound network access) to code you are running as a sandbox unless
  the user asked for that access.
- Delete what you made only for your own testing (`fenders apps delete NAME --yes --no-input`),
  and say what you left running.
- On an error, read its `code` and the hint under it; [reference/errors.md](reference/errors.md)
  says what each means. Do not retry a 4xx unchanged.

## Deploy a web app

1. Build an HTTP component. Rust: copy [templates/http-rust](templates/http-rust), put the app
   in `respond`, then:

   ```bash
   cargo build --release --target wasm32-wasip2
   ```

   Other languages and what a component must export: [reference/web-apps.md](reference/web-apps.md).
2. Deploy it. A new name creates the app; `--app NAME` releases new code to an existing one:

   ```bash
   fenders deploy target/wasm32-wasip2/release/fenders_http_app.wasm --name my-app --no-input -o json
   fenders deploy target/wasm32-wasip2/release/fenders_http_app.wasm --app my-app --no-input -o json
   ```

   It uploads, waits for the control plane to inspect the component, and releases it. An
   inspection failure (`not_validated`) means the file is not a component Fenders can run;
   fix the build, do not retry.
3. Get its address and test it:

   ```bash
   fenders apps url my-app --no-input -o json | jq -r .url
   curl -s "$URL/"
   ```

4. Debug with its output and request history (log lines arrive a few seconds late; ignore lines
   starting `FENDERS_`, which the runtime writes):

   ```bash
   fenders logs my-app --no-input -o json | jq -r .text
   fenders events my-app --no-input -o json
   ```

Configuration: `-e KEY=VALUE` and `--secret-env KEY=SECRET` on `deploy`, or on
`fenders releases create APP` to change them without new code.

## Run a command once

A component with a `main` (a command) runs on demand. A run takes no arguments or stdin: pass
data in environment variables.

```bash
fenders deploy job.wasm --name job -e INPUT=42 --no-input -o json
fenders run job --no-input -o json | jq '{state, exit_code}'
fenders logs job --no-input -o json | jq -r .text
```

`state` is `succeeded` or `failed`; `exit_code` is the program's. Template:
[templates/command-rust](templates/command-rust).

## More

- [reference/sandbox.md](reference/sandbox.md): running code you do not trust, with a runner
  app or as a command, and exactly what is isolated.
- [reference/web-apps.md](reference/web-apps.md): building components, HTTP details, sizes,
  secrets, schedules, outbound connections.
- [reference/cli.md](reference/cli.md): the commands you will use, with the JSON they return.
- [reference/errors.md](reference/errors.md): error codes and what to do.
