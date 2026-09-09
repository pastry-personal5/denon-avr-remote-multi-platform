#!/usr/bin/env bash
set -euo pipefail

captures=${1:?usage: capture-visual-baselines.sh CAPTURE-DIRECTORY}
scenarios=(connected source-picker unavailable settings receivers diagnostics messages)
scales=(100 200)

for scenario in "${scenarios[@]}"; do
  for scale in "${scales[@]}"; do
    echo "capturing scenario=${scenario} scale=${scale}%"
    DENON_AVR_CAPTURE_DIR="$captures" \
    DENON_AVR_CAPTURE_SCENARIO="$scenario" \
    DENON_AVR_CAPTURE_SCALE="$scale" \
      cargo run -p denon-avr-desktop --bin denon-avr-remote-gui
  done
done
