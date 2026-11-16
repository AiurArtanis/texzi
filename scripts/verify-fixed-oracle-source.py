"""Verify an exact-byte git archive plus one reviewed injected test, before Cargo."""
import argparse
import hashlib
import json
import pathlib
import subprocess

PRODUCT = "5a14645d38dbab6e6612cea5e0a585c740fb6cd7"
HARNESS = "60d4a5f5bba71f359488a1f9d27f9a0eca25e4db7a0f79f49c17a6d12ecd53f0"
INJECTED = "crates/texzi-document/tests/steamy_oracle.rs"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", required=True)
    parser.add_argument("--source", required=True)
    parser.add_argument("--out", required=True)
    args = parser.parse_args()
    root = pathlib.Path(args.source)
    output = pathlib.Path(args.out)
    if output.exists():
        raise RuntimeError("preserve existing manifest")
    entries = subprocess.check_output(["git", "-C", args.repo, "ls-tree", "-rz", PRODUCT]).split(b"\0")
    batch = subprocess.Popen(["git", "-C", args.repo, "cat-file", "--batch"], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    files = []
    try:
        for entry in filter(None, entries):
            info, name = entry.split(b"\t", 1)
            mode, kind, oid = info.split()
            assert mode == b"100644" and kind == b"blob", entry
            path = name.decode("utf-8")
            batch.stdin.write(oid + b"\n")
            batch.stdin.flush()
            returned, obj_kind, size = batch.stdout.readline().split()
            assert returned == oid and obj_kind == b"blob"
            blob = batch.stdout.read(int(size))
            assert len(blob) == int(size) and batch.stdout.read(1) == b"\n"
            actual = (root / path).read_bytes()
            assert actual == blob, "exact git blob mismatch: " + path
            files.append(dict(path=path, git_blob_oid=oid.decode(), bytes=len(actual),
                              sha256=hashlib.sha256(actual).hexdigest(), exact_blob_bytes=True))
    finally:
        batch.stdin.close()
        batch.wait(timeout=10)
    assert batch.returncode == 0
    test = root / INJECTED
    actual = test.read_bytes()
    assert hashlib.sha256(actual).hexdigest() == HARNESS, "reviewed harness bytes changed"
    expected = {f["path"] for f in files} | {INJECTED}
    found = {p.relative_to(root).as_posix() for p in root.rglob("*") if p.is_file()}
    assert found == expected, dict(unexpected=sorted(found - expected), missing=sorted(expected - found))
    payload = dict(product_commit=PRODUCT, transformation="none; archive bytes == git blob bytes",
                   files=files, injected_test=dict(path=INJECTED, bytes=len(actual), sha256=HARNESS),
                   unexpected_files=[])
    output.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(dict(product_commit=PRODUCT, product_files=len(files), exact_blob_files=len(files),
                          harness_sha256=HARNESS, unexpected_files=0)))


if __name__ == "__main__":
    main()
