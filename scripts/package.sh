#!/bin/bash
# Builds the release archives in dist/ (nothing is published, nothing is
# uploaded; a public release happens only when the owner says so):
#   baba-port-<v>-linux-x86_64.tar.gz    scripts/package/linux.sh
#   baba-port-<v>-windows-x86_64.zip     scripts/package/windows.sh (llvm-mingw)
#   baba-port-<v>-android-arm64.apk      scripts/package/android.sh
#                                        (-debugkey.apk without the release key)
#   baba-port-<v>-portmaster.zip         scripts/package/portmaster.sh
#   baba-port-<v>-macos-universal.zip    CI only (scripts/package/macos.sh on macOS)
#   SHA256SUMS                           over every archive in dist/
# then checks them: nothing from the game inside (scripts/check-no-game-data.sh
# --archives), names and layout against the Warpzone descriptor
# (packaging/warpzone/baba-port.toml: [install] assets, [launch] exec), and
# with --verify unpacks and runs each one headless (scripts/package/verify.sh).
# <v> is [workspace.package] version in Cargo.toml (CI: must equal the tag).
#
# Usage: scripts/package.sh [--verify] [--no-build] [--bundle-sdl]
#                           [--only linux,windows,android,portmaster]
#                           [--checks-only]
#   --bundle-sdl   Linux: ship $BABA_SDL2_LIB_DIR/libSDL2-2.0.so.0 (CI)
#   --checks-only  no builds: SHA256SUMS + checks over what is in dist/
#                  (the release workflow, after collecting every platform)
# CPU-heavy (four release builds): run it under perf-mode.
set -euo pipefail
source "$(dirname "$0")/../env.sh"
cd "$BABA_ROOT"
source scripts/package/common.sh
VERIFY=0 NOBUILD=() BUNDLE=() ONLY="linux,windows,android,portmaster" CHECKS_ONLY=0
while [ $# -gt 0 ]; do
    case $1 in
        --verify) VERIFY=1 ;;
        --no-build) NOBUILD=(--no-build) ;;
        --bundle-sdl) BUNDLE=(--bundle-sdl) ;;
        --only) ONLY=$2; shift ;;
        --only=*) ONLY=${1#--only=} ;;
        --checks-only) CHECKS_ONLY=1 ;;
        *) echo "unknown option $1" >&2; exit 2 ;;
    esac
    shift
done
VERSION=$(pkg_version)
[ -n "$VERSION" ] || { echo "cannot read the version from Cargo.toml" >&2; exit 1; }
mkdir -p "$DIST"
want() { [[ ",$ONLY," == *",$1,"* ]]; }

if [ $CHECKS_ONLY = 0 ]; then
    # Leftovers of other versions would end up in SHA256SUMS.
    find "$DIST" -maxdepth 1 -name 'baba-port-*' ! -name "baba-port-$VERSION-*" -delete
    rm -f "$DIST/SHA256SUMS"
    # (if/fi, not `want x && ...`: a failing build must stop the script)
    if want linux; then scripts/package/linux.sh "${NOBUILD[@]}" "${BUNDLE[@]}"; fi
    if want windows; then scripts/package/windows.sh "${NOBUILD[@]}"; fi
    if want android; then scripts/package/android.sh "${NOBUILD[@]}"; fi
    if want portmaster; then scripts/package/portmaster.sh "${NOBUILD[@]}"; fi
    if want macos; then scripts/package/macos.sh "${NOBUILD[@]}"; fi
fi

shopt -s nullglob
files=()
for f in "$DIST"/baba-port-"$VERSION"-*; do
    case $f in *.tar.gz|*.zip|*.apk) files+=("$(basename "$f")") ;; esac
done
[ ${#files[@]} -gt 0 ] || { echo "no release archives in $DIST" >&2; exit 1; }
(cd "$DIST" && sha256sum "${files[@]}" > SHA256SUMS && sha256sum -c --quiet SHA256SUMS)

# Names and layout against the descriptor's asset globs and exec paths.
"$PY" -I - "$DIST" "$VERSION" packaging/warpzone/baba-port.toml <<'PY'
import fnmatch, os, re, sys, tarfile, zipfile
dist, ver, desc = sys.argv[1:]
toml = open(desc).read()
def table(key):
    m = re.search(rf"^{key}\s*=\s*\{{(.*)\}}\s*$", toml, re.M)
    return dict(re.findall(r'(\w+)\s*=\s*"([^"]+)"', m.group(1)))
assets, execs = table("assets"), table("exec")
checksums = re.search(r'^checksums\s*=\s*"([^"]+)"', toml, re.M).group(1)
files = sorted(f for f in os.listdir(dist) if f.startswith(f"baba-port-{ver}-"))
bad = 0
for plat, glob in assets.items():
    hits = [f for f in files if fnmatch.fnmatch(f, glob)]
    if not hits:
        print(f"{plat}: no archive matches {glob!r} (not built)")
        continue
    if len(hits) > 1:
        print(f"{plat}: {glob!r} matches several archives: {hits}"); bad += 1
    f = hits[0]
    if plat not in execs:
        print(f"{plat}: {f}: asset glob {glob!r} ok (download only)")
        continue
    p = os.path.join(dist, f)
    names = zipfile.ZipFile(p).namelist() if f.endswith(".zip") else \
        [m.name for m in tarfile.open(p).getmembers()]
    # `*` matches one path segment.
    rx = re.compile("^" + re.escape(execs[plat]).replace(r"\*", "[^/]*") + "$")
    ok = [n for n in names if rx.match(n)]
    print(f"{plat}: {f}: asset glob {glob!r} ok, exec {execs[plat]!r} -> {ok[0] if ok else 'MISSING'}")
    bad += not ok
sums = open(os.path.join(dist, checksums)).read()
for f in files:
    if f.endswith((".tar.gz", ".zip", ".apk")) and f not in sums:
        print(f"{f} missing from {checksums}"); bad += 1
sys.exit(1 if bad else 0)
PY

scripts/check-no-game-data.sh --archives "${files[@]/#/$DIST/}"

if [ $VERIFY = 1 ]; then
    for f in "${files[@]}"; do
        case $f in
            *-linux-x86_64.tar.gz) scripts/package/verify.sh linux "$DIST/$f" ;;
            *-windows-x86_64.zip) scripts/package/verify.sh windows "$DIST/$f" ;;
            *-portmaster.zip) scripts/package/verify.sh portmaster "$DIST/$f" ;;
            *-android-arm64*.apk) scripts/package/verify.sh android "$DIST/$f" ;;
        esac
    done
fi
echo
cat "$DIST/SHA256SUMS"
for f in "${files[@]}"; do printf '%-48s %s\n' "$f" "$(human "$DIST/$f")"; done
