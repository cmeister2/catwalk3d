#!/usr/bin/env python3
"""Validate native AMD Windows renderer reports; preserve unavailable process CPU/RAM."""

import argparse
import importlib.util
import json
import os
from pathlib import Path, PureWindowsPath
import subprocess
import sys


sys.dont_write_bytecode = True
BASE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("renderer_summary_helpers", BASE.parent / "summary.py")
common = importlib.util.module_from_spec(spec)
spec.loader.exec_module(common)
require, phase, series, sha = common.require, common.phase, common.series, common.sha
EXPECTED = {"gear.obj": 512, "ap242-tessellated.stp": 3388, "synthetic-repeated.step": 786432}
BACKENDS = {"dx12": "Dx12", "vulkan": "Vulkan"}


def process(report, mode, name, backend, reload_scene, count):
    """Validate native identity/capabilities and the current profiler setup/job schema."""
    expected_operation = "same_process_reuse_device_reload_scene" if reload_scene else "same_process_reuse_scene_and_device"
    require(report.get("operation") == expected_operation and report.get("mode") == mode
            and report.get("reload_scene") is reload_scene, "operation/mode/reload differs")
    require(report["input"].startswith("\\\\") and PureWindowsPath(report["input"]).name == name, "input is not expected Windows UNC path")
    require(report.get("width") == report.get("height") == 512 and report.get("supersampling") == 1, "dimensions/sampling differ")
    require(report.get("background_srgb") == [245, 247, 250] and report.get("fallback_color_srgb") == [100, 160, 220], "colors differ")
    require(report.get("cpu_clock_ticks_per_second") is None, "unexpected native Windows procfs CPU measurement")
    if mode == "gpu":
        adapter = report["adapter"]
        require(adapter.get("vendor") == 0x1002 and adapter.get("device") == 0x15BF
                and adapter.get("device_type") == "IntegratedGpu" and adapter.get("backend") == BACKENDS[backend]
                and "radeon(tm) 780m" in adapter.get("name", "").lower(), "wrong/unverified native AMD hardware/backend")
        capabilities = adapter.get("capabilities")
        require(isinstance(capabilities, dict) and capabilities.get("wgpu_version") == "29.0.3", "actual renderer capabilities missing/version differs")
        for limits in ["adapter_limits", "device_limits"]:
            require(capabilities[limits].get("all_limits_debug") and capabilities[limits].get("max_vertex_attributes", 0) >= 12
                    and capabilities[limits].get("max_vertex_buffers", 0) >= 2
                    and capabilities[limits].get("max_texture_dimension_2d", 0) >= 512,
                    f"required actual renderer limits missing: {limits}")
        require(all(key in capabilities for key in ["adapter_features_debug", "device_features_debug", "downlevel"]), "capability details absent")
        require(all(key in capabilities["downlevel"] for key in ["flags_bits", "shader_model", "compute_shaders_supported"]), "downlevel capability details absent")
        phase(report.get("setup_device"), "setup_device")
    else:
        require(report.get("adapter") is None and report.get("setup_device") is None, "CPU path unexpectedly created GPU device")
    if reload_scene:
        require(report.get("setup") is None, "reload path cached source scene")
    else:
        common.validate_setup(report["setup"], mode, EXPECTED[name], name == "synthetic-repeated.step")
    require(len(report["jobs"]) == count, "job count differs")
    for index, job in enumerate(report["jobs"]):
        require(job.get("iteration") == index and job.get("first_render") is (index == 0)
                and job.get("first_render_of_scene") is (reload_scene or index == 0), "iteration/reuse labels differ")
        if reload_scene:
            common.validate_setup(job["scene_setup"], mode, EXPECTED[name], name == "synthetic-repeated.step")
            phase(job["scene_cleanup"], "scene_cleanup")
        else:
            require(job.get("scene_setup") is None and job.get("scene_cleanup") is None, "cached job reloaded")
        for key in ["render", "png_encode_to_memory", "job_with_cleanup_and_accounting"]:
            observed = phase(job[key], key)
            require(observed["user_cpu_ms"] is None and observed["system_cpu_ms"] is None, "Windows CPU field must be explicitly unavailable")
            require(all(observed[boundary]["rss_kib"] is None and observed[boundary]["process_lifetime_hwm_kib"] is None
                        for boundary in ["before", "after"]), "Windows process RAM field must be explicitly unavailable")
        require(job.get("png_write") is None, "profiling includes unexpected PNG-file write")
        for key in ["cpu_geometry_preparation_wall_ms", "cpu_rasterization_and_resolve_wall_ms"]:
            require(key in job["render"], "old render schema")
            require(job["render"][key] is None if mode == "gpu" else isinstance(job["render"][key], (int, float)), "fit/raster substage differs")
    phase(report["setup_jobs_and_cleanup"], "process scope")
    return report


def main():
    """Summarize all54 experiment reports and run the unchanged PNG comparison matrix."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bundle", type=Path)
    args = parser.parse_args()
    manifest = json.loads((args.bundle / "manifest.json").read_text())
    require(manifest["policy_sha256"] == sha(BASE.parent / "policy.json") == common.FROZEN_POLICY, "frozen image policy differs")
    for name, key in [("catwalk.exe", "catwalk_binary_sha256"), ("profile_gpu.exe", "profile_gpu_binary_sha256")]:
        require(sha(args.bundle / "bin" / name) == manifest["build"][key], "completed executable hash differs")
    for item in manifest["inputs"]:
        require(sha(args.bundle / "inputs" / item["name"]) == item["sha256"], "input hash differs")
    rows = []
    for backend in BACKENDS:
        for name in EXPECTED:
            for mode in ["cpu", "shared-cpu", "gpu"]:
                for experiment in ["fresh", "retained", "reload"]:
                    directory = args.bundle / "results" / backend / "profile" / Path(name).stem / mode
                    report_path = directory / f"{experiment}.json"
                    command_path = directory / f"{experiment}.command.json"
                    row = {"backend": backend, "input": name, "mode": mode, "experiment": experiment,
                           "status": "failed_or_missing", "process_cpu_ms": None, "process_rss_kib": None,
                           "process_hwm_kib": None, "resource_scope": "current profiler does not measure native Windows process CPU/RAM"}
                    try:
                        command = json.loads(command_path.read_text())
                        row["invocation"] = command
                        require(command["native"]["returncode"] == 0, "native process failed/timed out")
                        report = json.loads(report_path.read_text())
                        launches = []
                        if experiment == "fresh":
                            require(report.get("operation") == "fresh_process_per_job" and report.get("mode") == mode
                                    and report.get("reload_scene") is False and len(report["jobs"]) == 3
                                    and report["input"].startswith("\\\\") and PureWindowsPath(report["input"]).name == name,
                                    "fresh wrapper differs")
                            processes = [process(child["report"], mode, name, backend, False, 1) for child in report["jobs"]]
                            launches = [child["subprocess_launch_to_exit_ms"] for child in report["jobs"]]
                        else:
                            processes = [process(report, mode, name, backend, experiment == "reload", 3)]
                        jobs = [job for p in processes for job in p["jobs"]]
                        setups = [p["setup"] for p in processes if p["setup"] is not None]
                        setups += [job["scene_setup"] for job in jobs if job["scene_setup"] is not None]
                        row.update(status="passed", report_sha256=sha(report_path), job_observations=jobs,
                                   scene_preparations=setups, adapters=[p["adapter"] for p in processes],
                                   metrics={"fresh_native_launch_to_exit_ms": series(launches),
                                   "device_init_ms": series(None if p["setup_device"] is None else p["setup_device"]["wall_ms"] for p in processes),
                                   "scene_setup_ms": series(s["total_with_accounting"]["wall_ms"] for s in setups),
                                   "load_and_geometry_ms": series(s["load_and_geometry"]["wall_ms"] for s in setups),
                                   "render_ms": series(j["render"]["wall_ms"] for j in jobs),
                                   "job_total_ms": series(j["job_with_cleanup_and_accounting"]["wall_ms"] for j in jobs),
                                   "png_encode_ms": series(j["png_encode_to_memory"]["wall_ms"] for j in jobs),
                                   "process_scope_amortized_ms_per_job": series(p["setup_jobs_and_cleanup"]["wall_ms"] / len(p["jobs"]) for p in processes),
                                   "explicit_gpu_allocation_bytes": series(s.get("explicit_gpu_allocation_bytes") for s in setups)})
                    except (OSError, ValueError, KeyError, TypeError, AttributeError) as error:
                        row["error"] = str(error)
                    rows.append(row)
    comparisons = {}
    if manifest["stage"] != "profile":
        for backend in BACKENDS:
            result = subprocess.run([sys.executable, str(BASE.parent / "compare_results.py"), str(args.bundle / "results" / backend)],
                                    capture_output=True, text=True, env={**os.environ, "PYTHONDONTWRITEBYTECODE": "1"})
            comparisons[backend] = {"returncode": result.returncode, "stdout": result.stdout, "stderr": result.stderr}
    summary = {"schema_version": 1, "platform": manifest["platform"], "jobs_expected": 162,
               "passed": all(row["status"] == "passed" for row in rows) and all(value["returncode"] == 0 for value in comparisons.values()),
               "experiments": rows, "cli_comparisons": comparisons,
               "resource_limitations": "Native Windows CPU/RSS/HWM are unavailable, not zero. Explicit GPU estimates exclude drivers/shared/system memory. No NAS CPU/cgroup restrictions apply.",
               "timing_scopes": "Outer PowerShell/interop wall is orchestration overhead. Internal fresh launch-to-exit is native child lifetime; cached-frame and reloaded-job/process scopes differ. Preserve all vectors; do not add nested phase walls.",
               "source_provenance": manifest["build"], "analyzer_sha256": sha(Path(__file__))}
    output = args.bundle / "results/windows-summary.json"
    output.write_text(json.dumps(summary, indent=2) + "\n")
    print(f"{sum(row['status']=='passed' for row in rows)}/{len(rows)} native experiment reports passed; {output}")
    return 0 if summary["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
