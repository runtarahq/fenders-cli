# The commands you will use

Always add `--no-input -o json`. Stdout is JSON; progress, hints and errors are on stderr; the
exit status is 0 on success and non-zero on any failure. `-P PROFILE`, `-w WORKSPACE` and
`-p PROJECT` choose where a command applies; names and ids are both accepted.

## Where am I

```bash
fenders context --no-input -o json
```

```json
{"profile": "skilltest", "role": "tenant", "api_url": "https://127.0.0.1:29090/v1",
 "workspace": {"id": "44c0…", "name": "skilltest", "source": "config"},
 "project": {"id": "d8bf…", "name": "Production", "source": "config"},
 "auth": "credentials file"}
```

`fenders profile list --no-input -o json` lists the profiles; `fenders projects list --no-input -o json`
the projects of the workspace.

## Apps

| Command | Prints |
|---|---|
| `fenders deploy FILE.wasm --name APP` | `{"app": {…}, "http": {…}}` |
| `fenders deploy FILE.wasm --app APP` | the same, for a new version of an existing app |
| `fenders apps list` | `[{"id", "name", "state", "desired", …}]` |
| `fenders apps get APP` | the app record |
| `fenders apps url APP` | `{"url", "state", "executions", …}` |
| `fenders apps stop APP` / `apps start APP` | the finished operation |
| `fenders apps delete APP --yes` | the finished operation |
| `fenders events APP` | releases, starts and stops, and each HTTP request (`method`, `path`, `http_status`, `state`, `duration_ms`) |

## Runs and output

| Command | Prints |
|---|---|
| `fenders run APP` | the run: `{"id", "state", "exit_code", "reason", "duration_ms", …}`; exit status 1 if it failed |
| `fenders runs list APP` | runs, newest first |
| `fenders runs get APP RUN` | one run |
| `fenders runs cancel APP RUN` | the finished operation |
| `fenders logs APP` | one JSON object per line: `{"stream": "stdout"\|"stderr", "text", "run_id", "received", …}` |
| `fenders logs APP --run RUN` | only that run's output |

Read output text with `jq -r .text`. `logs -f` follows, which never ends: do not use it
in a script that must finish.

## Configuration

| Command | |
|---|---|
| `printf %s "$V" \| fenders secrets create NAME` | value from stdin |
| `printf %s "$V" \| fenders secrets set NAME` | a new value; runs started after use it |
| `fenders secrets list` | names only, never values |
| `fenders releases create APP -e K=V --secret-env K=SECRET` | new configuration, without new code; `--version N` goes back to an older version, `--size S` changes the size |
| `fenders triggers get APP` / `triggers set APP --schedule CRON --timezone TZ` | schedules, HTTP, manual |
| `fenders sizes` | the sizes and their memory |

## Background operations

Changes wait for their operation by default. With `--no-wait` they print the operation
instead; wait for it later:

```bash
fenders run APP --no-wait --no-input -o json | jq -r .id
fenders ops wait OPERATION --no-input -o json
```

## Anything else

```bash
fenders api '/workspaces/{workspace}/projects/{project}/apps' --no-input -o json
```

`{workspace}` and `{project}` are filled from the context. The contract is served by the
control plane at `/api/openapi.json`.
