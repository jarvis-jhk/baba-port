#!/usr/bin/env python3
"""Minimal APK builder for a Java-free NativeActivity app (stdlib + openssl CLI).

Replaces aapt2 + zipalign + apksigner (and with them the JDK and the SDK
build-tools) for an app that has no Java code and no compiled resources:

  * AndroidManifest.xml is written directly in Android's binary XML format
    (attribute resource IDs below were checked against android-35's
    android.jar with `aapt2 dump resources`, see docs/platform-android.md);
  * the zip is written with stored entries aligned like zipalign does
    (4 bytes; 16 KiB for uncompressed .so so they can be mapped in place on
    16 KiB-page devices);
  * it is signed with APK Signature Scheme v2 (enough for minSdk >= 24), using
    `openssl` for the key, the self-signed certificate and the signature.

Usage:
  apk.py build --out app.apk --package org.x --label X --lib-name foo \
      --so lib/arm64-v8a/libfoo.so=path/to/libfoo.so [--asset name=path ...] \
      --key-dir DIR [--version-code N] [--version-name S] [--min-sdk 26] \
      [--target-sdk 35] [--orientation sensorLandscape] [--extract-native-libs]
  apk.py verify app.apk      # re-checks the v2 signature (no apksigner needed)
  apk.py dump-manifest app.apk
"""
import argparse
import hashlib
import io
import os
import struct
import subprocess
import sys
import tempfile
import zipfile

ANDROID_NS = "http://schemas.android.com/apk/res/android"

# android.R.attr IDs (android-35 public.xml; verified with aapt2 dump resources).
ATTR_IDS = {
    "theme": 0x01010000, "label": 0x01010001, "icon": 0x01010002, "name": 0x01010003,
    "hasCode": 0x0101000C, "debuggable": 0x0101000F, "exported": 0x01010010,
    "launchMode": 0x0101001D, "screenOrientation": 0x0101001E, "configChanges": 0x0101001F,
    "value": 0x01010024, "mimeType": 0x01010026, "minSdkVersion": 0x0101020C, "versionCode": 0x0101021B,
    "versionName": 0x0101021C, "targetSdkVersion": 0x01010270, "glEsVersion": 0x01010281,
    "required": 0x0101028E, "isGame": 0x010103F4, "extractNativeLibs": 0x010104EA,
    "resizeableActivity": 0x010104F6, "appCategory": 0x01010545,
}
THEME_BLACK_NOTITLEBAR_FULLSCREEN = 0x0103000A  # @android:style/Theme.Black.NoTitleBar.Fullscreen
ORIENTATIONS = {"unspecified": -1, "landscape": 0, "portrait": 1, "user": 2, "sensor": 4,
                "sensorLandscape": 6, "sensorPortrait": 7, "fullSensor": 10, "userLandscape": 11,
                "fullUser": 13}
# mcc|mnc|locale|touchscreen|keyboard|keyboardHidden|navigation|orientation|screenLayout|
# uiMode|screenSize|smallestScreenSize|density|layoutDirection: never restart the activity.
CONFIG_CHANGES = 0x3FFF

# Res_value data types
T_REF, T_STRING, T_DEC, T_HEX, T_BOOL = 0x01, 0x03, 0x10, 0x11, 0x12


class Hex(int):
    pass


class Ref(int):
    pass


def manifest_tree(a):
    """The manifest as (tag, [(name, value, android_ns)], children)."""
    A = lambda name, v: (name, v, True)  # noqa: E731
    app_attrs = [A("label", a.label), A("hasCode", False), A("isGame", True),
                 A("appCategory", 0), A("extractNativeLibs", bool(a.extract_native_libs))]
    if a.debuggable:
        app_attrs.append(A("debuggable", True))
    activity = ("activity", [
        A("name", "android.app.NativeActivity"), A("label", a.label), A("exported", True),
        A("theme", Ref(THEME_BLACK_NOTITLEBAR_FULLSCREEN)),
        A("configChanges", Hex(CONFIG_CHANGES)), A("launchMode", 2),  # singleTask
        A("screenOrientation", ORIENTATIONS[a.orientation]), A("resizeableActivity", True),
    ], [
        ("meta-data", [A("name", "android.app.lib_name"), A("value", a.lib_name)], []),
        ("intent-filter", [], [
            ("action", [A("name", "android.intent.action.MAIN")], []),
            ("category", [A("name", "android.intent.category.LAUNCHER")], []),
        ]),
    ] + ([
        # "Open with" / "Share to" for files of these types (the game data
        # import, docs/platform-android.md); read through getIntent() + JNI.
        ("intent-filter", [], [
            ("action", [A("name", "android.intent.action.VIEW")], []),
            ("action", [A("name", "android.intent.action.SEND")], []),
            ("category", [A("name", "android.intent.category.DEFAULT")], []),
        ] + [("data", [A("mimeType", m)], []) for m in a.open_mime]),
    ] if a.open_mime else []))
    return ("manifest", [
        A("versionCode", int(a.version_code)), A("versionName", a.version_name),
        ("package", a.package, False),
    ], [
        ("uses-sdk", [A("minSdkVersion", int(a.min_sdk)), A("targetSdkVersion", int(a.target_sdk))], []),
    ] + [("uses-permission", [A("name", perm)], []) for perm in a.permission] + [
        ("uses-feature", [A("glEsVersion", Hex(0x20000)), A("required", True)], []),
        ("uses-feature", [A("name", "android.hardware.touchscreen"), A("required", False)], []),
        ("uses-feature", [A("name", "android.hardware.gamepad"), A("required", False)], []),
        ("application", app_attrs, [activity]),
    ])


def encode_axml(root):
    """Android binary XML (ResXMLTree) for one element tree, UTF-16 string pool."""
    attr_names, others = [], []

    def collect(node):
        tag, attrs, kids = node
        for name, v, ns in sorted(attrs, key=lambda t: ATTR_IDS.get(t[0], 1 << 32) if t[2] else 1 << 32):
            if ns:
                if name not in attr_names:
                    attr_names.append(name)
            elif name not in others:
                others.append(name)
            if isinstance(v, str) and v not in others:
                others.append(v)
        if tag not in others:
            others.append(tag)
        for k in kids:
            collect(k)

    collect(root)
    attr_names.sort(key=lambda n: ATTR_IDS[n])
    strings = attr_names + [s for s in ["android", ANDROID_NS] + others if s not in attr_names]
    strings = list(dict.fromkeys(strings))
    idx = {s: i for i, s in enumerate(strings)}

    # string pool
    data, offs = b"", []
    for s in strings:
        offs.append(len(data))
        u = s.encode("utf-16-le")
        data += struct.pack("<H", len(u) // 2) + u + b"\0\0"
    while len(data) % 4:
        data += b"\0"
    hdr = 28
    strings_start = hdr + 4 * len(strings)
    pool = struct.pack("<HHIIIIII", 0x0001, hdr, strings_start + len(data), len(strings), 0, 0,
                       strings_start, 0) + b"".join(struct.pack("<I", o) for o in offs) + data
    resmap = struct.pack("<HHI", 0x0180, 8, 8 + 4 * len(attr_names)) + b"".join(
        struct.pack("<I", ATTR_IDS[n]) for n in attr_names)

    NONE = 0xFFFFFFFF
    body = [struct.pack("<HHIIIII", 0x0100, 16, 24, 1, NONE, idx["android"], idx[ANDROID_NS])]
    line = [1]

    def emit(node):
        tag, attrs, kids = node
        line[0] += 1
        enc = []
        for name, v, ns in sorted(attrs, key=lambda t: ATTR_IDS.get(t[0], 1 << 32) if t[2] else 1 << 32):
            raw = NONE
            if isinstance(v, bool):
                typ, d = T_BOOL, 0xFFFFFFFF if v else 0
            elif isinstance(v, Ref):
                typ, d = T_REF, int(v)
            elif isinstance(v, Hex):
                typ, d = T_HEX, int(v)
            elif isinstance(v, int):
                typ, d = T_DEC, v & 0xFFFFFFFF
            else:
                typ, d, raw = T_STRING, idx[v], idx[v]
            enc.append(struct.pack("<IIIHBBI", idx[ANDROID_NS] if ns else NONE, idx[name], raw,
                                   8, 0, typ, d))
        body.append(struct.pack("<HHIIIIIHHHHHH", 0x0102, 16, 36 + 20 * len(enc), line[0], NONE,
                                NONE, idx[tag], 20, 20, len(enc), 0, 0, 0) + b"".join(enc))
        for k in kids:
            emit(k)
        line[0] += 1
        body.append(struct.pack("<HHIIIII", 0x0103, 16, 24, line[0], NONE, NONE, idx[tag]))

    emit(root)
    body.append(struct.pack("<HHIIIII", 0x0101, 16, 24, line[0] + 1, NONE, idx["android"], idx[ANDROID_NS]))
    payload = pool + resmap + b"".join(body)
    return struct.pack("<HHI", 0x0003, 8, 8 + len(payload)) + payload


def decode_axml(buf):
    """Tiny pretty-printer (for dump-manifest / self-checks)."""
    pos, out, depth = 8, [], 0
    strings, ids = [], []
    while pos < len(buf):
        typ, hsz, size = struct.unpack_from("<HHI", buf, pos)
        if typ == 0x0001:
            n, _, flags, sstart = struct.unpack_from("<IIII", buf, pos + 8)
            for i in range(n):
                o = pos + sstart + struct.unpack_from("<I", buf, pos + 28 + 4 * i)[0]
                ln = struct.unpack_from("<H", buf, o)[0]
                strings.append(buf[o + 2:o + 2 + 2 * ln].decode("utf-16-le"))
        elif typ == 0x0180:
            ids = list(struct.unpack_from("<%dI" % ((size - 8) // 4), buf, pos + 8))
        elif typ == 0x0102:
            _, name, _, _, cnt = struct.unpack_from("<IIHHH", buf, pos + 16)
            parts = []
            for i in range(cnt):
                ans, an, raw, _, _, t, d = struct.unpack_from("<IIIHBBI", buf, pos + 36 + 20 * i)
                rid = " (0x%08x)" % ids[an] if an < len(ids) else ""
                val = strings[d] if t == T_STRING else ("@0x%08x" % d if t == T_REF else (
                    str(d == 0xFFFFFFFF).lower() if t == T_BOOL else (hex(d) if t == T_HEX else str(d))))
                parts.append("%s%s%s=%s" % ("android:" if ans != 0xFFFFFFFF else "", strings[an], rid, val))
            out.append("  " * depth + "<" + strings[name] + " " + " ".join(parts) + ">")
            depth += 1
        elif typ == 0x0103:
            depth -= 1
        pos += size
    return "\n".join(out)


# ---------------------------------------------------------------- zip

def write_zip(entries):
    """entries: [(name, bytes, compress, align)] -> zip bytes (deterministic)."""
    bio = io.BytesIO()
    with zipfile.ZipFile(bio, "w") as zf:
        for name, data, compress, align in entries:
            zi = zipfile.ZipInfo(name, date_time=(1981, 1, 1, 1, 1, 2))
            zi.compress_type = zipfile.ZIP_DEFLATED if compress else zipfile.ZIP_STORED
            zi.create_system = 0
            zi.external_attr = 0
            if not compress:
                # Pad with an extra field like zipalign -p (id 0xd935: u16 alignment + zeros)
                # so the data starts on an `align` boundary.
                start = bio.tell() + 30 + len(name.encode())
                pad = (-(start + 6)) % align
                zi.extra = struct.pack("<HHH", 0xD935, 2 + pad, align) + b"\0" * pad
            if compress:
                zf.writestr(zi, data, compress_type=zipfile.ZIP_DEFLATED, compresslevel=9)
            else:
                zf.writestr(zi, data)
    return bio.getvalue()


def zip_sections(apk):
    eocd = apk.rfind(b"PK\x05\x06")
    if eocd < 0:
        raise SystemExit("not a zip")
    cd_size, cd_off = struct.unpack_from("<II", apk, eocd + 12)
    return cd_off, cd_size, eocd


# ---------------------------------------------------------------- APK Signature Scheme v2

V2_BLOCK_ID = 0x7109871A
SIG_RSA_PKCS1_SHA256 = 0x0103
SIG_ECDSA_SHA256 = 0x0201
MAGIC = b"APK Sig Block 42"


def lp(b):
    return struct.pack("<I", len(b)) + b


def chunked_digest(sections):
    chunks = []
    for s in sections:
        for i in range(0, len(s), 1 << 20):
            c = s[i:i + (1 << 20)]
            chunks.append(hashlib.sha256(b"\xa5" + struct.pack("<I", len(c)) + c).digest())
    return hashlib.sha256(b"\x5a" + struct.pack("<I", len(chunks)) + b"".join(chunks)).digest()


def openssl(*args, data=None):
    return subprocess.run(["openssl", *args], input=data, check=True, capture_output=True).stdout


def ensure_key(key_dir, algo):
    os.makedirs(key_dir, exist_ok=True)
    key, cert = os.path.join(key_dir, "key.pem"), os.path.join(key_dir, "cert.der")
    if not os.path.exists(key):
        if algo == "ec":
            openssl("genpkey", "-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-256", "-out", key)
        else:
            openssl("genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048", "-out", key)
        os.chmod(key, 0o600)
    if not os.path.exists(cert):
        openssl("req", "-x509", "-new", "-key", key, "-subj", "/CN=baba-port", "-days", "10000",
                "-sha256", "-outform", "DER", "-out", cert)
    return key, cert


def sign_v2(apk, key, cert):
    cd_off, cd_size, eocd = zip_sections(apk)
    s1, s3, s4 = apk[:cd_off], apk[cd_off:cd_off + cd_size], apk[eocd:]
    pub = openssl("pkey", "-in", key, "-pubout", "-outform", "DER")
    is_ec = b"\x2a\x86\x48\xce\x3d\x02\x01" in pub  # id-ecPublicKey OID
    alg = SIG_ECDSA_SHA256 if is_ec else SIG_RSA_PKCS1_SHA256
    digest = chunked_digest([s1, s3, s4])
    signed_data = (lp(lp(struct.pack("<I", alg) + lp(digest)))
                   + lp(lp(open(cert, "rb").read()))
                   + lp(b""))
    with tempfile.NamedTemporaryFile() as f:
        f.write(signed_data)
        f.flush()
        sig = openssl("dgst", "-sha256", "-sign", key, f.name)
    signer = lp(signed_data) + lp(lp(struct.pack("<I", alg) + lp(sig))) + lp(pub)
    value = lp(lp(signer))
    pair = struct.pack("<QI", 4 + len(value), V2_BLOCK_ID) + value
    size = len(pair) + 8 + 16  # pairs + trailing size + magic
    block = struct.pack("<Q", size) + pair + struct.pack("<Q", size) + MAGIC
    new_eocd = bytearray(s4)
    struct.pack_into("<I", new_eocd, 16, cd_off + len(block))
    return s1 + block + s3 + bytes(new_eocd)


def verify_v2(apk):
    cd_off, cd_size, eocd = zip_sections(apk)
    if apk[cd_off - 16:cd_off] != MAGIC:
        raise SystemExit("no APK signing block")
    size = struct.unpack_from("<Q", apk, cd_off - 24)[0]
    blk_start = cd_off - size - 8
    pos, v2 = blk_start + 8, None
    while pos < cd_off - 24:
        ln, pid = struct.unpack_from("<QI", apk, pos)
        if pid == V2_BLOCK_ID:
            v2 = apk[pos + 12:pos + 8 + ln]
        pos += 8 + ln
    if v2 is None:
        raise SystemExit("no v2 block")

    def rd(b, o):
        n = struct.unpack_from("<I", b, o)[0]
        return b[o + 4:o + 4 + n], o + 4 + n

    signers, _ = rd(v2, 0)
    signer, _ = rd(signers, 0)
    signed_data, o = rd(signer, 0)
    sigs, o = rd(signer, o)
    pub, _ = rd(signer, o)
    digests, _ = rd(signed_data, 0)
    d0, _ = rd(digests, 0)
    alg = struct.unpack_from("<I", d0)[0]
    want, _ = rd(d0, 4)
    eocd_b = bytearray(apk[eocd:])
    struct.pack_into("<I", eocd_b, 16, blk_start)
    got = chunked_digest([apk[:blk_start], apk[cd_off:cd_off + cd_size], bytes(eocd_b)])
    if got != want:
        raise SystemExit("digest mismatch")
    s0, _ = rd(sigs, 0)
    sig, _ = rd(s0, 4)
    with tempfile.TemporaryDirectory() as td:
        open(os.path.join(td, "pub.der"), "wb").write(pub)
        open(os.path.join(td, "sig"), "wb").write(sig)
        open(os.path.join(td, "data"), "wb").write(signed_data)
        r = subprocess.run(["openssl", "dgst", "-sha256", "-keyform", "DER", "-verify",
                            os.path.join(td, "pub.der"), "-signature", os.path.join(td, "sig"),
                            os.path.join(td, "data")], capture_output=True)
    if r.returncode != 0:
        raise SystemExit("signature does not verify")
    print("v2 signature OK (alg 0x%04x, block %d bytes)" % (alg, size + 8))


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    b = sub.add_parser("build")
    b.add_argument("--out", required=True)
    b.add_argument("--package", required=True)
    b.add_argument("--label", required=True)
    b.add_argument("--lib-name", required=True)
    b.add_argument("--so", action="append", default=[], help="apk/path=local/path")
    b.add_argument("--asset", action="append", default=[], help="name=local/path (stored)")
    b.add_argument("--key-dir", required=True)
    b.add_argument("--require-key", action="store_true",
                   help="fail instead of creating a key when --key-dir has none (release key)")
    b.add_argument("--file", action="append", default=[],
                   help="apk/path=local/path, deflated (licence texts)")
    b.add_argument("--key-algo", choices=["ec", "rsa"], default="ec")
    b.add_argument("--version-code", default="1")
    b.add_argument("--version-name", default="dev")
    b.add_argument("--min-sdk", default="26")
    b.add_argument("--target-sdk", default="35")
    b.add_argument("--orientation", default="sensorLandscape", choices=sorted(ORIENTATIONS))
    b.add_argument("--extract-native-libs", action="store_true",
                   help="compress .so in the APK (smaller download, extracted at install)")
    b.add_argument("--debuggable", action="store_true")
    b.add_argument("--permission", action="append", default=[],
                   help="uses-permission, e.g. android.permission.INTERNET")
    b.add_argument("--open-mime", action="append", default=[],
                   help="MIME type the app offers to open (VIEW) and receive (SEND), e.g. application/zip")
    v = sub.add_parser("verify")
    v.add_argument("apk")
    d = sub.add_parser("dump-manifest")
    d.add_argument("apk")
    a = ap.parse_args()

    if a.cmd == "verify":
        verify_v2(open(a.apk, "rb").read())
        return
    if a.cmd == "dump-manifest":
        with zipfile.ZipFile(a.apk) as z:
            print(decode_axml(z.read("AndroidManifest.xml")))
        return

    entries = [("AndroidManifest.xml", encode_axml(manifest_tree(a)), True, 4)]
    for spec in a.so:
        name, path = spec.split("=", 1)
        entries.append((name, open(path, "rb").read(), a.extract_native_libs, 16384))
    for spec in a.asset:
        name, path = spec.split("=", 1)
        entries.append(("assets/" + name, open(path, "rb").read(), False, 4))
    for spec in a.file:
        name, path = spec.split("=", 1)
        entries.append((name, open(path, "rb").read(), True, 4))
    unsigned = write_zip(entries)
    if a.require_key and not all(os.path.isfile(os.path.join(a.key_dir, f))
                                 for f in ("key.pem", "cert.der")):
        raise SystemExit(f"{a.key_dir}: no key.pem + cert.der (release key missing)")
    key, cert = ensure_key(a.key_dir, a.key_algo)
    signed = sign_v2(unsigned, key, cert)
    tmp = a.out + ".part"
    with open(tmp, "wb") as f:
        f.write(signed)
    os.replace(tmp, a.out)
    print("%s: %d bytes" % (a.out, len(signed)))


if __name__ == "__main__":
    sys.exit(main())
