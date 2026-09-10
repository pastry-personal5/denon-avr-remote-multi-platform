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
        denon-avr-application:denon-avr-domain | \
        denon-avr-infrastructure:denon-avr-application | \
        denon-avr-infrastructure:denon-avr-domain | \
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

# Session events and factory/session contracts have one owner and exactly one
# definition each. Counting matching files alone would miss duplicates placed
# together in ports.rs.
session_contracts=$(rg -n '^pub (enum|trait) (SessionEvent|ReceiverSession|SessionFactory)' crates --glob '*.rs')
[ "$(printf '%s\n' "$session_contracts" | wc -l | tr -d ' ')" -eq 3 ] || fail "session contracts must each have exactly one definition"
if printf '%s\n' "$session_contracts" | rg -q -v '^crates/application/src/ports\.rs:'; then
    fail "session contracts must be defined only in application ports"
fi

echo "architecture boundaries OK"
