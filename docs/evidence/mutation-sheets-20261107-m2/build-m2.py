import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tarfile
import time
import traceback

checkpoint = sys.argv[1]
assert re.fullmatch(r"[0-9a-f]{40}", checkpoint)
short = checkpoint[:7]
root = Path.home() / (".cache/files-m2-updates/sheets-" + short)
source = root / "source"
archive = root / ("source-" + short + ".tar")
manifest_path = root / ("source-manifest-" + short + ".json")
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
blob = lambda data: hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()
record = {"source_checkpoint": checkpoint, "stage": "preflight", "resource_gate": "NOT_MEASURED", "startup_gate": "NOT_MEASURED"}
try:
    manifest = json.loads(manifest_path.read_text())
    assert manifest["source_checkpoint"] == checkpoint
    assert sha(archive) == manifest["archive_sha256"]
    assert sha(root / "node-vendor-manifest.json") == manifest["node_vendor_manifest_sha256"]
    expected = {item["path"]: item for item in manifest["files"]}
    assert len(expected) == manifest["source_file_count"] == len(expected)
    assert not source.exists(), "Preserve prior build evidence; never overwrite an existing source tree"
    old = Path.home() / ".cache/files-m2-updates/sheets-f1ce00c/source/target"
    assert "ARM aarch64" in subprocess.check_output(["file", str(old / "release/files")], text=True)
    old_size = int(subprocess.check_output(["du", "-sb", str(old)], text=True).split()[0])
    free = shutil.disk_usage(root).free
    record.update({"source_archive_sha256": sha(archive), "source_archive_bytes": archive.stat().st_size,
                   "source_manifest_sha256": sha(manifest_path), "native_cache_source": str(old),
                   "native_cache_apparent_bytes": old_size, "disk_free_before_bytes": free})
    assert free > old_size + 512 * 1024 * 1024, "Insufficient space for a conservative native cache copy"
    with tarfile.open(archive) as package:
        members = package.getmembers()
        entries = [item for item in members if not item.isdir()]
        assert {item.name for item in entries} == set(expected) and len(entries) == len(expected)
        for item in members:
            path = Path(item.name)
            assert not path.is_absolute() and ".." not in path.parts
            assert item.isfile() or item.isdir(), item.name
            if item.isfile():
                data = package.extractfile(item).read()
                entry = expected[item.name]
                assert len(data) == entry["bytes"] and hashlib.sha256(data).hexdigest() == entry["sha256"]
                assert blob(data) == entry["git_blob"] and not data.startswith(b"\x7fELF")
                assert stat.S_IMODE(item.mode) == int(entry["git_mode"], 8) & 0o777
        source.mkdir(mode=0o700)
        package.extractall(source, filter="data")

    def verify_source():
        for name, entry in expected.items():
            path = source / name
            assert path.is_file() and not path.is_symlink(), name
            data = path.read_bytes()
            assert len(data) == entry["bytes"] and hashlib.sha256(data).hexdigest() == entry["sha256"], name
            assert blob(data) == entry["git_blob"], name
            assert stat.S_IMODE(path.stat().st_mode) == int(entry["git_mode"], 8) & 0o777, name
        return len(expected)

    actual = {str(path.relative_to(source)) for path in source.rglob("*") if path.is_file() or path.is_symlink()}
    assert actual == set(expected)
    record["source_files_verified"] = verify_source()
    record["source_path_set_verified_before_build"] = True
    record["stage"] = "native-cache-copy"
    subprocess.run(["cp", "-a", "--reflink=auto", str(old), str(source / "target")], check=True)
    stale = root / "stale-cmake"
    preserved = []
    for pattern in ["*/build/fltk-*", "*/build/fltk-sys-*", "*/.fingerprint/fltk-*", "*/.fingerprint/fltk-sys-*"]:
        for path in sorted((source / "target").glob(pattern)):
            relative = path.relative_to(source / "target")
            destination = stale / relative
            assert not destination.exists()
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.move(str(path), str(destination))
            preserved.append(str(relative))
    record["preserved_stale_cmake"] = preserved
    private = root / "private-build"
    private.mkdir(mode=0o700)
    env = dict(os.environ)
    env["PYTHONDONTWRITEBYTECODE"] = "1"
    for key, name in [("TMPDIR", "tmp"), ("FILES_CONFIG_DIR", "config"), ("XDG_DATA_HOME", "data")]:
        directory = private / name
        directory.mkdir(mode=0o700)
        env[key] = str(directory)
    env["FILES_SOURCE_CHECKPOINT"] = checkpoint
    env["CARGO_TARGET_DIR"] = str(source / "target")
    env.pop("CARGO_CHECK", None)
    record["stage"] = "native-release-build"
    started = time.monotonic()
    print(json.dumps({"stage": record["stage"], "checkpoint": checkpoint, "verified_source_files": len(expected),
                      "preserved_cache_entries": len(preserved), "log": str(root / "build.log")}), flush=True)
    with (root / "build.log").open("x") as log:
        result = subprocess.run(["cargo", "build", "--release", "--locked", "--offline"], cwd=source,
                                env=env, stdout=log, stderr=subprocess.STDOUT)
    record["build_returncode"] = result.returncode
    record["elapsed_seconds"] = time.monotonic() - started
    record["source_files_after_build_verified"] = verify_source()
    assert sha(archive) == manifest["archive_sha256"]
    assert sha(manifest_path) == record["source_manifest_sha256"]
    record["source_archive_after_build_verified"] = True
    if result.returncode == 0:
        release = source / "target/release"
        for name in ["files", "files-helper", "files-html-helper"]:
            path = release / name
            identity = subprocess.check_output(["file", str(path)], text=True).strip()
            assert "ARM aarch64" in identity, identity
            record[name + "_sha256"] = sha(path)
            record[name + "_file"] = identity
        for name in ["files-html-helper.LICENSES", "files-html-helper.build.json"]:
            record[name + "_sha256"] = sha(release / name)
        stamp = json.loads((release / "files-html-helper.build.json").read_text())
        spec = importlib.util.spec_from_file_location("files_html_recipe", source / "scripts/html-engine-probe.py")
        recipe = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(recipe)
        assert recipe.adapter_fingerprint() == stamp["adapter_sha256"]
        assert stamp["binary_sha256"] == record["files-html-helper_sha256"]
        record["html_stamp"] = stamp
        record["reused_native_html_recipe_verified"] = True

        record["stage"] = "built-not-installed-or-native-accepted"
    else:
        record["stage"] = "build-failed-evidence-preserved"
    (root / "build-result.json").write_text(json.dumps(record, indent=2) + "\n")
    print(json.dumps(record), flush=True)
    raise SystemExit(result.returncode)
except Exception as error:
    record["stage_at_failure"] = record["stage"]
    record["stage"] = "failed-evidence-preserved"
    record["error"] = type(error).__name__ + ": " + str(error)
    (root / "failure.log").write_text(traceback.format_exc())
    (root / "build-result.json").write_text(json.dumps(record, indent=2) + "\n")
    print(json.dumps(record), flush=True)
    raise
