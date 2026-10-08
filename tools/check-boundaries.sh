#!/bin/sh
# Architectural checks intentionally use the workspace's standard Cargo and
# jq tooling plus POSIX shell and ripgrep; no custom Cargo plugin is required.
set -eu

fail() {
    echo "architecture boundary violation: $1" >&2
    exit 1
}

contains() {
    rg -q "$1" "$2"
}

# The workspace cutover is complete. A second tree would silently become a
# competing architecture even if Cargo does not compile it.
[ ! -e src ] || fail "legacy root src/ tree must not be reintroduced"
if rg -q 'src/(domain|application|protocol|infrastructure|bin)|src/gui\.rs|src/lib\.rs' \
    --glob '*.md' --glob '!docs/archive/**' \
    AGENTS.md README.md ARCHITECTURE.md docs; then
    fail "active guidance must not reference the retired root source tree"
fi

# Domain describes receiver concepts only. Protocol owns wire shapes; neither
# runtime nor serialization/network/filesystem dependencies may leak inward.
if contains '^(use|extern crate) .*\b(tokio|serde|quick_xml|iced|denon_avr_)|\b(TcpStream|UdpSocket)\b|std::(net|fs)' crates/domain/src; then
    fail "domain must be independent of runtime, I/O, serialization, and presentation"
fi

# Protocol may depend on domain but is deliberately unaware of application,
# adapters, and presentation.
if contains 'denon_avr_(application|infrastructure|gui)|crate::(application|infrastructure)' crates/protocol/src; then
    fail "protocol must not depend on application, infrastructure, or presentation"
fi

# Application depends only on the domain. Concrete adapter and wire imports
# here would bypass ports.
if contains 'denon_avr_(protocol|infrastructure)|crate::(protocol|infrastructure)' crates/application/src; then
    fail "application must not import protocol or infrastructure"
fi

# Delivery packages never parse receiver wire formats or construct concrete
# adapters. Composition is limited to the desktop executable.
if contains 'denon_avr_(protocol|infrastructure)' crates/gui-lib/src; then
    fail "GUI library must not import protocol or infrastructure"
fi
if contains 'denon_avr_protocol' apps/cli/src; then
    fail "CLI must use application ports rather than protocol encoding"
fi
# The CLI reaches the receiver only through the control-service port. It may
# compose the service, but it never constructs or names a session.
if contains '\b(X3800hSession|AvrSession|CanonicalReceiverSession)\b' apps/cli/src; then
    fail "CLI must not construct or name a receiver session"
fi

# Validate every normal workspace dependency edge from Cargo's resolved
# metadata. This catches renamed dependencies and new packages, neither of
# which manifest text matching can do reliably. Dev/build dependencies are
# excluded intentionally because they are not part of the runtime graph.
command -v jq >/dev/null 2>&1 || fail "jq is required to inspect Cargo metadata"
boundary_metadata=$(mktemp "${TMPDIR:-/tmp}/denon-boundaries-metadata.XXXXXX")
boundary_edges=$(mktemp "${TMPDIR:-/tmp}/denon-boundaries-edges.XXXXXX")
trap 'rm -f "$boundary_metadata" "$boundary_edges"' EXIT HUP INT TERM
cargo metadata --no-deps --format-version 1 >"$boundary_metadata"
jq -r '
        .packages[]
        | .name as $from
        | .dependencies[]
        | select(.kind == null and (.name | startswith("denon-avr-")))
        | "\($from):\(.name)"
    ' "$boundary_metadata" >"$boundary_edges"
while IFS=: read -r from to; do
    case "$from:$to" in
        denon-avr-protocol:denon-avr-domain | \
        denon-avr-policy:denon-avr-domain | \
        denon-avr-application:denon-avr-domain | \
        denon-avr-application:denon-avr-policy | \
        denon-avr-api-contract:denon-avr-application | \
        denon-avr-api-contract:denon-avr-domain | \
        denon-avr-api-client:denon-avr-api-contract | \
        denon-avr-api-client:denon-avr-application | \
        denon-avr-api-client:denon-avr-domain | \
        denon-avr-api-server:denon-avr-api-contract | \
        denon-avr-api-server:denon-avr-application | \
        denon-avr-api-server:denon-avr-domain | \
        denon-avr-api-server:denon-avr-infrastructure | \
        denon-avr-infrastructure:denon-avr-application | \
        denon-avr-infrastructure:denon-avr-domain | \
        denon-avr-infrastructure:denon-avr-policy | \
        denon-avr-infrastructure:denon-avr-protocol | \
        denon-avr-gui-lib:denon-avr-application | \
        denon-avr-gui-lib:denon-avr-domain | \
        denon-avr-cli:denon-avr-application | \
        denon-avr-cli:denon-avr-domain | \
        denon-avr-cli:denon-avr-infrastructure | \
        denon-avr-desktop:denon-avr-application | \
        denon-avr-desktop:denon-avr-domain | \
        denon-avr-desktop:denon-avr-gui-lib | \
        denon-avr-desktop:denon-avr-infrastructure | \
        denon-avr-diagnostics:denon-avr-domain | \
        denon-avr-diagnostics:denon-avr-infrastructure | \
        denon-avr-diagnostics:denon-avr-protocol) ;;
        *) fail "forbidden normal workspace dependency: $from -> $to" ;;
    esac
done <"$boundary_edges"

# The policy engine is a pure function over the domain: no async runtime,
# serialization, filesystem, network, or clock reads. The caller supplies the
# time and the ledger, so a decision can be reproduced from its inputs.
if contains '\b(async|tokio|serde)\b|\.await|\b(SystemTime|Instant)\b|std::(fs|net|io|process|thread)|\b(File|TcpStream|UdpSocket)\b' crates/policy/src; then
    fail "policy must be pure: no async, serialization, filesystem, network, or clock reads"
fi

# The edge check above sees direct dependencies only. The resolved graph also
# catches a package reached through another, so policy must reach the domain and
# nothing else.
policy_graph=$(cargo tree -p denon-avr-policy --edges normal --prefix none --format '{p}' | awk '{print $1}' | sort -u | tr '\n' ' ')
[ "$policy_graph" = "denon-avr-domain denon-avr-policy " ] || fail "policy must resolve to the domain only, found: $policy_graph"

# The wire contract is data and conversions: no HTTP, no async runtime, no
# filesystem. The transport is the server's and the client's.
if contains '\b(axum|hyper|tower|tokio)\b|\bhttp::|std::fs' crates/api-contract/src; then
    fail "api-contract must not name an HTTP stack, an async runtime, or the filesystem"
fi

# Neither the receiver adapters nor the wire protocols may be reached from the
# contract, directly or through another package.
contract_graph=$(cargo tree -p denon-avr-api-contract --edges normal --prefix none --format '{p}' | awk '{print $1}' | sort -u | tr '\n' ' ')
case "$contract_graph" in
    *denon-avr-infrastructure* | *denon-avr-protocol*) fail "api-contract must not reach infrastructure or protocol, found: $contract_graph" ;;
esac

# The client is small enough to run inside an agent's process: it speaks plain
# HTTP/1.1 over a Unix socket, so no TLS stack and no server framework may be in
# its graph, and it reaches neither the receiver adapters nor the wire protocols.
client_graph=$(cargo tree -p denon-avr-api-client --edges normal --prefix none --format '{p}' | awk '{print $1}' | sort -u | tr '\n' ' ')
for forbidden in rustls native-tls openssl security-framework ring aws-lc-rs webpki \
    axum warp actix-web rocket tower-http poem salvo denon-avr-infrastructure denon-avr-protocol; do
    case " $client_graph" in
        *" $forbidden "*) fail "api-client must not reach $forbidden, found: $client_graph" ;;
    esac
done

# The kinds are `normal,features`: `features` alone also walks dev-dependencies,
# and a test-only fake server must not trip this rule. The probe is run on the
# server as well, which does enable `server`, so a change in how Cargo prints
# features cannot make the rule pass by finding nothing.
server_features='^(hyper|hyper-util) feature "(server|server-auto|server-graceful)"'
cargo tree -p denon-avr-api-server --edges normal,features --prefix none | rg -q "$server_features" ||
    fail "the feature probe for the client's graph found no server feature on api-server"
if cargo tree -p denon-avr-api-client --edges normal,features --prefix none | rg -q "$server_features"; then
    fail "api-client must not enable the server side of hyper or hyper-util"
fi

# The server and the client are built for Unix only, and say so when they are not.
for root in apps/api-server/src/lib.rs crates/api-client/src/lib.rs; do
    contains '#\[cfg\(not\(unix\)\)\]' "$root" || fail "$root must refuse to build on a platform that is not Unix"
done

# The canonical receiver-session contract has one definition, in the
# application crate's session module, and nothing else defines a receiver
# session. Counting matching files alone would miss a duplicate placed next to
# the original.
session_contract=$(rg -n '^pub trait CanonicalReceiverSession\b' crates --glob '*.rs')
[ "$(printf '%s\n' "$session_contract" | wc -l | tr -d ' ')" -eq 1 ] || fail "the receiver-session contract must have exactly one definition"
if printf '%s\n' "$session_contract" | rg -q -v '^crates/application/src/session_v3\.rs:'; then
    fail "the receiver-session contract must be defined only in the application session module"
fi

# The legacy controller path was retired in version 4. Its names must not come
# back: a second session contract or coordinator would compete with the control
# service, which is the one owner of receiver sessions.
legacy_names=$(rg -n '\b(ReceiverController|ControllerHandle|SessionFactory|CanonicalSessionFactory|CanonicalSessionAdapter|AsyncStatusGateway|AsyncControlGateway)\b|^pub (enum|trait) (SessionEvent|ReceiverSession)\b' crates apps --glob '*.rs' || true)
[ -z "$legacy_names" ] || fail "retired legacy session path must not be reintroduced: $legacy_names"

# The Operation Gate in the control service is the only caller of a session's
# `operate` outside tests and the session implementation.
operate_callers=$(rg -l '\.operate\(' --glob '*.rs' --glob '!**/tests/**' crates apps |
    rg -v '^(crates/application/src/service\.rs|crates/infrastructure/src/x3800h_session\.rs)$' || true)
[ -z "$operate_callers" ] || fail "only the Operation Gate may call a session's operate: $operate_callers"

echo "architecture boundaries OK"
