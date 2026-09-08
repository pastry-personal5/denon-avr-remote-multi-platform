#!/bin/sh
set -eu

files="crates/domain/src/*.rs crates/application/src/ports.rs crates/application/src/receiver_selection.rs crates/application/src/main_zone_status.rs crates/protocol/src/*.rs crates/protocol/src/avr/*.rs"
for file in $files; do
    if grep -nEi "tokio|serde|ApplicationService|crate::infrastructure|TcpStream|UdpSocket|std::net|std::fs" "$file"; then
        echo "layer boundary violation: $file" >&2
        exit 1
    fi
done

# Package-level dependency direction checks.
if grep -q '^denon-avr-' crates/domain/Cargo.toml; then
    echo "domain must not declare external dependencies" >&2
    exit 1
fi
if sed '/^\[dev-dependencies\]/,$d' crates/gui-lib/Cargo.toml | grep -q 'denon-avr-infrastructure'; then
    echo "GUI package has a forbidden infrastructure dependency" >&2
    exit 1
fi
if grep -q 'denon-avr-infrastructure\|denon-avr-protocol' crates/application/Cargo.toml; then
    echo "application package has a forbidden dependency" >&2
    exit 1
fi

echo "layer boundaries OK"
