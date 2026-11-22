# Errors and what to do

Errors go to stderr as `✗ message (code, HTTP status)`, a hint, and a request id. Act on the
code.

| Code / message | Meaning | Do |
|---|---|---|
| `no profile selected` | No profile on this machine | Ask the user to add one (`fenders profile add`). |
| `could not verify …'s certificate` | The profile lacks the control plane's CA | Ask the user for the CA file; they pass `--ca-cert`. |
| `unauthorized` (401) | The key expired, was revoked, or its tenant deleted | Ask the user for a new key. Do not look for other credentials. |
| `permission_denied` (403) | The key may not do this (an operator command, a missing permission, another project) | Tell the user which permission is missing; do not switch to the operator profile. |
| `workspace_suspended` (403) | The operator suspended the tenant; reads still work | Tell the user; nothing can change until it is resumed. |
| `not_found` (404), `no app named …` | Wrong name or id, or the wrong project | Check `fenders context` and `fenders apps list`. |
| `not_validated` (409) | The file is not a component Fenders can run | Fix the build (see web-apps.md); do not retry. |
| `quota_exhausted` (409) | The tenant's allowance (CPU, memory, instances, storage, transfer) is used | Stop or delete apps you no longer need, choose a smaller `--size`, or ask the user to have the operator raise it. |
| `in_use` (409) | Something still depends on it | The message says what; change that first. |
| `already_exists` (409) | It exists | Use the existing one. |
| `revision_conflict` (409) | It changed since it was read | Run the same command again. |
| `rate_limited` (429) | Too many requests | Wait a few seconds, then retry once. |
| `busy` (503) | The control plane was busy | Retry the same command. |
| `Public HTTPS egress is not configured` | Outbound HTTPS is off for the whole control plane | Tell the user; only the operator can turn it on. |
| `Run APP failed: command-failed-or-cancelled` | The program failed | Read the run's stderr: `fenders logs APP --run RUN --no-input -o json`. See sandbox.md for what each trap means. |

`-v` before a command logs its HTTP requests to stderr, useful when a message is unclear.
