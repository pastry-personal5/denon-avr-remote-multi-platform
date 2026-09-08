#!/bin/sh
set -eu

files="src/domain/*.rs src/application/ports.rs src/application/receiver_selection.rs src/application/main_zone_status.rs src/protocol/*.rs src/protocol/avr/*.rs"
for file in $files; do
    if grep -nEi "tokio|serde|ApplicationService|crate::infrastructure|TcpStream|UdpSocket|std::net|std::fs" "$file"; then
        echo "layer boundary violation: $file" >&2
        exit 1
    fi
done

echo "layer boundaries OK"
