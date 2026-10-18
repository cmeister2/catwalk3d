#!/usr/bin/env python3
"""Run completed native Windows GPU builds from WSL using UNC paths, never WSL graphics."""

import argparse
import base64
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import time


BASE = Path(__file__).resolve().parent
REPO = BASE.parents[5]
POLICY_SHA256 = "88f12e77f2ac5b2d30ad7a4383f309aa47ecd6d67ad388eb2291f3a70f79f38f"
BACKENDS = ["dx12", "vulkan"]
ADAPTER = "Radeon(TM) 780M"
NATIVE_WRAPPER = r"""
$ErrorActionPreference = 'Stop'
$payload = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('__PAYLOAD__')) | ConvertFrom-Json
$timer = [Diagnostics.Stopwatch]::StartNew()
try {
  $process = Start-Process -FilePath $payload.exe -ArgumentList $payload.commandline `
    -WorkingDirectory ([Environment]::GetFolderPath('UserProfile')) -PassThru -NoNewWindow `
    -RedirectStandardOutput $payload.stdout -RedirectStandardError $payload.stderr
  $null = $process.Handle
  [IO.File]::WriteAllText($payload.pidfile, [string]$process.Id)
  $done = $process.WaitForExit([int]$payload.timeout_ms)
  if (-not $done) {
    & taskkill.exe /PID $process.Id /T /F | Out-Null
    $null = $process.WaitForExit(5000)
    $code = 124
  } else {
    $process.WaitForExit()
    $code = $process.ExitCode
    if ($null -eq $code) { throw 'Native process exited but its exit code is unavailable' }
  }
  @{ returncode = $code; timed_out = (-not $done); native_pid = $process.Id; native_wall_ms = $timer.Elapsed.TotalMilliseconds } | ConvertTo-Json -Compress
} catch {
  @{ returncode = 125; launcher_error = [string]$_; native_wall_ms = $timer.Elapsed.TotalMilliseconds } | ConvertTo-Json -Compress
}
"""


def sha(path):
    """Hash completed artifacts without guessing their source provenance."""
    result = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            result.update(chunk)
    return result.hexdigest()


def unc(path):
    """Require WSL UNC paths for every input/output and native GPU executable."""
    result = subprocess.run(["wslpath", "-w", str(path.resolve())], capture_output=True, text=True, check=True).stdout.strip()
    if not result.startswith("\\\\"):
        raise ValueError(f"place benchmark artifacts in the WSL filesystem to obtain UNC paths: {result}")
    return result


def encoded_powershell(script):
    """Use UTF-16LE encoded commands rather than shell interpolation of paths."""
    return ["powershell.exe", "-NoLogo", "-NoProfile", "-NonInteractive", "-EncodedCommand",
            base64.b64encode(script.encode("utf-16le")).decode("ascii")]


def invoke(binary, arguments, stem, timeout=120):
    """Capture native logs, command/return code and separate native/interop wall scopes."""
    stem.parent.mkdir(parents=True, exist_ok=True)
    payload = {"exe": unc(binary), "commandline": subprocess.list2cmdline(arguments),
               "stdout": unc(stem.with_suffix(".stdout")), "stderr": unc(stem.with_suffix(".stderr")),
               "pidfile": unc(stem.with_suffix(".pid")), "timeout_ms": timeout * 1000}
    body = base64.b64encode(json.dumps(payload).encode()).decode("ascii")
    command = encoded_powershell(NATIVE_WRAPPER.replace("__PAYLOAD__", body))
    start = time.perf_counter()
    try:
        launched = subprocess.run(command, capture_output=True, text=True, timeout=timeout + 45)
        outer = {"returncode": launched.returncode, "stdout": launched.stdout, "stderr": launched.stderr}
        result = json.loads(launched.stdout.strip()) if launched.returncode == 0 else {"returncode": 125}
    except (subprocess.TimeoutExpired, json.JSONDecodeError) as error:
        stem.with_suffix(".launcher-error.txt").write_text(str(error))
        raise RuntimeError(f"launcher state is uncertain; stop suite and inspect native PID in {stem.with_suffix('.pid')}") from error
    record = {"executable_unc": payload["exe"], "arguments": arguments,
              "native_commandline": payload["commandline"], "outer_interop_wall_ms": (time.perf_counter() - start) * 1000,
              "launcher": outer, "native": result, "process_ram_cpu": "not measured; profiler procfs fields are null on Windows"}
    stem.with_suffix(".command.json").write_text(json.dumps(record, indent=2) + "\n")
    return result.get("returncode", 125)


def main():
    """Require owner-supplied completed build metadata before any Windows execution."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--catwalk-binary", type=Path, required=True)
    parser.add_argument("--profile-binary", type=Path, required=True)
    parser.add_argument("--build-metadata", type=Path, required=True)
    parser.add_argument("--synthetic", type=Path, default=Path("/tmp/catwalk-synthetic-repeated.step"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--prepare-only", action="store_true", help="stage supplied builds/inputs without any Windows execution")
    parser.add_argument("--stage", choices=["all", "profile", "cli"], default="all")
    args = parser.parse_args()
    metadata = json.loads(args.build_metadata.read_text())
    for key in ["source_provenance", "compiler", "command", "cargo_lock_sha256", "profile_gpu_source_sha256",
                "catwalk_binary_sha256", "profile_gpu_binary_sha256"]:
        if not metadata.get(key):
            raise ValueError(f"completed build metadata must supply {key}")
    for binary, key in [(args.catwalk_binary, "catwalk_binary_sha256"), (args.profile_binary, "profile_gpu_binary_sha256")]:
        if sha(binary) != metadata[key] or binary.open("rb").read(2) != b"MZ":
            raise ValueError(f"not the supplied completed Windows executable: {binary}")
    if sha(BASE.parent / "policy.json") != POLICY_SHA256:
        raise ValueError("frozen PNG policy changed")
    sources = {
        "gear.obj": REPO / "examples/models/gear.obj",
        "ap242-tessellated.stp": REPO / "tests/fixtures/formats/scientific_cad/step-nist-ap242-tessellated.stp",
        "synthetic-repeated.step": args.synthetic,
        "jpeg-textured.3mf": REPO / "tests/fixtures/formats/mesh_scene/3mf-consortium-jpeg-textures.3mf",
        "alpha-modes.glb": REPO / "tests/fixtures/formats/mesh_scene/glb-khronos-alpha-modes.glb",
    }
    synthetic_manifest = args.synthetic.with_name(args.synthetic.name + ".json")
    synthetic = json.loads(synthetic_manifest.read_text())
    if synthetic["step_sha256"] != sha(args.synthetic) or synthetic["logical_triangle_count"] != 786432:
        raise ValueError("synthetic workload manifest differs")
    args.output.mkdir(parents=True, exist_ok=False)
    for directory in ["bin", "inputs", "results"]:
        (args.output / directory).mkdir()
    binaries = {"catwalk": args.output / "bin/catwalk.exe", "profile": args.output / "bin/profile_gpu.exe"}
    shutil.copy2(args.catwalk_binary, binaries["catwalk"])
    shutil.copy2(args.profile_binary, binaries["profile"])
    for binary in binaries.values():
        binary.chmod((binary.stat().st_mode & 0o777) | 0o100)
    manifest = {"schema_version": 1, "platform": "native Windows via WSL interop; no WSL rendering",
                "created_utc": datetime.now(timezone.utc).isoformat(), "build": metadata,
                "backends": BACKENDS, "adapter_substring": ADAPTER, "size": 512, "samples": 1,
                "iterations": 3, "jobs_expected": 162, "stage": args.stage, "inputs": [],
                "process_ram_cpu": "unavailable in current profiler on Windows; null is not zero",
                "restrictions": "Windows host, no NAS cgroups or CPU3 pinning; GPU explicit allocation budget256MiB is not a total RAM cap",
                "policy_sha256": POLICY_SHA256, "runner_sha256": sha(Path(__file__)),
                "synthetic_manifest_sha256": sha(synthetic_manifest)}
    for name, path in sources.items():
        destination = args.output / "inputs" / name
        shutil.copy2(path, destination)
        manifest["inputs"].append({"name": name, "sha256": sha(destination), "bytes": destination.stat().st_size})
    shutil.copy2(synthetic_manifest, args.output / "inputs/synthetic-repeated.step.json")
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    if args.prepare_only:
        print(f"Prepared only; no Windows execution: {args.output}")
        return 0
    inventory = r"Get-CimInstance Win32_VideoController | Select-Object Name,PNPDeviceID,DriverVersion,VideoProcessor,AdapterRAM | ConvertTo-Json -Depth 4"
    inventory_result = subprocess.run(encoded_powershell(inventory), capture_output=True, text=True, timeout=45)
    (args.output / "results/windows-video-inventory.json").write_text(json.dumps({"returncode": inventory_result.returncode,
        "stdout": inventory_result.stdout, "stderr": inventory_result.stderr, "scope": "driver inventory only, not allocation/peak-memory evidence"}, indent=2) + "\n")
    failed = False
    for backend in BACKENDS:
        results = args.output / "results" / backend
        if args.stage != "cli":
            for name in list(sources)[:3]:
                for mode in ["cpu", "shared-cpu", "gpu"]:
                    directory = results / "profile" / Path(name).stem / mode
                    directory.mkdir(parents=True, exist_ok=True)
                    for experiment in ["fresh", "retained", "reload"]:
                        arguments = [unc(args.output / "inputs" / name), "--mode", mode, "--iterations", "3", "--size", "512", "--samples", "1", "--backend", backend, "--adapter", ADAPTER, "--gpu-memory-mib", "256", "--report", unc(directory / f"{experiment}.json")]
                        arguments += {"fresh": ["--fresh-process"], "retained": [], "reload": ["--reload-scene"]}[experiment]
                        code = invoke(binaries["profile"], arguments, directory / experiment)
                        failed |= code != 0
                        print(f"profile {backend} {Path(name).stem} {mode} {experiment}: exit {code}", flush=True)
        if args.stage != "profile":
            for name in sources:
                support = "supported" if name in list(sources)[:3] else "unsupported"
                directory = results / "cli" / Path(name).stem / support
                directory.mkdir(parents=True, exist_ok=True)
                statuses = ["renderer\tsamples\tbackground\texit_status\texpectation_met\n"]
                for samples in [1, 2]:
                    for background in ["opaque", "transparent"]:
                        for renderer in (["cpu", "gpu"] if support == "supported" else ["cpu", "auto", "gpu"]):
                            stem = directory / f"{renderer}-s{samples}-{background}"
                            arguments = [unc(args.output / "inputs" / name), "--output", unc(stem.with_suffix(".png")), "--render-geometry", "--renderer", renderer, "--size", "512", "--samples", str(samples), "--gpu-backend", backend, "--gpu-adapter", ADAPTER, "--gpu-memory-mib", "256"]
                            if background == "transparent":
                                arguments.append("--transparent")
                            code = invoke(binaries["catwalk"], arguments, stem)
                            rejected = support == "unsupported" and renderer == "gpu"
                            stderr = stem.with_suffix(".stderr").read_text(errors="replace") if stem.with_suffix(".stderr").exists() else ""
                            met = (code == 1 and not stem.with_suffix(".png").exists() and "supports opaque untextured materials" in stderr) if rejected else code == 0 and stem.with_suffix(".png").exists()
                            failed |= not met
                            statuses.append(f"{renderer}\t{samples}\t{background}\t{code}\t{int(met)}\n")
                (directory / "status.tsv").write_text("".join(statuses))
                print(f"cli {backend} {Path(name).stem}: command matrix complete", flush=True)
    (args.output / "execution-complete.json").write_text(json.dumps({"completed_utc": datetime.now(timezone.utc).isoformat(), "command_expectations_passed": not failed}) + "\n")
    print(args.output)
    return int(failed)


if __name__ == "__main__":
    raise SystemExit(main())
