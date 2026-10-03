#!/usr/bin/env bash
# Codex contract test (v0.3 plan M2, leader): the pinned, unmodified Codex runs with the config.toml
# PageLamp writes into its dedicated CODEX_HOME. CI runs it on Linux x64; locally,
# CODEX_BIN=<a codex binary> skips the download.
#
#   .github/scripts/codex-contract.sh config    # PageLamp's config.toml into $WORK/codex-home
#   .github/scripts/codex-contract.sh download  # the pinned asset: size, SHA-256, unzstd, --version
#   .github/scripts/codex-contract.sh check     # --strict-config load, no MCP servers, features off
#   .github/scripts/codex-contract.sh sandbox   # informational: the read-only sandbox with the bare binary
#   .github/scripts/codex-contract.sh pin       # print the parsed pin for $CODEX_TARGET
#
# No sign-in, no model call: nothing here needs a ChatGPT account.
set -euo pipefail

PIN=crates/pagelamp-llm/data/codex-pin.toml
TARGET=${CODEX_TARGET:-x86_64-unknown-linux-musl}
WORK=${WORK:-${RUNNER_TEMP:-$(mktemp -d)}}
CODEX_HOME_DIR="$WORK/codex-home"
BIN=${CODEX_BIN:-$WORK/codex}

fail() { echo "::error::codex-contract: $*" >&2; exit 1; }

# tag, version and the asset for $TARGET, from the pin (plain TOML; Python 3.10 has no tomllib).
pin() {
  python3 - "$PIN" "$TARGET" <<'PY'
import re, sys
text = open(sys.argv[1], encoding="utf-8").read()
target = sys.argv[2]
def get(key, block):
    m = re.search(r'^' + key + r'\s*=\s*"([^"]+)"', block, re.M)
    return m.group(1) if m else None
head = text.split("[[asset]]")[0]
for block in text.split("[[asset]]")[1:]:
    if get("target", block) == target:
        size = re.search(r'^size\s*=\s*(\d+)', block, re.M)
        print(f'TAG="{get("tag", head)}"')
        print(f'VERSION="{get("version", head)}"')
        print(f'NAME="{get("name", block)}"')
        print(f'SHA256="{get("sha256", block)}"')
        print(f'SIZE="{size.group(1) if size else ""}"')
        break
else:
    sys.exit(f"no [[asset]] for {target} in {sys.argv[1]}")
PY
}

case "${1:-}" in
  pin)
    pin
    ;;

  config)
    mkdir -p "$CODEX_HOME_DIR"
    CODEX_CONFIG_OUT="$CODEX_HOME_DIR" cargo test -p pagelamp-llm --lib --locked -- --ignored contract_config
    test -s "$CODEX_HOME_DIR/config.toml" || fail "contract_config wrote no config.toml"
    echo "--- PageLamp's config.toml"
    cat "$CODEX_HOME_DIR/config.toml"
    ;;

  download)
    eval "$(pin)"
    [ -n "$NAME" ] && [ -n "$SHA256" ] && [ -n "$SIZE" ] || fail "incomplete pin for $TARGET"
    curl --proto '=https' --tlsv1.2 -fsSL --retry 5 --retry-connrefused \
      -o "$WORK/codex.zst" "https://github.com/openai/codex/releases/download/$TAG/$NAME"
    size=$(wc -c < "$WORK/codex.zst" | tr -d ' ')
    [ "$size" = "$SIZE" ] || fail "$NAME is $size bytes, the pin says $SIZE"
    echo "$SHA256  $WORK/codex.zst" | sha256sum -c - || fail "SHA-256 of $NAME differs from the pin"
    zstd -q -d -f "$WORK/codex.zst" -o "$WORK/codex"
    chmod +x "$WORK/codex"
    got=$("$WORK/codex" --version)
    echo "$got"
    case "$got" in *" $VERSION"*) ;; *) fail "expected Codex $VERSION, got: $got" ;; esac
    ;;

  check)
    [ -s "$CODEX_HOME_DIR/config.toml" ] || fail "run 'config' first"
    export CODEX_HOME="$CODEX_HOME_DIR"
    # 1. The config loads strictly (unknown keys are errors). doctor loads it without signing in;
    #    its auth and network notes are expected here and ignored.
    out=$("$BIN" --strict-config doctor < /dev/null 2>&1 || true)
    echo "$out"
    if printf '%s\n' "$out" | grep -E -q 'Error loading config|config could not be loaded|✗ config'; then
      fail "Codex rejected PageLamp's config.toml under --strict-config"
    fi
    # 2. No MCP server is configured, so none can start.
    mcp=$("$BIN" mcp list < /dev/null 2>&1)
    echo "$mcp"
    printf '%s\n' "$mcp" | grep -F -q "No MCP servers configured" || fail "MCP servers are configured"
    # 3. Every feature PageLamp sets has that effective state in Codex's own view.
    "$BIN" features list < /dev/null > "$WORK/features.txt" 2>&1
    python3 - "$CODEX_HOME_DIR/config.toml" "$WORK/features.txt" <<'PY'
import re, sys
config = open(sys.argv[1], encoding="utf-8").read()
listing = open(sys.argv[2], encoding="utf-8").read()
m = re.search(r'^\[features\]\s*$(.*?)(?=^\[|\Z)', config, re.M | re.S)
if not m:
    sys.exit("config.toml has no [features] table")
wanted = dict(re.findall(r'^([A-Za-z0-9_]+)\s*=\s*(true|false)\s*$', m.group(1), re.M))
state = {}
for line in listing.splitlines():
    parts = line.split()
    if len(parts) >= 3 and parts[-1] in ("true", "false"):
        state[parts[0]] = parts[-1]
unknown = [k for k in sorted(wanted) if k not in state]
bad = [f"{k}: config {wanted[k]}, Codex says {state[k]}" for k in sorted(wanted) if k in state and state[k] != wanted[k]]
print(f"{len(wanted)} features checked")
if unknown:
    # Accepted by --strict-config (aliases or retired names) but not listed: they have no effect.
    print("::warning::codex-contract: config keys not in `codex features list` (no effect?): " + ", ".join(unknown))
if bad:
    sys.exit("feature states differ:\n  " + "\n  ".join(bad))
PY
    ;;

  sandbox)
    # Informational until A7 item 8 is settled: does `-s read-only` need bwrap with the bare binary?
    export CODEX_HOME="$CODEX_HOME_DIR"
    echo "bwrap on this runner: $(command -v bwrap || echo none)"
    if "$BIN" sandbox -- true < /dev/null; then
      echo "sandbox: true ran inside Codex's sandbox"
    else
      echo "::warning::codex-contract: Codex's sandbox did not run `true` with the bare binary"
      exit 1
    fi
    if "$BIN" sandbox -- sh -c 'echo x > "$HOME/.pagelamp-sandbox-probe"' < /dev/null 2>/dev/null; then
      rm -f "$HOME/.pagelamp-sandbox-probe"
      echo "::warning::codex-contract: a write to \$HOME succeeded inside the sandbox"
      exit 1
    fi
    echo "sandbox: a write to \$HOME was refused"
    ;;

  *)
    echo "usage: $0 pin|config|download|check|sandbox" >&2
    exit 2
    ;;
esac
