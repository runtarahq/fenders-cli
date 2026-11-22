#!/usr/bin/env bash
# Drive fenders-cli against a real control plane, end to end, and check
# every answer with jq.
#
#   FENDERS_E2E_URL=https://127.0.0.1:9090 \
#   FENDERS_E2E_CA=pki/ca.pem \
#   FENDERS_E2E_TOKEN_FILE=STATE/operator.token \
#   bash scripts/e2e_local.sh [path/to/fenders]
#
# It creates a tenant of its own (and deletes it at the end), so it is safe
# on a control plane that serves others. With FENDERS_E2E_WASM naming a
# command component (fenders' examples/…/example-command.wasm) and a node
# registered, it also deploys, runs, reads logs and cancels; without, those
# steps are skipped. Its profiles live in a temporary config directory.
set -euo pipefail

fenders=${1:-target/debug/fenders}
fenders=$(cd "$(dirname "$fenders")" && pwd)/$(basename "$fenders")
: "${FENDERS_E2E_URL:?the management API of the control plane, https://HOST:PORT}"
: "${FENDERS_E2E_CA:?the CA certificate (PEM) that signed its certificate}"
: "${FENDERS_E2E_TOKEN_FILE:?the operator token file}"

export FENDERS_CONFIG_DIR=$(mktemp -d)
unset FENDERS_TOKEN FENDERS_API_KEY FENDERS_PROFILE FENDERS_API_URL FENDERS_CA_CERT
tenant="e2e-$(date +%s)-$$"
f() { "$fenders" --no-input -o json "$@"; }
pass=0
ok() { pass=$((pass + 1)); printf '  ✓ %s\n' "$1"; }
fail() { printf '  ✗ %s\n' "$1" >&2; exit 1; }
expect() { # expect DESCRIPTION JQ_FILTER JSON
  jq -e "$2" >/dev/null <<<"$3" || fail "$1 — got: $(head -c 400 <<<"$3")"
  ok "$1"
}
cleanup() {
  f -P e2e-op tenants suspend "$tenant" --reason "e2e cleanup" >/dev/null 2>&1 || true
  f -P e2e-op tenants delete "$tenant" --yes >/dev/null 2>&1 || true
  rm -rf "$FENDERS_CONFIG_DIR"
}
trap cleanup EXIT

echo "profiles"
out=$(f profile add e2e-op --url "$FENDERS_E2E_URL" --ca-cert "$FENDERS_E2E_CA" --token-file "$FENDERS_E2E_TOKEN_FILE")
expect "the operator token is recognised" '.profile.role == "operator" and .current' "$out"
if f profile add e2e-noca --url "$FENDERS_E2E_URL" --token-file "$FENDERS_E2E_TOKEN_FILE" 2>/dev/null; then
  fail "a profile without the CA should not verify"
fi
ok "without the CA the certificate is refused"

echo "operator"
out=$(f -P e2e-op status)
expect "status answers" '.readyz == "ready" and (.control_plane.components | length) > 0' "$out"
out=$(f -P e2e-op tenants create "$tenant" --cpu-percent 200 --memory 1GiB --instances 4 --transfer 2GiB)
expect "a tenant is created with its allowance" '.cpu_percent == 200 and .memory_mib == 1024 and .instances == 4' "$out"
out=$(f -P e2e-op tenants set "$tenant" --instances 6)
expect "its allowance changes, the rest kept" '.instances == 6 and .cpu_percent == 200' "$out"
out=$(f -P e2e-op keys issue "$tenant" --label "e2e" --save-profile e2e-tenant)
expect "a key is issued and saved as a profile" '(.token | startswith("fnd_tok_")) and .saved_profile == "e2e-tenant"' "$out"
out=$(f -P e2e-op keys list "$tenant")
expect "the key is listed without its token" 'length == 1 and (.[0].token == null)' "$out"
out=$(f -P e2e-op nodes list)
expect "nodes are listed" 'type == "array"' "$out"

echo "tenant"
out=$(f -P e2e-tenant context)
expect "the tenant profile starts in its workspace and project" '.role == "tenant" and .workspace.id != null and .project.id != null' "$out"
if f -P e2e-tenant nodes list 2>/dev/null; then fail "a tenant key must not list nodes"; fi
ok "a tenant key is refused the operator's routes"
printf 'v4lue-one' | f -P e2e-tenant secrets create e2e-secret >/dev/null
printf 'v4lue-two' | f -P e2e-tenant secrets set e2e-secret >/dev/null
out=$(f -P e2e-tenant secrets list)
expect "a secret is created and set" 'map(.name) | index("e2e-secret") != null' "$out"
f -P e2e-tenant secrets delete e2e-secret --yes >/dev/null
out=$(f -P e2e-tenant projects create e2e-proj)
expect "a project is created" '.name == "e2e-proj"' "$out"
f -P e2e-tenant projects rename e2e-proj e2e-proj2 >/dev/null
f -P e2e-tenant projects delete e2e-proj2 --yes >/dev/null
ok "a project is renamed and deleted"

if [[ -n ${FENDERS_E2E_WASM:-} ]]; then
  echo "runs (FENDERS_E2E_WASM)"
  out=$(f -P e2e-tenant apps deploy e2e-app --artifact "$FENDERS_E2E_WASM" --manual)
  expect "a component deploys" '.app.state == "ready" or .state == "ready" or (.. | .state? == "ready")' "$out"
  f -P e2e-tenant run e2e-app >/dev/null
  out=$(f -P e2e-tenant runs list e2e-app)
  expect "a run succeeds" '.[0].state == "succeeded"' "$out"
  out=$(f -P e2e-tenant logs e2e-app | jq -s .)
  expect "its output is decoded" 'map(.text) | length > 0 and all(. != null)' "$out"
  run=$(f -P e2e-tenant run e2e-app --no-wait | jq -r '.resource_id // .id')
  f -P e2e-tenant runs cancel e2e-app "$run" >/dev/null || true
  out=$(f -P e2e-tenant runs get e2e-app "$run")
  expect "a run is cancelled (or had already ended)" '.state == "cancelled" or .state == "succeeded"' "$out"
  f -P e2e-tenant apps stop e2e-app >/dev/null
  f -P e2e-tenant apps delete e2e-app --yes >/dev/null
  ok "the app is stopped and deleted"
fi

echo "audit and lifecycle"
out=$(f -P e2e-tenant audit)
expect "the audit trail records the key's label" 'map(.actor_label) | index("e2e") != null' "$out"
f -P e2e-op tenants suspend "$tenant" --reason "e2e" >/dev/null
out=$(f -P e2e-op tenants get "$tenant")
expect "the tenant is suspended" '.lifecycle_state == "suspended"' "$out"
f -P e2e-op tenants delete "$tenant" --yes >/dev/null
if f -P e2e-tenant context >/dev/null 2>&1 && f -P e2e-tenant secrets list >/dev/null 2>&1; then
  fail "a deleted tenant's key must stop working"
fi
ok "the tenant is deleted and its key no longer works"

echo "$pass checks passed"
