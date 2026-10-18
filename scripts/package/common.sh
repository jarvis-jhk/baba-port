# Shared helpers of scripts/package.sh and scripts/package/*.sh (sourced
# after env.sh and `cd "$BABA_ROOT"`). Nothing here publishes anything.

DIST="${BABA_DIST_DIR:-$BABA_ROOT/dist}"
PY=python3
[ -x "$BABA_ROOT/tools/venv/bin/python" ] && PY="$BABA_ROOT/tools/venv/bin/python"

# Release version: [workspace.package] version in Cargo.toml, which is also
# what the game shows (CARGO_PKG_VERSION, `baba-port --version`) and what a
# release tag must say (v<version>).
pkg_version() {
    sed -n '/^\[workspace\.package\]/,/^\[/{s/^version *= *"\(.*\)"/\1/p}' "$BABA_ROOT/Cargo.toml" | head -1
}

# Android versionCode from the version: MAJOR*10000 + MINOR*100 + PATCH
# (0.1.0 -> 100; must only ever grow).
pkg_version_code() {
    local v=${1%%-*} IFS=.
    set -- $v
    echo $(( ${1:-0} * 10000 + ${2:-0} * 100 + ${3:-0} ))
}

# Timestamp for reproducible archives: $SOURCE_DATE_EPOCH (CI: the commit
# time) or a fixed date.
pkg_epoch() { echo "${SOURCE_DATE_EPOCH:-1767225600}"; }

human() { wc -c < "$1" | awk '{ printf "%.2f MB", $1 / 1e6 }'; }

# pkg_clean_stage DIR: recreate an empty staging directory.
pkg_clean_stage() { rm -rf "$1"; mkdir -p "$1"; }

# pkg_stage_docs DIR TRIPLE [PACKAGE]: licences, notices, player readme.
pkg_stage_docs() {
    local dir=$1 triple=$2 package=${3:-baba-port}
    install -m 644 LICENSE-MIT LICENSE-APACHE THIRD_PARTY_NOTICES.md "$dir/"
    install -m 644 packaging/README-player.md "$dir/README.md"
    mkdir -p "$dir/licenses"
    "$PY" -I packaging/gen-licenses.py "$dir/licenses/RUST_DEPENDENCIES.txt" "$triple" "$package" >/dev/null
    chmod 644 "$dir/licenses/RUST_DEPENDENCIES.txt"
}

# pkg_tar STAGE NAME OUT: deterministic .tar.gz of STAGE/NAME (sorted, owner
# 0, fixed mtime; executables 755, the rest 644).
pkg_tar() {
    "$PY" -I - "$1" "$2" "$3" "$(pkg_epoch)" <<'PY'
import gzip, io, os, sys, tarfile
stage, name, out, epoch = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
buf = io.BytesIO()
with tarfile.open(fileobj=buf, mode="w", format=tarfile.GNU_FORMAT) as tf:
    root = os.path.join(stage, name)
    paths = [root]
    for dp, dn, fn in os.walk(root):
        dn.sort()
        paths += [os.path.join(dp, d) for d in dn] + [os.path.join(dp, f) for f in sorted(fn)]
    for p in sorted(paths, key=lambda p: os.path.relpath(p, stage)):
        ti = tf.gettarinfo(p, arcname=os.path.relpath(p, stage))
        ti.uid = ti.gid = 0
        ti.uname = ti.gname = ""
        ti.mtime = epoch
        if ti.isdir() or os.access(p, os.X_OK):
            ti.mode = 0o755
        else:
            ti.mode = 0o644
        if ti.isreg():
            with open(p, "rb") as fh:
                tf.addfile(ti, fh)
        else:
            tf.addfile(ti)
with open(out + ".part", "wb") as f:
    with gzip.GzipFile(filename="", mode="wb", fileobj=f, compresslevel=9, mtime=epoch) as gz:
        gz.write(buf.getvalue())
os.replace(out + ".part", out)
PY
}

# pkg_zip STAGE OUT [MEMBERS...]: deterministic zip of STAGE's contents
# (default: everything in STAGE), Unix modes kept (play scripts and the
# PortMaster binary stay executable).
pkg_zip() {
    local stage=$1 out=$2
    shift 2
    "$PY" -I - "$stage" "$out" "$(pkg_epoch)" "$@" <<'PY'
import os, sys, time, zipfile
stage, out, epoch = sys.argv[1], sys.argv[2], int(sys.argv[3])
members = sys.argv[4:] or sorted(os.listdir(stage))
dt = time.gmtime(max(epoch, 315532800))[:6]
files = []
for m in members:
    p = os.path.join(stage, m)
    if os.path.isdir(p):
        files.append(p)
        for dp, dn, fn in os.walk(p):
            dn.sort()
            files += [os.path.join(dp, d) for d in dn] + [os.path.join(dp, f) for f in sorted(fn)]
    else:
        files.append(p)
files.sort(key=lambda p: os.path.relpath(p, stage))
with zipfile.ZipFile(out + ".part", "w") as z:
    for p in files:
        rel = os.path.relpath(p, stage)
        if os.path.isdir(p):
            zi = zipfile.ZipInfo(rel + "/", dt)
            zi.external_attr = (0o40755 << 16) | 0x10
            z.writestr(zi, b"")
            continue
        zi = zipfile.ZipInfo(rel, dt)
        zi.create_system = 3
        zi.external_attr = (0o100755 if os.access(p, os.X_OK) else 0o100644) << 16
        zi.compress_type = zipfile.ZIP_DEFLATED
        with open(p, "rb") as fh:
            z.writestr(zi, fh.read(), compresslevel=9)
os.replace(out + ".part", out)
PY
}

# pkg_max_glibc BIN: newest non-weak GLIBC_x.y the binary needs.
pkg_max_glibc() {
    readelf -V "$1" | grep -E 'Name: GLIBC_[0-9.]+ +Flags: none' | grep -o 'GLIBC_[0-9.]*' \
        | sed 's/GLIBC_//' | sort -uV | tail -1
}

# pkg_check_glibc BIN MAX: fail if BIN needs a newer glibc than MAX.
pkg_check_glibc() {
    local need
    need=$(pkg_max_glibc "$1")
    if [ -n "$need" ] && [ "$(printf '%s\n%s\n' "$need" "$2" | sort -V | tail -1)" != "$2" ]; then
        echo "$1 needs glibc $need > $2" >&2
        return 1
    fi
    echo "glibc needed: ${need:-none} (limit $2)"
}
