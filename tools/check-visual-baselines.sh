#!/usr/bin/env bash
set -euo pipefail

platform=${1:?usage: check-visual-baselines.sh PLATFORM CAPTURE-DIRECTORY}
captures=${2:?usage: check-visual-baselines.sh PLATFORM CAPTURE-DIRECTORY}
root="tests/visual-baselines/${platform}"

case "$platform" in
  macos|windows|linux) ;;
  *) echo "PLATFORM must be macos, windows, or linux" >&2; exit 2 ;;
esac

if [[ ! -d "$captures" ]]; then
  echo "capture directory does not exist: $captures" >&2
  exit 2
fi

required=(
  connected-100pct.png connected-200pct.png
  source-picker-100pct.png source-picker-200pct.png
  unavailable-100pct.png unavailable-200pct.png
  settings-100pct.png settings-200pct.png
  receivers-100pct.png receivers-200pct.png
  diagnostics-100pct.png diagnostics-200pct.png
  messages-100pct.png messages-200pct.png
)

failed=0
for file in "${required[@]}"; do
  baseline="$root/$file"
  capture="$captures/$file"
  if [[ ! -f "$baseline" ]]; then
    echo "missing committed baseline: $baseline" >&2
    failed=1
  elif [[ ! -f "$capture" ]]; then
    echo "missing native capture: $capture" >&2
    failed=1
  elif ! cmp -s "$baseline" "$capture"; then
    echo "visual regression: $file differs from $baseline" >&2
    failed=1
  fi
done

if (( failed )); then
  exit 1
fi

echo "visual baselines match for $platform"
