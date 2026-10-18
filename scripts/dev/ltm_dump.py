"""Print the members and the start of the input log of a libTAS .ltm movie (untrusted data).

    python -I scripts/dev/ltm_dump.py MOVIE.ltm [N]
Reads the tar.gz in memory; nothing is extracted to disk.
"""
import gzip
import io
import sys
import tarfile
import zipfile


def open_ltm(path):
    data = open(path, "rb").read()
    if data[:2] == b"PK":  # TASVideos wraps publication downloads in a zip
        z = zipfile.ZipFile(io.BytesIO(data))
        data = z.read(z.namelist()[0])
    while data[:2] == b"\x1f\x8b":  # .ltm is a tar.gz; downloads may be gzipped once more
        data = gzip.decompress(data)
    t = tarfile.open(fileobj=io.BytesIO(data), mode="r:")
    return {m.name: t.extractfile(m).read() for m in t.getmembers() if m.isfile()}


if __name__ == "__main__":
    files = open_ltm(sys.argv[1])
    n = int(sys.argv[2]) if len(sys.argv) > 2 else 40
    for name, b in files.items():
        print("==", name, len(b))
    print(files.get("config.ini", b"").decode(errors="replace")[:1500])
    lines = files["inputs"].decode(errors="replace").splitlines()
    print("frames:", len(lines))
    for l in lines[:n]:
        print(l)
