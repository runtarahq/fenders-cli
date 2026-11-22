# Running code in a Fenders sandbox

Use this when the user wants code run somewhere it cannot hurt anything: code you generated,
code from the user, code from the internet.

There are two ways. Prefer the first when a runner for the language is available.

## A. Through a runner app

A runner is an ordinary HTTP app that executes code it is sent (a JavaScript or Python
interpreter built as a WebAssembly component). It is not part of Fenders: it is deployed like
any app, and how to call it (path, method, body, response) is the runner's own. Read the
runner's documentation, or ask the user, for that contract; do not guess it.

1. Find it. Is it already deployed in this project?

   ```bash
   fenders apps list --no-input -o json | jq -r '.[] | [.name, .state] | @tsv'
   ```

2. If not, deploy it once from the `.wasm` the user gives you (ask for the path; do not
   download one yourself):

   ```bash
   fenders deploy js-runner.wasm --name js-runner --no-input -o json
   ```

3. Get its address:

   ```bash
   fenders apps url js-runner --no-input -o json | jq -r .url
   ```

4. Send code as the runner expects, for example a POST with the source as the body, and read
   the response. Each request is executed in a fresh instance (see below).
5. When something fails, the runner's own output is in its logs and each request is in its
   events:

   ```bash
   fenders logs js-runner --no-input -o json | jq -r .text
   fenders events js-runner --no-input -o json | jq '.[0:5][] | {method, path, http_status, state}'
   ```

Leave the runner deployed for later requests unless the user asks otherwise.

## B. As a command

For code that compiles to `wasm32-wasip2` (Rust, C, Go, …), build it as a command component
(start from `templates/command-rust`), deploy it, run it once, read its output, and delete it.

```bash
cargo build --release --target wasm32-wasip2
fenders deploy target/wasm32-wasip2/release/fenders-command.wasm --name sandbox-task -e INPUT=42 --no-input -o json
fenders run sandbox-task --no-input -o json > run.json; echo "exit status $?"
fenders logs sandbox-task --no-input -o json | jq -r 'select(.text | startswith("FENDERS_") | not) | "[\(.stream)] \(.text)"'
fenders apps delete sandbox-task --yes --no-input -o json
```

- **Building happens on this machine.** Compiling code runs its build scripts (`build.rs`) and
  procedural macros here, outside the sandbox. Only build untrusted code yourself when it has
  neither (and no dependencies that do); otherwise use a runner, or ask the user.
- A run takes no arguments and no stdin. Give it data in environment variables (`-e`), or
  build the data into the program.
- `fenders run` exits non-zero when the run fails, and still prints the run as JSON:
  `{"state": "failed", "exit_code": 1, "reason": "execution-failed", …}`. A successful run is
  `{"state": "succeeded", "exit_code": 0, …}`.
- WASI reports only success or failure: `std::process::exit(3)` arrives as exit code 1. To
  tell failures apart, print to stdout or stderr and read the logs.
- Why it failed is the last lines of the run's stderr:
  - `wasm trap: all fuel consumed by WebAssembly`: it used its CPU budget (below).
  - `wasm trap: wasm 'unreachable' instruction executed`: a panic or abort, including running
    out of memory.
  - `Exited with i32 exit status 1`: the program exited with an error.
- Lines starting `FENDERS_` are the runtime's, not the program's.

## What is isolated, and what limits apply

Measured on a Fenders control plane, not assumed:

| | |
|---|---|
| **Machine** | Each app runs in a microVM of its own, with its own cgroup and uid on the node. Apps of different tenants never share one. |
| **Between requests** | Every HTTP request and every run gets a fresh WebAssembly instance: no memory, globals or state carry over from the previous request. |
| **Network** | None. A TCP connect fails with `Permission denied`. An app reaches the outside only through a connection attached to it (`fenders apps set-connections`), and only to those destinations. Do not attach one to sandboxed code unless the user asks. |
| **Files** | None by default: `/` and `/tmp` do not exist for the code. |
| **Memory** | The app's size decides it (`fenders sizes`, column `WASM LINEAR MEMORY MIB`): `small` 4 MiB, `medium` 40 MiB, `large` 104 MiB. Beyond it, allocation fails and the program aborts. |
| **CPU** | Each run or request has a fixed budget of WebAssembly work, about a second of busy computation on `small`; larger sizes get through it faster (about 0.3 s on `large`) but not further. A long computation fails with `all fuel consumed`. |
| **Allowance** | All of a tenant's apps share its allowance. A larger size can be refused with `quota_exhausted` while other apps hold their share; stop or delete apps you no longer need, or ask the operator for more. |

So for code you do not trust:

- Run it as the user's tenant, never with the operator profile.
- Use a separate app (a separate runner, or a separate command) for code from parties that
  must not see each other's output: requests to one app share its logs.
- Choose `--size medium` or `--size large` only when the code needs the memory.
