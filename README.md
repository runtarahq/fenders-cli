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
fenders -P acme deploy hello.wasm --name hello    # upload → inspect → deploy → release
fenders -P acme run hello --logs                  # start a run and print its output
fenders -P acme logs hello -f                     # follow its output
fenders -P acme audit                             # what was changed, and by which key
```

The [guide](#guide) below goes through each of these, and the rest, with real output.

## Guide

This walks through everything the CLI does, first as the operator of a control plane, then as a
tenant. Every command and every output below was run against a local control plane at
`https://127.0.0.1:29090` with one node, started as in
[the Fenders embedding guide](https://github.com/runtarahq/fenders/blob/main/docs/EMBEDDING-API.md);
put your own address in its place. Ids and times will differ on yours, and long outputs are
trimmed here (`...`, or fewer columns).

### Words

| Word | What it is |
|---|---|
| **Control plane** | `fenders-control-plane`: the API this CLI talks to. It holds every tenant, app and run. |
| **Operator** | Whoever runs the control plane. Its credential, `STATE/operator.token`, may do everything. |
| **Tenant** | One customer of the control plane, with an allowance (CPU, memory, storage, instances, transfer). The API calls it a **workspace**; `fenders tenants` and `fenders workspaces` are the same command. |
| **Project** | A part of a tenant's workspace. Every workspace starts with one, `Production`. Apps, artifacts, secrets and connections belong to a project. |
| **Key** | A tenant's API key (`fnd_tok_…`), issued by the operator, scoped to the workspace or one project, with permissions and an expiry of at most 30 days. |
| **Profile** | This CLI's name for one control plane (URL and CA) and one credential (the operator token or a key). |
| **Node** | A KVM machine that runs the code. **Edge**: a control-plane process that serves HTTP apps, when there is more than one. |

### 1. Connect as the operator

You need three things from the control plane's host: its address, the CA that `fenders-pki`
made (`pki/ca.pem`), and the operator token the control plane wrote at first start.

```console
$ fenders profile add lab --url https://127.0.0.1:29090 --ca-cert pki/ca.pem --token-file control/operator.token
✓ Added profile lab for the operator at https://127.0.0.1:29090/v1 (credential in the credentials file)
  Next: `fenders tenants create NAME` and `fenders keys issue`.
```

- The URL may omit `/v1`; it is added.
- The CLI asks `GET /v1/me` who the credential is, so the profile knows it is the operator's.
- Without `--ca-cert` the connection is refused, because the control plane's certificate is
  not signed by a public CA:

  ```console
  ✗ could not verify https://127.0.0.1:29090/v1's certificate
    Pass the CA that signed it: --ca-cert pki/ca.pem (fenders-pki's output).
  ```

- Instead of `--token-file`, the token can be piped in (`… | fenders profile add lab --url …`)
  or typed at a hidden prompt.

The first profile becomes the current one. Check it:

```console
$ fenders profile list
NAME   ROLE      API URL
lab *  operator  https://127.0.0.1:29090/v1

$ fenders status
  api       https://127.0.0.1:29090/v1
  healthz   ok
  readyz    ready

process_id  68e05dcb040949a6882da187fc8908ac
uptime_ms   3504174
components
  •
    component              scheduler
    consecutive_failures   0
    failures               0
    ...
```

`status` shows the two probes anyone may ask (`/healthz`, `/readyz`) and, for the operator, the
state of each of the control plane's loops, its database and its gateway.

### 2. Tenants

Create one per customer. The allowance is what the tenant may hold at once (CPU in hundredths
of a core, memory, instances) and what it may store and send a month. Sizes take `MiB`, `GiB`,
`MB`, `GB` or plain bytes. Flags left out get the defaults: one core, 512MiB, 1GiB storage, 2
instances, 1GiB transfer.

```console
$ fenders tenants create acme --cpu-percent 400 --memory 2GiB --instances 4 --transfer 10GiB
✓ Created tenant acme d802c9e3bbb841fdb3f9099ee69ede94
  Give it a key: fenders keys issue d802c9e3bbb841fdb3f9099ee69ede94 --save-profile acme

$ fenders tenants list
ID                                NAME   LIFECYCLE STATE  CPU PERCENT  MEMORY MIB  INSTANCES  CREATED
d802c9e3bbb841fdb3f9099ee69ede94  acme   active           400          2048        4          just now
4db1d77551234aae9fcb00456b910bd9  gamma  active           100          512         2          36 min ago
```

A tenant's name is a label: two may share one. Commands take the name or the id; when a name
is ambiguous, use the id.

Change what it may use. Only the flags given change; the CLI sends the rest of the allowance
as it is, with the revision it read, so a concurrent change is refused instead of overwritten:

```console
$ fenders tenants set acme --instances 6 --default-size small
✓ Updated tenant acme

$ fenders tenants http-limits acme --concurrent 32 --per-minute 1200
✓ Updated the HTTP limits of acme

$ fenders tenants get acme
allowance_revision        2
cpu_percent               400
default_app_size          small
http_concurrent_requests  32
http_requests_per_minute  1200
id                        d802c9e3bbb841fdb3f9099ee69ede94
instances                 6
lifecycle_state           active
memory_mib                2048
name                      acme
storage_bytes             1073741824
transfer_bytes            10737418240
...
```

### 3. Keys

A key is how a tenant (or your product, on its behalf) uses the API. Its token is shown once,
alone on stdout, so `TOKEN=$(fenders keys issue …)` works. `--save-profile` also stores it as
a tenant profile on this machine:

```console
$ fenders keys issue acme --label 'team:acme' --save-profile acme
✓ Issued key fenders-cli for acme
fnd_tok_…
! This is the only time the token is shown.
  Saved as profile acme: `fenders -P acme context`, or `fenders profile use acme`
```

Narrow keys for machines: fewer permissions, a short life, one project, and a label that
appears in the audit trail with every change the key makes:

```console
$ fenders keys issue acme --name ci --permissions read,runs:write --ttl 12h --label 'ci:nightly'
✓ Issued key ci for acme
fnd_tok_…
! This is the only time the token is shown.

$ fenders keys list acme
ID                                NAME         PREFIX            PERMISSIONS       EXPIRES  REVOKED  CREATED
2377bcc1024f425cb5bb295bcc6c643f  ci           fnd_tok_c7d1be52  read, runs:write  in 11 h  no       just now
22a0e65a9458483c8e764a9fb6cc1340  fenders-cli  fnd_tok_9e5d06b7  apps:write, …     in 29 d  no       just now

$ fenders keys revoke acme 2377bcc1024f425cb5bb295bcc6c643f
✓ Revoked key 2377bcc1024f425cb5bb295bcc6c643f
```

Permissions: `read`, `apps:write`, `runs:write`, `projects:write`, `connections:write`,
`secrets:write`, or `all` (the default). `--ttl` is 1 minute to 30 days (`15m`, `12h`, `30d`);
re-issue before it ends. A revoked key stops working at once.

A key can do nothing of the operator's:

```console
$ fenders -P acme nodes list
✗ API key cannot access this endpoint (permission_denied, HTTP 403)
  This credential may not do that. Operator actions need an operator profile (`fenders profile use …`); a tenant key needs the permission or project scope — the operator can issue one with `fenders keys issue`.
  request id: e53b4da8c4db4aa1b3111ce5fa37aa70
```

### 4. Nodes, edges and outbound access

Register a node with its certificate; the CLI computes the pin the control plane checks (the
SHA-256 of the certificate, as `fenders-pki` prints it), or takes `--pin`:

```console
$ fenders nodes register everest-1 --cert pki/node.pem --endpoint https://127.0.0.1:29094 \
    --runtime wasmtime-50.0.0-fenders-v1 --cpu-percent 400 --memory 4GiB --instances 8
✓ Registered node everest-1

$ fenders nodes list
ID                                NAME       ENDPOINT                 CPU PERCENT  MEMORY MIB  INSTANCES  DRAINING
5f206b97d9b24152a0c1f2abca57067d  everest-1  https://127.0.0.1:29094  400          4096        8          no
```

Then start `fenders-node` on that machine pointing at the control plane. Take a node out of
service and back:

```console
$ fenders nodes drain everest-1
✓ Draining node everest-1

$ fenders nodes undrain everest-1
✓ Node everest-1 takes runs again
```

`nodes set` changes its capacity, `nodes disable|enable` switches it off and on, and
`nodes delete` removes a drained one. With one control-plane process there are no edges
(`fenders edges list` is empty); with several, `edges register|drain|delete` and
`edges routes` manage them the same way.

Apps reach HTTPS origins only when the control plane was started with `--public-egress` and
the operator has turned the public egress policy on. Until then a tenant attaching an origin
is told `Public HTTPS egress is not configured`. Turn it on with the digest the control plane
reports:

```console
$ fenders egress-policy get
configured_digest   619b8ee9c2926b9b5268221d3a43598dd26cce2c5f544cdcb40a414985cc995e
digest              —
enabled             no
...

$ fenders egress-policy enable --digest 619b8ee9c2926b9b5268221d3a43598dd26cce2c5f544cdcb40a414985cc995e
✓ Public egress policy on
```

### 5. Work as a tenant

With the profile from `keys issue --save-profile` (or `fenders profile add acme --url … --ca-cert …`
with the key piped in), switch to it, or name it per command with `-P acme`:

```console
$ fenders profile use acme
✓ Now using profile acme

$ fenders context
  profile     acme (tenant)
  api         https://127.0.0.1:29090/v1
  auth        credentials file
  workspace   acme d802c9e3bbb841fdb3f9099ee69ede94
  project     Production 56124290ed0f435fb89ce7fdbae45a61
```

A tenant profile starts in its key's workspace and, if there is only one, its project. Make
more projects and choose where commands apply:

```console
$ fenders projects create staging
✓ Created project staging

$ fenders use acme/Production
✓ Now using workspace acme · project Production
```

`-w WORKSPACE` and `-p PROJECT` (or `FENDERS_WORKSPACE`/`FENDERS_PROJECT`) override it for
one command. `projects rename` and `projects delete` change the others; the default project
cannot be deleted.

### 6. Deploy and run

`deploy` uploads a component, waits for the control plane to inspect it, creates the app and
releases it. A component that is a command can be run by hand; one that handles HTTP gets an
address.

```console
$ fenders deploy job.wasm --name report
✓ Uploaded job.wasm artifact 4830d337571447e4a64bf7acc2ac961f
✓ Component validated: command
✓ Deploy report — done

  app       report d72c9c87d8ee4465a484d054a9b18f81
  state     ready
  desired   enabled

  Logs:   fenders logs report -f
  Run it: fenders run report

$ fenders run report --logs
✓ Run report — done
...
exit_code         0
state             succeeded
...
11:46:29.206 out  FENDERS_UPSTREAM_BOOT
11:46:29.206 out  FENDERS_COMMAND_FIXTURE
11:46:29.206 out  command complete
11:46:29.206 out  FENDERS_COMPONENT_COMPLETE
11:46:29.206 out  FENDERS_UPSTREAM_LINKED

$ fenders runs list report
ID                                SOURCE  STATE      CREATED   DURATION MS  VERSION  UPDATED
7a52f653376849278f14134bfcfb7718  manual  succeeded  just now  0            1        just now

$ fenders logs report
11:46:29.206 7a52f653 out  FENDERS_UPSTREAM_BOOT
11:46:29.206 7a52f653 out  FENDERS_COMMAND_FIXTURE
...
```

`logs -f` keeps following; `--run RUN` shows one run. `runs cancel APP RUN` stops a run that
has not ended. `fenders sizes` lists the instance sizes `--size` takes:

```console
$ fenders sizes
ID      STATUS        CPU PERCENT  GUEST MEMORY MIB  RESERVATION MEMORY MIB  VM MEMORY MIB  WASM LINEAR MEMORY MIB
small   experimental  25           32                88                      64             4
medium  experimental  50           96                152                     128            40
large   experimental  100          224               280                     256            104
```

An HTTP component is served by the control plane's gateway under its `--domain`:

```console
$ fenders deploy web.wasm --name web
✓ Uploaded web.wasm artifact 872a6c5bda6a4368a2d3accfc104a180
✓ Component validated: HTTP handler
✓ Deploy web — done

  app       web fd239d5e3d90405091b20346796fb059
  state     ready
  desired   enabled
  url       https://app-fd239d5e3d90405091b20346796fb059.apps.fenders.local

$ fenders apps url web
https://app-fd239d5e3d90405091b20346796fb059.apps.fenders.local
```

Point DNS for `*.DOMAIN` at the gateway port. Locally, `curl --resolve` does it. The local
control plane serves its gateway with its own certificate, which does not name app hosts, hence
`-k`; give the gateway a certificate for `*.DOMAIN` with `--gateway-cert`/`--gateway-key` in
production:

```console
$ curl -sk --resolve app-fd239d5e3d90405091b20346796fb059.apps.fenders.local:29092:127.0.0.1 \
    https://app-fd239d5e3d90405091b20346796fb059.apps.fenders.local:29092/hello
fenders:/hello
```

Deploy new code to an existing app with `--app`; it becomes a new version and is released:

```console
$ fenders deploy job.wasm --app report
✓ Uploaded job.wasm artifact b7c078e83597461fb15419b0f3161e2c
✓ Component validated: command
✓ Create version — done
✓ Release report — done

  app       report 154b1633463b4112ac5a411a258b99e6
  state     ready
  desired   enabled
```

(`fenders apps deploy NAME --artifact FILE` asks before releasing to an app that exists, so in
scripts use `deploy --app`.) `apps stop|start` switch an app off and on, and `apps delete`
removes it.

### 7. Configuration and secrets

Secrets are encrypted in the control plane and never shown again. Values come from stdin,
`--from-file` or a hidden prompt, never from a flag:

```console
$ printf %s "s3cr3t-pass" | fenders secrets create db-password
✓ Stored secret db-password
  Use it with `--secret-env ENV_NAME=db-password` on deploy or release

$ printf %s "n3w-pass" | fenders secrets set db-password
✓ Updated secret db-password; runs started from now on use it
```

Environment and secrets are part of a release. Release the current version with new ones:

```console
$ fenders releases create report -e REPORT_KIND=daily --secret-env DB_PASSWORD=db-password
✓ Release report — done

$ fenders releases list report
ID                                VERSION ID                        STATE  CREATED   VERSION
6441fab62ede47f3adc94142796c710c  5c8b7c9553294a8d979f420358101907  ready  just now  1
86bc82d2f8cd4344abd1cd6e72a0ab83  5c8b7c9553294a8d979f420358101907  ready  just now  1
```

`deploy` takes the same `-e` and `--secret-env` flags. `versions list` shows the code an app
has had; `releases create --version N` goes back to one. `secrets delete` refuses while a
release, run or connection names the secret.

### 8. Triggers and schedules

```console
$ fenders schedule preview '0 6 * * *' --timezone Europe/Kyiv
expression         0 6 * * *
timezone           Europe/Kyiv
occurrences
  •
    local  2026-11-23T06:00:00+02:00
    utc    2026-11-23T04:00:00+00:00
  ...

$ fenders triggers set report --schedule '0 6 * * *' --timezone Europe/Kyiv
✓ Update triggers — done
http      no
manual    yes
revision  2
schedule
  enabled     yes
  expression  0 6 * * *
  next_at     2026-11-23T04:00:00+00:00 (in 17 h)
  timezone    Europe/Kyiv
  ...

$ fenders triggers set report --no-schedule
✓ Update triggers — done
```

`--http true|false` and `--manual true|false` switch the other two triggers.
`fenders events report` lists what happened to an app: releases, runs, starts and stops.

### 9. Outbound connections

An app reaches nothing outside unless it is given a connection. Register one in the project,
then allow it on the app (`NAME:PREFIX` gives the connection an environment prefix in the app):

```console
$ fenders https-access validate https://api.github.com
origins  https://api.github.com:443

$ fenders connections create https github --origin https://api.github.com
✓ Register connection github — done

$ fenders connections create tcp postgres --host 1.1.1.1 --port 5432
✓ Register connection postgres — done

$ fenders apps set-connections report github postgres:PG
✓ Update connections — done

$ fenders apps connections report
KIND   TARGET                      STATE    PREFIX  ID
tcp    postgres                    allowed  PG      e491226cdb5f4602964283a2c81624ed
https  https://api.github.com:443  allowed  —       5b8c146c520c4536a0d97bd25979e2e9
```

HTTPS origins need the operator's public egress policy on (section 4); `fenders https-access show`
says whether it is. `connections revoke NAME` removes one; apps using it lose access.

### 10. Usage and audit

```console
$ fenders usage
artifact_bytes             414222
cpu_usec                   26053
scope                      project
storage_bytes              414222
allowance
  cpu_percent     400
  instances       6
  memory_mib      2048
  ...
```

`--workspace-wide` covers every project; `--history` lists closed months.

The audit trail lists every change in a workspace, with the label of the key that made it:

```console
$ fenders audit -n 3
CREATED   ACTOR                             ACTOR LABEL  ACTION           RESOURCE ID
just now  22a0e65a9458483c8e764a9fb6cc1340  team:acme    endpoint.create  c7e25532a23742d88798574867b2eff3
just now  control-plane                     —            ingress.admit    9728b412c5534cb39a53c426e8b02f52
just now  control-plane                     —            execution.assign aad80339d3d9439ea2430b1e85d3c42c
```

Filter with `--action`, `--actor-label`, `--since` and `--until`. The operator reads any
tenant's: `fenders -P lab audit acme --action operator.suspend`.

### 11. Suspend, resume, delete

Suspending a tenant ends its runs and leaves its keys able only to read:

```console
$ fenders -P lab tenants suspend acme --reason "invoice overdue"
✓ Suspended

$ printf %s "abcd" | fenders -P acme secrets create another
✗ Workspace is suspended; read-only access remains available (workspace_suspended, HTTP 403)
  The workspace is suspended: reads still work, changes wait until the operator resumes it (`fenders tenants resume`).

$ fenders -P lab tenants resume acme
✓ Resumed
```

A tenant is deleted only once it is suspended. Deleting it removes its keys, secrets and
storage, and cannot be undone:

```console
$ fenders -P lab tenants delete acme --yes
✗ Suspend the workspace, and wait until it is suspended, before deleting it (in_use, HTTP 409)

$ fenders -P lab tenants suspend acme --reason "offboarding"
✓ Suspended

$ fenders -P lab tenants delete acme --yes
✓ Deleted tenant acme

$ fenders -P acme apps list
✗ Unauthorized (unauthorized, HTTP 401)

$ fenders profile remove acme
✓ Removed profile acme
  Current profile: lab
```

### 12. Scripting and CI

Output is JSON whenever stdout is not a terminal, or with `-o json`/`--json`; progress and
errors go to stderr, and the exit status is non-zero on any failure:

```console
$ fenders apps list -o json | jq -r '.[] | [.name, .state] | @tsv'
web	ready
report	ready

$ fenders runs list report --json | jq -c '.[0] | {state, exit_code, duration_ms}'
{"state":"succeeded","exit_code":0,"duration_ms":0}
```

In CI there is no saved profile: give the URL, the CA and the key through flags and the
environment, and name the workspace and project:

```console
$ FENDERS_TOKEN=fnd_tok_… fenders --api-url https://127.0.0.1:29090 --ca-cert pki/ca.pem \
    -w acme -p Production apps list
ID                                NAME    STATE  DESIRED  UPDATED    REVISION  CREATED
fd239d5e3d90405091b20346796fb059  web     ready  enabled  1 min ago  3         1 min ago
d72c9c87d8ee4465a484d054a9b18f81  report  ready  enabled  just now   6         2 min ago
```

Add `--no-input` so nothing waits for a prompt. Long operations can be started and waited for
separately:

```console
$ fenders run report --no-wait -o json | jq -r .id
b937c47f7d5140b58261a3964a8f2f6f

$ fenders ops wait b937c47f7d5140b58261a3964a8f2f6f
✓ run-app — done
```

Anything the commands do not cover is one `api` call away; `{workspace}` and `{project}` are
filled from the context, and the call carries the credential and an Idempotency-Key:

```console
$ fenders api '/workspaces/{workspace}/projects' -X POST -d '{"name":"scratch"}'
{
  "created": "2026-11-22T10:48:31.879105+00:00",
  "id": "967cae1530684a7f8df4b7f8bb58e6e0",
  "is_default": false,
  "name": "scratch",
  "slug": "scratch",
  "workspace_id": "d802c9e3bbb841fdb3f9099ee69ede94"
}
```

`fenders completion bash|zsh|fish|powershell|elvish` prints a completion script, e.g.
`fenders completion zsh > "${fpath[1]}/_fenders"`.

### 13. When something fails

Every error shows the control plane's message and code, a hint, and the request id the
control plane logged.

| You see | It means |
|---|---|
| `could not verify …'s certificate` | Add `--ca-cert` with the CA that signed the control plane's certificate. |
| `unauthorized, HTTP 401` | The key expired, was revoked, or its tenant was deleted. `fenders profile show` checks it. |
| `permission_denied, HTTP 403` | An operator command with a tenant key, or a key without the permission or project. |
| `workspace_suspended` | The operator suspended the tenant; reads still work. |
| `quota_exhausted` | An allowance is used up; the operator raises it with `tenants set`. |
| `in_use` | Something still depends on it, or (for a tenant) it is not suspended yet; the message says which. |
| `revision_conflict` | It changed since it was read; run the command again. |
| `Public HTTPS egress is not configured` | The operator has not turned on the public egress policy (section 4). |

`-v` logs every HTTP request to stderr.

## AI assistants

`fenders skill install` installs an [Agent Skill](https://docs.claude.com/en/docs/agents-and-tools/agent-skills/overview)
that teaches an AI assistant (Claude Code, or any agent that reads skills) to use this CLI: to
build and deploy `.wasm` web apps, and to run code it does not trust in a Fenders sandbox,
either through a runner app (a JS or Python interpreter deployed as an HTTP app) or built as a
command and run once.

```console
$ fenders skill install                                 # ~/.claude/skills/fenders, for every project
$ fenders skill install --dir .claude/skills/fenders    # or for one project
$ fenders skill show reference/sandbox.md               # read any of its files
```

The skill is built into the binary, so it always matches the installed version's commands;
run `skill install --force` after `fenders update` to refresh a copy you edited. It holds:

- `SKILL.md`: when to use Fenders, the checks to make first, rules (a tenant profile, never the
  operator's; `--no-input -o json`; secrets only on stdin), and the deploy and run workflows.
- `reference/`: building components and calling HTTP apps, running untrusted code and what is
  isolated (measured: no network, no files, a fresh instance per request, memory by size, a
  CPU budget per run), the commands with the JSON they return, and every error code.
- `templates/`: a Rust HTTP handler and a Rust command that build into components Fenders runs.

Give the assistant its own tenant key, not the operator's:

```console
$ fenders keys issue acme --permissions read,apps:write,runs:write --ttl 1d --label agent --save-profile agent
$ fenders profile use agent
```

Tested by giving the skill to Claude Code and asking it to build and deploy a JSON web app
(it did, from the template, and called it), to run untrusted Rust in a sandbox (it did, read
the output, and deleted the app), and to run JavaScript with no runner deployed (it stopped and
asked for the runner's `.wasm` instead of inventing one). The tests in `src/cmd/skill.rs` hold
the skill's every `fenders …` command to what the binary accepts.

## Commands

| Area | Commands |
|---|---|
| Profiles | `profile add \| list \| use \| show \| remove`, `auth login \| logout \| status \| token`, `use [WORKSPACE[/PROJECT]]`, `context` |
| Operator | `tenants list \| get \| create \| set \| http-limits \| suspend \| resume \| delete`, `keys issue \| list \| revoke`, `nodes list \| get \| register \| set \| drain \| undrain \| disable \| enable \| delete`, `edges list \| register \| drain \| undrain \| delete \| routes`, `egress-policy get \| enable \| disable` |
| Either | `status`, `audit [TENANT]` |
| Ship | `deploy FILE.wasm`, `run APP`, `logs APP [-f]` |
| Apps | `apps deploy \| list \| get \| create \| start [--release] \| stop \| delete \| url \| connections \| set-connections` |
| App history | `versions …`, `releases …`, `runs list \| get \| create \| cancel`, `events APP`, `triggers get \| set` |
| Project | `projects list \| create \| rename \| delete`, `artifacts list \| get \| inspect \| upload \| delete`, `connections list \| get \| create https\|tcp \| revoke`, `secrets list \| get \| create \| set \| delete`, `ops list \| get \| wait`, `usage`, `sizes`, `https-access show \| validate`, `schedule preview` |
| Anything else | `api PATH [-X METHOD] [-d JSON\|@file]` with `{workspace}`/`{project}` placeholders |
| AI assistants | `skill install [--dir DIR] [--force]`, `skill show [FILE]`, `skill files` |
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

# Against a real control plane (creates and then deletes a tenant of its own):
FENDERS_E2E_URL=https://HOST:9090 FENDERS_E2E_CA=pki/ca.pem \
FENDERS_E2E_TOKEN_FILE=STATE/operator.token \
FENDERS_E2E_WASM=example-command.wasm \
bash scripts/e2e_local.sh target/debug/fenders   # runs too, when a node is registered
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
  cmd/           one module per command group (profile.rs, operator.rs, skill.rs, …)
skills/fenders/  the agent skill `fenders skill install` writes (built into the binary)
install.sh       installer / updater / uninstaller for macOS and Linux
```
