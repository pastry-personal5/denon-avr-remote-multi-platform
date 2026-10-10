#!/bin/bash
# Spike S5: build the test bundles. See docs/research/local-network-permission-server-macos.md.
#
# Each bundle has the shape the macOS package will have. Its main executable is `s5-app`,
# a small AppKit application that stands in for the GUI (the first S5 run used a plain
# executable instead, which could not tell us whether a real app is asked for permission).
# Beside it in Contents/MacOS are `s5-host`, which starts the nested server and checks that
# it can reach the receiver, and the real `denon-avr-api-server`. The bundle is signed in one
# of three ways:
#
#   deep  codesign --force --deep, as tools/package-macos.sh does today
#   same  the nested programs signed first with the bundle's identifier, then the bundle
#   own   the server signed first with its own identifier (<bundle id>.server), the host
#         with the bundle's, then the bundle
#
# Usage:
#   bash tools/s5/build-bundles.sh                      own and same, under a fresh bundle id
#   bash tools/s5/build-bundles.sh --cases same,own     the cases to build, in the order to run them
#   bash tools/s5/build-bundles.sh --rebuild            the last fresh build's id, tag and cases again
#   bash tools/s5/build-bundles.sh --cases own --copy   one case, and a byte-identical copy of its
#                                                       bundle in target/s5-copy (the install-flow test)
#   bash tools/s5/build-bundles.sh --production same --tag ps   one case, under the real id
#                                                       com.denonavr.remote (a tag is required)
#   bash tools/s5/build-bundles.sh --id ID --tag TAG    an id of your own choosing; --tag is
#                                                       required unless ID is one this script printed
#                                                       (com.denonavr.s5.<14 digits>)
#
# The build ends by printing the base id, the tag and the `open` lines, in the order to run
# them. Run exactly those lines. `deep` is built only if --cases names it.
#
# A fresh id has never been granted Local Network permission. --rebuild builds again with
# the same identities and a new code hash AND a new Mach-O UUID for every program in the
# bundle (see `version` and `assemble` below), as a new release has, which is how the run
# checks whether a grant survives an update. It reads the id from target/s5/last-build.txt,
# so nobody retypes it.
#
# The bundles are named S5-<case>-<tag>.app. System Settings lists apps by name, and the
# first runs showed that a grant can follow a name, so a fresh id also gets a fresh name:
# the tag is the last six digits of a printed id unless --tag says otherwise. An id that is
# not a printed one, or --production, without a tag would reuse the names S5-<case>.app
# that earlier runs used, so it is refused.
#
# Environment: S5_OUT (default target/s5), S5_ROOT (default /tmp/s5), S5_RECEIVER_HOST
# (written to S5_ROOT/config.txt), S5_MAIN=plain (use s5-host as the main executable, as
# the first run did), S5_SAME_BYTES=1 (leave the programs as built: no per-build run-path and
# no new UUID, so that bundles share the programs' UUIDs as built, as runs 1 to 5 had them),
# S5_USAGE_TEXT=1 (add NSLocalNetworkUsageDescription), S5_SKIP_BUILD=1
# with S5_SERVER_BIN, S5_HOST_BIN and S5_APP_BIN (use programs already built; S5_HOST_BIN
# must be a host that knows --retag-uuid).
set -euo pipefail

usage() {
  sed -n '2,/^set -euo/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//' >&2
  exit 2
}

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "s5: macOS is required (Darwin host)" >&2
  exit 1
fi
command -v codesign >/dev/null 2>&1 || { echo "s5: codesign not found" >&2; exit 1; }
command -v install_name_tool >/dev/null 2>&1 || { echo "s5: install_name_tool not found (Xcode command line tools)" >&2; exit 1; }

script_dir=$(cd "$(dirname "$0")" && pwd -P)
root_dir=$(cd "$script_dir/../.." && pwd -P)
out_dir=${S5_OUT:-"$root_dir/target/s5"}
s5_root=${S5_ROOT:-/tmp/s5}
main_kind=${S5_MAIN:-appkit}
target_triple=aarch64-apple-darwin
production_id=com.denonavr.remote

[[ "$main_kind" == appkit || "$main_kind" == plain ]] || usage

fail() {
  echo "s5: $*" >&2
  exit 2
}

base_id=""
production=""
tag=""
cases=""
rebuild=""
copy=""
while [[ $# -gt 0 ]]; do
  case $1 in
    --id) base_id=${2:?--id needs a value}; shift 2 ;;
    --production) production=${2:?--production needs deep, same or own}; shift 2 ;;
    --tag) tag=${2:?--tag needs letters and digits}; shift 2 ;;
    --cases) cases=${2:?--cases needs a list such as own,same}; shift 2 ;;
    --rebuild) rebuild=1; shift ;;
    --copy) copy=1; shift ;;
    *) usage ;;
  esac
done
[[ -z "$tag" || "$tag" =~ ^[A-Za-z0-9]+$ ]] || usage

# --rebuild: the last fresh build's id, tag and cases, so that nobody types an id again.
last_build="$out_dir/last-build.txt"
if [[ -n "$rebuild" ]]; then
  [[ -z "$base_id" && -z "$production" && -z "$tag" ]] ||
    fail "--rebuild takes no --id, --production or --tag: it reuses the last fresh build's"
  [[ -s "$last_build" ]] ||
    fail "--rebuild: no earlier fresh build is recorded in $last_build; build once without arguments first"
  base_id=$(sed -n 's/^id=//p' "$last_build")
  tag=$(sed -n 's/^tag=//p' "$last_build")
  [[ -n "$cases" ]] || cases=$(sed -n 's/^cases=//p' "$last_build")
  [[ -n "$base_id" && -n "$tag" && -n "$cases" ]] || fail "$last_build is damaged; build once without arguments"
fi

if [[ -n "$production" ]]; then
  case $production in
    deep | same | own) ;;
    *) usage ;;
  esac
  [[ -z "$cases" ]] || fail "--production takes the one case itself (--production same), not --cases"
  [[ -n "$tag" ]] ||
    fail "--production needs --tag (for example --tag ps): the names S5-$production.app were used by earlier runs, and the system may remember a name"
  cases=$production
else
  if [[ -z "$base_id" ]]; then
    base_id=com.denonavr.s5.$(date +%Y%m%d%H%M%S)
  elif [[ -z "$tag" && ! "$base_id" =~ ^com\.denonavr\.s5\.[0-9]{14}$ ]]; then
    fail "--id '$base_id' is not an id this script printed (com.denonavr.s5.<14 digits>). Without a tag the bundles would be named S5-own.app and so on, which earlier runs used, and the system may remember those names. Copy the id exactly as printed, or use --rebuild for the last build, or add --tag."
  fi
  [[ -n "$cases" ]] || cases=own,same
  if [[ -z "$tag" ]]; then
    tag=${base_id: -6}
  fi
fi

# The cases, in the order they will be run: no duplicates, only the three known.
variants=""
IFS=, read -r -a case_list <<<"$cases"
[[ ${#case_list[@]} -gt 0 ]] || fail "--cases is empty"
for variant in "${case_list[@]}"; do
  case $variant in
    deep | same | own) ;;
    *) fail "--cases: '$variant' is not deep, same or own" ;;
  esac
  case " $variants " in
    *" $variant "*) fail "--cases names '$variant' twice" ;;
  esac
  variants="$variants $variant"
done
variants=${variants# }
cases_csv=${variants// /,}

# --copy: a byte-identical copy of one fresh bundle, at another path under the same file name,
# as when an app is run from a mounted disk image and then from /Applications.
if [[ -n "$copy" ]]; then
  [[ -z "$rebuild" && -z "$production" ]] || fail "--copy is for a fresh build: not with --rebuild or --production"
  [[ ${#case_list[@]} -eq 1 ]] || fail "--copy needs exactly one case (for example --cases own), not '$cases_csv'"
fi
copy_dir="$out_dir-copy"

suffix=""
[[ -z "$tag" ]] || suffix="-$tag"

mkdir -p "$out_dir" "$s5_root"

if [[ -z "${S5_SKIP_BUILD:-}" ]]; then
  command -v cargo >/dev/null 2>&1 || { echo "s5: cargo not found" >&2; exit 1; }
  if ! target_libdir=$(rustc --print target-libdir --target "$target_triple" 2>/dev/null) || [[ ! -d "$target_libdir" ]]; then
    echo "s5: Rust target $target_triple is not installed (rustup target add $target_triple)" >&2
    exit 1
  fi
  echo "Building denon-avr-api-server for $target_triple..."
  cargo build --manifest-path "$root_dir/Cargo.toml" --release --target "$target_triple" \
    -p denon-avr-api-server --bin denon-avr-api-server
  echo "Building s5-host..."
  CARGO_TARGET_DIR="$out_dir/host-target" cargo build --manifest-path "$script_dir/host/Cargo.toml" \
    --release --target "$target_triple"
  server_bin="${CARGO_TARGET_DIR:-$root_dir/target}/$target_triple/release/denon-avr-api-server"
  host_bin="$out_dir/host-target/$target_triple/release/s5-host"
  app_bin=""
  if [[ "$main_kind" == appkit ]]; then
    command -v xcrun >/dev/null 2>&1 || { echo "s5: xcrun (Xcode command line tools) not found" >&2; exit 1; }
    echo "Building s5-app (AppKit)..."
    mkdir -p "$out_dir/app-build"
    xcrun swiftc -O -swift-version 5 -module-cache-path "$out_dir/app-build/modules" \
      -o "$out_dir/app-build/s5-app" "$script_dir/app/main.swift"
    app_bin="$out_dir/app-build/s5-app"
  fi
else
  server_bin=${S5_SERVER_BIN:?S5_SKIP_BUILD needs S5_SERVER_BIN}
  host_bin=${S5_HOST_BIN:?S5_SKIP_BUILD needs S5_HOST_BIN}
  app_bin=${S5_APP_BIN:-}
  [[ "$main_kind" == plain || -n "$app_bin" ]] || { echo "s5: S5_SKIP_BUILD needs S5_APP_BIN (or S5_MAIN=plain)" >&2; exit 1; }
fi
[[ -x "$server_bin" ]] || { echo "s5: no server binary at $server_bin" >&2; exit 1; }
[[ -x "$host_bin" ]] || { echo "s5: no host binary at $host_bin" >&2; exit 1; }
[[ "$main_kind" == plain || -x "$app_bin" ]] || { echo "s5: no app binary at $app_bin" >&2; exit 1; }

# A different value every run. It reaches the main executable's signature through
# Info.plist, and `assemble` also puts it into the nested programs (a run-path load
# command) and gives every program in the bundle a UUID of its own, so that two builds of
# one identity differ in every program's code hash and UUID, as two releases do, and so that
# no two bundles share a program's UUID (runs 1 to 5 had bundles that did).
version=$(date +%s)

# A bundle that is not finished (unsigned, or without its UUIDs) must not be left where it can
# be opened by mistake, whatever stops the build.
building=""
remove_unfinished() {
  [[ -z "$building" ]] || rm -rf "$building"
}
trap remove_unfinished EXIT

usage_text=""
if [[ -n "${S5_USAGE_TEXT:-}" ]]; then
  usage_text="  <key>NSLocalNetworkUsageDescription</key><string>S5 test: connects to the receiver on the local network.</string>"
fi

if [[ "$main_kind" == appkit ]]; then
  main_exec=s5-app
else
  main_exec=s5-host
fi

assemble() {
  local variant=$1 id=$2
  local app="$out_dir/S5-$variant$suffix.app"
  local macos="$app/Contents/MacOS"
  rm -rf "$app"
  building="$app"
  mkdir -p "$macos"
  cp "$host_bin" "$macos/s5-host"
  cp "$server_bin" "$macos/denon-avr-api-server"
  [[ "$main_kind" == appkit ]] && cp "$app_bin" "$macos/s5-app"
  chmod 755 "$macos"/*
  # A real update changes the programs themselves (see `version` above). Only these copies in
  # the bundle are touched. Both steps invalidate the signature; every variant below signs
  # again. S5_SAME_BYTES=1 leaves the programs as built, as runs 1 to 5 had them.
  if [[ -z "${S5_SAME_BYTES:-}" ]]; then
    local nested changed
    for nested in s5-host denon-avr-api-server; do
      changed=$(install_name_tool -add_rpath "/s5-build-$version" "$macos/$nested" 2>&1) || {
        echo "s5: cannot make $nested differ for this build: $changed" >&2
        exit 1
      }
    done
    for nested in s5-host denon-avr-api-server s5-app; do
      [[ -e "$macos/$nested" ]] || continue
      changed=$("$host_bin" --retag-uuid "$macos/$nested" 2>&1) || {
        echo "s5: cannot give $nested a UUID of its own: $changed" >&2
        echo "s5: is $host_bin an s5-host that knows --retag-uuid? Build it again." >&2
        exit 1
      }
    done
  fi
  cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDisplayName</key><string>S5 $variant$suffix</string>
  <key>CFBundleExecutable</key><string>$main_exec</string>
  <key>CFBundleIdentifier</key><string>$id</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>S5 $variant$suffix</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.0.$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>CFBundleSupportedPlatforms</key><array><string>MacOSX</string></array>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSPrincipalClass</key><string>NSApplication</string>
$usage_text
</dict>
</plist>
PLIST
  case $variant in
    deep)
      codesign --force --deep --sign - --identifier "$id" "$app"
      ;;
    same)
      codesign --force --sign - --identifier "$id" "$macos/denon-avr-api-server"
      [[ "$main_kind" == appkit ]] && codesign --force --sign - --identifier "$id" "$macos/s5-host"
      codesign --force --sign - --identifier "$id" "$app"
      ;;
    own)
      codesign --force --sign - --identifier "$id.server" "$macos/denon-avr-api-server"
      [[ "$main_kind" == appkit ]] && codesign --force --sign - --identifier "$id" "$macos/s5-host"
      codesign --force --sign - --identifier "$id" "$app"
      ;;
  esac
  codesign --verify --strict --verbose=2 "$app"
  echo "  $app"
  local binary identifier uuid
  for binary in "$main_exec" s5-host denon-avr-api-server; do
    [[ -e "$macos/$binary" ]] || continue
    [[ "$binary" == s5-host && "$main_exec" == s5-host ]] && continue
    identifier=$(codesign -dvvv "$macos/$binary" 2>&1 | sed -n 's/^Identifier=//p')
    uuid=$("$host_bin" --print-uuid "$macos/$binary" 2>/dev/null) || uuid="(UUID unreadable)"
    printf '    %-22s %-46s %s\n' "$binary" "$identifier" "$uuid"
  done
  building=""
}

echo
echo "Bundles (main program: $main_exec, code version $version):"
for variant in $variants; do
  if [[ -n "$production" ]]; then
    id=$production_id
  else
    id="$base_id.$variant"
  fi
  assemble "$variant" "$id"
done

# The copy is made after signing, so it is byte for byte the original: same id, same UUIDs.
copy_app="$copy_dir/S5-$variants$suffix.app"
if [[ -n "$copy" ]]; then
  mkdir -p "$copy_dir"
  rm -rf "$copy_app"
  building="$copy_app"
  ditto "$out_dir/S5-$variants$suffix.app" "$copy_app"
  codesign --verify --strict "$copy_app"
  building=""
  echo "  copy: $copy_app"
fi

if [[ -n "${S5_RECEIVER_HOST:-}" ]]; then
  printf 'host=%s\n' "$S5_RECEIVER_HOST" > "$s5_root/config.txt"
elif [[ ! -s "$s5_root/config.txt" ]]; then
  if [[ -t 0 ]]; then
    read -r -p "Receiver IP address (written to $s5_root/config.txt): " receiver_host
    printf 'host=%s\n' "$receiver_host" > "$s5_root/config.txt"
  else
    echo "s5: no receiver address yet; write 'host=<address>' to $s5_root/config.txt" >&2
  fi
fi

# Only a build that got this far is remembered, and only a fresh one (not --production).
if [[ -z "$production" ]]; then
  {
    printf 'id=%s\n' "$base_id"
    printf 'tag=%s\n' "$tag"
    printf 'cases=%s\n' "$cases_csv"
    printf 'built=%s\n' "$(date '+%Y-%m-%d %H:%M:%S')"
  } > "$last_build"
fi

echo
echo "=================================================================="
if [[ -n "$production" ]]; then
  echo "bundle id  $production_id   (the real one; the installed app has it too)"
else
  echo "base id    $base_id"
fi
echo "tag        $tag"
echo "cases      ${variants// /, }"
echo "=================================================================="
echo "Run exactly these lines, in this order, one at a time. Each ends when its window says"
echo "FINISHED. Write down the exact name on any dialog, and use the buttons in the window."
if [[ -n "$copy" ]]; then
  original="$out_dir/S5-$variants$suffix.app"
  echo "  1. open -W \"$copy_app\""
  echo "       # the COPY, as an app run from a mounted disk image"
  echo "  2. open -W \"$original\""
  echo "       # the original, while the copy still exists"
  echo "  3. rm -rf \"$copy_app\""
  echo "       # delete the copy, as ejecting the disk image does"
  echo "  4. open -W \"$original\""
  echo "       # the original again, with the copy gone"
else
  n=0
  for variant in $variants; do
    n=$((n + 1))
    echo "  $n. open -W \"$out_dir/S5-$variant$suffix.app\""
  done
fi
echo "Read a report with:  grep -E '^(PASS|FAIL|WARN|NOTE|SUMMARY)' $s5_root/result-<case>$suffix.txt"
if [[ -z "$production" ]]; then
  echo "To build again under these same ids (a rebuild):  bash tools/s5/build-bundles.sh --rebuild"
fi
