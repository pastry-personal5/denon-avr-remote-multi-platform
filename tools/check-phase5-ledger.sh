#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ledger="$root_dir/docs/v3/phase-5-requirement-ledger.md"

test -f "$ledger" || {
  echo "missing Phase 5 requirement ledger: $ledger" >&2
  exit 1
}

for id in SMK-01 SMK-02 SMK-03 SMK-04 SMK-05 SMK-06 SMK-07 SMK-08 SMK-09 SMK-10; do
  rg -q "\| $id \|" "$ledger" || {
    echo "Phase 5 ledger is missing $id" >&2
    exit 1
  }
done

for fixture in \
  crates/infrastructure/tests/core_smoke.rs \
  crates/infrastructure/tests/receiver_scenarios.rs \
  crates/infrastructure/tests/receiver_properties.rs \
  crates/infrastructure/tests/x3800h_trace_replay.rs \
  crates/gui-lib/tests/async_apis.rs; do
  test -f "$root_dir/$fixture" || {
    echo "Phase 5 ledger fixture is missing: $fixture" >&2
    exit 1
  }
done

echo "Phase 5 requirement ledger OK"
