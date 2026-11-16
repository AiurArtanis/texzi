#!/usr/bin/env python3
"""Create the complete ZIP and two stand-alone HTML deliverables, offline."""
from pathlib import Path
import argparse
import hashlib
import json
import shutil
import zipfile

ROOT = Path(__file__).resolve().parents[1]

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT.parent / "texzi-deliverables")
    args = parser.parse_args()
    output = args.output.resolve()
    if output == ROOT or ROOT in output.parents:
        parser.error("Output must be outside the source package directory")
    output.mkdir(parents=True, exist_ok=True)
    files = sorted(p for p in ROOT.rglob("*") if p.is_file()
                   and "__pycache__" not in p.parts and p.suffix not in {".pyc", ".zip"}
                   and p.name != "manifest-sha256.json")
    manifest = {
        "schemaVersion": 1, "product": "texzi", "designVersion": "1.0",
        "date": "2026-11-15", "hash": "SHA-256",
        "note": "This manifest excludes itself. Checksums verify package content, not native product performance.",
        "files": [{"path": p.relative_to(ROOT).as_posix(), "bytes": p.stat().st_size,
                   "sha256": hashlib.sha256(p.read_bytes()).hexdigest()} for p in files]
    }
    manifest_file = ROOT / "manifest-sha256.json"
    manifest_file.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    files.append(manifest_file)
    archive = output / "texzi-design-v1.0.zip"
    with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        for file in sorted(files):
            info = zipfile.ZipInfo("texzi-design-v1/" + file.relative_to(ROOT).as_posix(),
                                  date_time=(2026, 11, 15, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o644 << 16
            z.writestr(info, file.read_bytes())
    handbook = output / "texzi-design-handbook-v1.0.html"
    prototype = output / "texzi-prototype-v1.0.html"
    shutil.copyfile(ROOT / "texzi-design-handbook.html", handbook)
    shutil.copyfile(ROOT / "prototype/index.html", prototype)
    with zipfile.ZipFile(archive) as z:
        assert z.testzip() is None, "ZIP CRC validation failed"
        assert len(z.infolist()) == len(files), "ZIP file count mismatch"
    result = [{"file": str(file), "bytes": file.stat().st_size,
               "sha256": hashlib.sha256(file.read_bytes()).hexdigest()}
              for file in (archive, handbook, prototype)]
    print(json.dumps({"filesInPackage": len(files), "deliverables": result},
                     ensure_ascii=False, indent=2))

if __name__ == "__main__":
    main()
