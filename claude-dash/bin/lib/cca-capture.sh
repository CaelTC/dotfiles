# cca-capture.sh — the shared capture lifecycle behind `cca` (human sessions) and
# `cc` (agent sessions). Sourced, not executed: it runs inline in the caller's
# top-level shell so trap/signal/process semantics are exactly the caller's own.
#
# The caller sets three vars, then `source`s this file (with no args, so this
# file inherits the caller's positional params as claude's args):
#   _cca_prog    — program name for diagnostics ("cca" | "cc")
#   _cca_origin  — the Session's Origin: "human" | "agent" (tags the start record)
#   _cca_perm    — permission behavior: "force-auto" | "passthrough"
#
# Per invocation this:
#   1. mints a short Session id,
#   2. picks a free localhost port and starts `claude-dash proxy` on it for that id,
#   3. WAITS for the Proxy to confirm it bound its port (ADR-0002 fail-open),
#   4. ONLY THEN writes the Session's `start` record (via `claude-dash
#      record-start`, so the JSONL schema stays owned by the Rust code; `--agent`
#      is passed for Agent Origin) and exports ANTHROPIC_BASE_URL at the Proxy,
#   5. runs `claude`, either forcing `--permission-mode auto` (human) or passing
#      the caller's args straight through (agent, so it can pass
#      `--dangerously-skip-permissions` and run unattended).
#
# Fail-open (ADR-0002): the dashboard must NEVER block `claude`. Capture is
# best-effort. We health-check the Proxy before committing capture; if the Proxy
# can't bind/start (or doesn't report ready in time), we run `claude` DIRECTLY —
# without ANTHROPIC_BASE_URL, so that one run is uncaptured — rather than
# failing. A missing dashboard row is acceptable; a blocked session is not. Per
# the ADR there is NO mid-session supervision here (no watchdog/restart) — just
# this pre-launch health check plus the direct-launch fallback.

# How long to wait for the Proxy to report it bound its port before giving up and
# launching claude directly (fail-open). Kept small so a failing Proxy never
# delays claude noticeably.
local -r ready_timeout_secs=2

# The capture binary. Overridable for dev (point at target/{debug,release}).
local dash="${CLAUDE_DASH_BIN:-claude-dash}"

# The permission behavior, resolved once into the args prepended to `claude`.
# force-auto (cca): keep `--permission-mode auto`. passthrough (cc): nothing —
# the caller's args flow straight through so an agent can pass
# `--dangerously-skip-permissions`.
local -a claude_perm
if [[ "$_cca_perm" == "force-auto" ]]; then
  claude_perm=(--permission-mode auto)
else
  claude_perm=()
fi

# The Session's Origin, resolved once into the args passed to `record-start`.
# agent (cc) tags the start record Origin=Agent; human (cca) passes nothing and
# the record defaults to Human.
local -a record_origin
if [[ "$_cca_origin" == "agent" ]]; then
  record_origin=(--agent)
else
  record_origin=()
fi

# Fail-open (ADR-0002): never let a capture problem block claude. Warn with the
# reason ($1), then run claude DIRECTLY — without ANTHROPIC_BASE_URL, so this run
# is uncaptured but not blocked. The remaining args are claude's, passed through
# with the caller's permission behavior. This is the one definition of the
# fail-open launch contract.
fail_open() {
  print -u2 "${_cca_prog}: $1; launching claude without capture"
  shift
  exec claude "${claude_perm[@]}" "$@"
}

# 1. Mint a short, filename-safe, human-labelable Session id (first 8 hex of a uuid).
local id
id="$(uuidgen | tr 'A-Z' 'a-z' | tr -d '-')"
id="${id[1,8]}"

# 2. Pick a free localhost port. Bind port 0 in a throwaway socket and read back
#    the port the OS assigned — the small TOCTOU window before the proxy rebinds
#    it is acceptable for a single-user dev tool (and slice 05 adds fail-open).
local port
port="$(
  python3 - <<'PY' 2>/dev/null
import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
PY
)"
if [[ -z "$port" ]]; then
  fail_open "could not pick a free port" "$@"
fi

local base_url="http://127.0.0.1:${port}"
local cwd="$PWD"
local project="${PWD:t}"   # zsh: basename of cwd

# 3. Start the per-Session Proxy in the background on the chosen port. Capture its
#    stdout to a temp file so we can wait on the Proxy's machine-readable `READY`
#    line (printed once it has bound the port). The Proxy's stdout/stderr are
#    ours to consume — separate from claude's stdio, so this never corrupts what
#    the client sees.
local ready_log
ready_log="$(mktemp -t cca-ready.XXXXXX)"
"$dash" proxy --addr "127.0.0.1:${port}" --id "$id" >"$ready_log" 2>/dev/null &
local proxy_pid=$!

# 4. Wait — with a bounded timeout — for the Proxy to confirm it bound its port.
#    We poll the captured stdout for the `READY` line rather than probing the TCP
#    port: the READY line is printed by the Proxy process itself only after a
#    successful bind, so it proves THIS Proxy is up (a TCP connect could be
#    answered by some unrelated process that grabbed a reused port). The loop is
#    strictly bounded by ready_timeout_secs and also bails the instant the Proxy
#    process dies, so the wait itself can never hang and block claude.
local proxy_ready=0
local -ri max_polls=$(( ready_timeout_secs * 20 ))  # 20 polls/sec at sleep 0.05
local -i poll=0
while (( poll++ < max_polls )); do
  if grep -q '^READY ' "$ready_log" 2>/dev/null; then
    proxy_ready=1
    break
  fi
  # If the Proxy process has already exited it will never print READY — stop
  # waiting immediately and fall through to the direct (capture-free) launch.
  kill -0 "$proxy_pid" 2>/dev/null || break
  sleep 0.05
done
rm -f "$ready_log"

if (( ! proxy_ready )); then
  # Fail-open (ADR-0002): the Proxy failed to bind/start (or didn't report ready
  # in time). Reap any stray Proxy, then fall open — no ANTHROPIC_BASE_URL, no
  # start record — so this run is uncaptured but never blocked. The single-place
  # invariant holds: we have NOT exported ANTHROPIC_BASE_URL on this path.
  kill "$proxy_pid" 2>/dev/null
  fail_open "proxy did not come up" "$@"
fi

# --- The Proxy is confirmed bound. Only now do we commit capture. ---

# Tear the Proxy down when this shell exits, however claude returns.
trap 'kill "$proxy_pid" 2>/dev/null' EXIT INT TERM

# 5. Write the Session's `start` record. The pid we record is THIS shell's pid —
#    the process that represents the Session for slice-04 liveness. `claude` runs
#    as a foreground child (not `exec`'d), so this shell stays alive for exactly
#    the Session's lifetime and its pid tracks the Session's liveness. `--agent`
#    (present only for Agent Origin) tags the start record.
"$dash" record-start \
  --id "$id" \
  --project "$project" \
  --cwd "$cwd" \
  --pid "$$" \
  "${record_origin[@]}" \
  || print -u2 "${_cca_prog}: failed to write start record for session $id"

# 6. Point inference at the Proxy. This is the ONE place ANTHROPIC_BASE_URL is
#    ever exported, and it is reached only AFTER readiness is confirmed above —
#    capture is committed only once the Proxy is known to be bound (ADR-0002).
export ANTHROPIC_BASE_URL="$base_url"

# 7. Run claude through the Proxy, passing through all args. force-auto keeps
#    `--permission-mode auto`; passthrough runs the caller's args verbatim (so an
#    agent can pass `--dangerously-skip-permissions`). Not `exec` — we keep the
#    shell alive so we can write the `end` record and the EXIT trap can reap the
#    Proxy once claude returns.
claude "${claude_perm[@]}" "$@"

# 8. claude has returned. Write the Session's `end` record (via record-end, so
#    the JSONL schema stays Rust-owned) — this is what moves the Session out of
#    the active box and into Session History. Best-effort: if it fails, the TUI
#    still detects the ended Session via pid-liveness once this shell exits.
"$dash" record-end --id "$id" \
  || print -u2 "${_cca_prog}: failed to write end record for session $id"
