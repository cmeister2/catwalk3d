#!/usr/bin/env python3
"""Package completed, explicitly identified builds; never infer their source provenance."""

import argparse
import hashlib
import json
from pathlib import Path
import shutil


BASE = Path(__file__).resolve().parent
REPO = BASE.parents[4]


def sha(path):
    """Hash artifact bytes, including large binaries without loading them all at once."""
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def main():
    """Create a new bundle only after the actual build metadata is supplied."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--catwalk-binary", type=Path, required=True)
    parser.add_argument("--profile-binary", type=Path, required=True)
    parser.add_argument("--build-metadata", type=Path, required=True)
    parser.add_argument("--synthetic", type=Path, default=Path("/tmp/catwalk-synthetic-repeated.step"))
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    metadata = json.loads(args.build_metadata.read_text())
    required = ["source_provenance", "cargo_lock_sha256", "compiler", "command",
                "catwalk_binary_sha256", "profile_gpu_binary_sha256", "profile_gpu_source_sha256"]
    for key in required:
        if not metadata.get(key):
            raise ValueError(f"completed build metadata must provide {key}")
    assert sha(args.catwalk_binary) == metadata["catwalk_binary_sha256"], "catwalk hash differs from supplied completed build"
    assert sha(args.profile_binary) == metadata["profile_gpu_binary_sha256"], "profile_gpu hash differs from supplied completed build"
    synthetic_manifest = args.synthetic.with_name(args.synthetic.name + ".json")
    synthetic = json.loads(synthetic_manifest.read_text())
    assert synthetic["step_sha256"] == sha(args.synthetic)
    assert (synthetic["source_mesh_count"], synthetic["source_triangle_count"], synthetic["instance_count"], synthetic["logical_triangle_count"]) == (1, 12288, 64, 786432)
    sources = {
        "gear.obj": REPO / "examples/models/gear.obj",
        "ap242-tessellated.stp": REPO / "tests/fixtures/formats/scientific_cad/step-nist-ap242-tessellated.stp",
        "synthetic-repeated.step": args.synthetic,
        "jpeg-textured.3mf": REPO / "tests/fixtures/formats/mesh_scene/3mf-consortium-jpeg-textures.3mf",
        "alpha-modes.glb": REPO / "tests/fixtures/formats/mesh_scene/glb-khronos-alpha-modes.glb",
    }
    assert all(path.is_file() for path in sources.values())
    args.output.mkdir(parents=True, exist_ok=False)
    for directory in ["bin", "inputs", "scripts"]:
        (args.output / directory).mkdir()
    shutil.copy2(args.catwalk_binary, args.output / "bin/catwalk")
    shutil.copy2(args.profile_binary, args.output / "bin/profile_gpu")
    shutil.copy2(BASE / "run-on-nas.sh", args.output / "run-on-nas.sh")
    for name in ["run-profile-case.sh", "run-cli-case.sh", "capture-memory.sh"]:
        shutil.copy2(BASE / name, args.output / "scripts" / name)
    shutil.copy2(BASE / "policy.json", args.output / "policy.json")
    shutil.copy2(synthetic_manifest, args.output / "inputs/synthetic-repeated.step.json")
    manifest = json.loads((BASE / "manifest-template.json").read_text())
    manifest["status"] = "finalized"
    manifest["build"] = metadata
    manifest["inputs"] = []
    for name, path in sources.items():
        shutil.copy2(path, args.output / "inputs" / name)
        manifest["inputs"].append({"name": name, "source_path": str(path), "sha256": sha(path), "bytes": path.stat().st_size})
    manifest["synthetic_workload_manifest_sha256"] = sha(synthetic_manifest)
    manifest["policy_sha256"] = sha(BASE / "policy.json")
    manifest["scripts_sha256"] = {
        name: sha(BASE / name)
        for name in ["run-on-nas.sh", "run-profile-case.sh", "run-cli-case.sh", "capture-memory.sh",
                     "prepare_bundle.py", "compare_png.py", "compare_results.py", "summary.py"]
    }
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    checksums = []
    for path in sorted(args.output.rglob("*")):
        if path.is_file():
            checksums.append(f"{sha(path)}  {path.relative_to(args.output)}\n")
    (args.output / "SHA256SUMS").write_text("".join(checksums))
    print(args.output)


if __name__ == "__main__":
    main()
