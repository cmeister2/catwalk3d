#!/usr/bin/env python3
"""Validate and summarize actual NAS renderer reports without inventing missing results."""

import argparse
import hashlib
import json
import math
from pathlib import Path
import statistics
import sys


BASE = Path(__file__).resolve().parent
FROZEN_POLICY = "88f12e77f2ac5b2d30ad7a4383f309aa47ecd6d67ad388eb2291f3a70f79f38f"
IMAGE_ID = "sha256:ffabbe46b4a5b81eb9cd73334047454bd6420a5cc668f43d6b9920bc1ae070dd"
MODES = ["cpu", "shared-cpu", "gpu"]
EXPERIMENTS = ["fresh", "retained", "reload"]


def require(condition, message):
    """Reject invalid evidence even when Python assertions are disabled."""
    if not condition:
        raise ValueError(message)


def sha(path):
    """Hash artifacts and raw reports without assuming their source state."""
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def number(value, name):
    """Require a finite nonnegative measurement; missing is never substituted with zero."""
    require(isinstance(value, (int, float)) and not isinstance(value, bool)
            and math.isfinite(value) and value >= 0, f"invalid {name}: {value!r}")
    return value


def resources(value, name):
    """Validate one memory/CPU endpoint without treating null values as zero."""
    require(isinstance(value, dict), f"missing resources {name}")
    for key in ["rss_kib", "process_lifetime_hwm_kib", "user_cpu_ticks_total", "system_cpu_ticks_total"]:
        require(key in value, f"missing {name}.{key}")
        if value[key] is not None:
            number(value[key], name + "." + key)
    return value


def phase(value, name):
    """Validate a measured interval and its resource boundaries."""
    require(isinstance(value, dict), f"missing phase {name}")
    number(value.get("wall_ms"), name + ".wall_ms")
    for key in ["user_cpu_ms", "system_cpu_ms"]:
        require(key in value, f"missing {name}.{key}")
        if value[key] is not None:
            number(value[key], name + "." + key)
    for boundary in ["before", "after"]:
        resources(value.get(boundary), name + "." + boundary)
    return value


def cpu(value):
    """Keep unavailable CPU-time observations absent."""
    if value is None or value["user_cpu_ms"] is None or value["system_cpu_ms"] is None:
        return None
    return value["user_cpu_ms"] + value["system_cpu_ms"]


def series(values):
    """Preserve every observation, order and missing value alongside descriptive statistics."""
    values = list(values)
    observed = [value for value in values if value is not None]
    return {"values": values, "observed": len(observed), "missing_or_not_applicable": len(values) - len(observed),
            "median": statistics.median(observed) if observed else None,
            "minimum": min(observed) if observed else None, "maximum": max(observed) if observed else None}


def high_water(value):
    """Find the highest sampled process-lifetime HWM, without summing overlapping memory."""
    values = []
    if isinstance(value, dict):
        if value.get("process_lifetime_hwm_kib") is not None:
            values.append(value["process_lifetime_hwm_kib"])
        values.extend(high_water(child) for child in value.values())
    elif isinstance(value, list):
        values.extend(high_water(child) for child in value)
    return max((item for item in values if item is not None), default=None)


def hashes(path):
    """Read the actual deployed artifact hashes captured on the NAS."""
    result = {}
    for line in path.read_text().splitlines():
        digest, name = line.split(maxsplit=1)
        result[name] = digest
    return result


def exit_status(path):
    """Preserve missing or malformed command statuses as failed evidence."""
    if not path.exists():
        return None
    try:
        return int(path.read_text())
    except ValueError as error:
        raise ValueError(f"invalid exit status in {path}") from error


def provenance(manifest, manifest_path, results, stage):
    """Check finalized build identities, input hashes, frozen policy and deployed scripts."""
    require(manifest.get("status") == "finalized", "manifest is not finalized")
    build = manifest["build"]
    for key in ["source_provenance", "compiler", "command", "cargo_lock_sha256", "profile_gpu_source_sha256",
                "catwalk_binary_sha256", "profile_gpu_binary_sha256"]:
        require(build.get(key), f"missing completed-build metadata {key}")
    for key in ["cargo_lock_sha256", "profile_gpu_source_sha256", "catwalk_binary_sha256", "profile_gpu_binary_sha256"]:
        digest = build[key]
        require(isinstance(digest, str) and len(digest) == 64
                and all(character in "0123456789abcdef" for character in digest), f"invalid SHA-256 in {key}")
    require(manifest["container"]["image_id"] == IMAGE_ID, "unexpected runtime image in manifest")
    require(manifest["policy_sha256"] == FROZEN_POLICY == sha(BASE / "policy.json"), "frozen image policy differs")
    deployed = hashes(results / f"deployed-sha256-{stage}.txt")
    expected = {"bin/catwalk": build["catwalk_binary_sha256"], "bin/profile_gpu": build["profile_gpu_binary_sha256"],
                "manifest.json": sha(manifest_path), "policy.json": FROZEN_POLICY}
    expected.update({"inputs/" + item["name"]: item["sha256"] for item in manifest["inputs"]})
    expected["inputs/synthetic-repeated.step.json"] = manifest["synthetic_workload_manifest_sha256"]
    for name in ["run-profile-case.sh", "run-cli-case.sh", "capture-memory.sh"]:
        expected["scripts/" + name] = manifest["scripts_sha256"][name]
    expected["run-on-nas.sh"] = manifest["scripts_sha256"]["run-on-nas.sh"]
    for name, digest in expected.items():
        require(deployed.get(name) == digest, f"deployed hash mismatch/missing: {name}")
    verified = (results / f"checksums-verified-{stage}.txt").read_text()
    require("FAILED" not in verified and "bin/profile_gpu: OK" in verified and "bin/catwalk: OK" in verified,
            "bundle checksum verification is absent or failed")
    host = (results / f"host-before-{stage}.txt").read_text()
    require("image=" + IMAGE_ID in host, "actual runtime image not verified")
    return {"verified": True, "manifest_sha256": sha(manifest_path), "deployed_artifact_count": len(deployed),
            "policy_sha256": FROZEN_POLICY, "profile_gpu_binary_sha256": build["profile_gpu_binary_sha256"]}


def container_memory(path):
    """Report the case-wide cgroup peak separately from per-process observations."""
    lines = path.read_text().splitlines()
    found = {}
    names = {"/memory.max_usage_in_bytes": "peak_bytes", "/memory.limit_in_bytes": "limit_bytes",
             "/memory.memsw.limit_in_bytes": "memory_plus_swap_limit_bytes", "/memory.failcnt": "failcnt"}
    for index, line in enumerate(lines[:-1]):
        for suffix, name in names.items():
            if line.endswith(suffix):
                found[name] = int(lines[index + 1])
        if line == "proc_self_cpus_allowed_list":
            found["cpus_allowed_list"] = lines[index + 1]
    require(found.get("limit_bytes") == found.get("memory_plus_swap_limit_bytes") == 536870912,
            "actual 512 MiB memory/memsw limits not verified")
    require(found.get("cpus_allowed_list") == "3", "actual CPU 3 affinity not verified")
    require("peak_bytes" in found and "failcnt" in found, "missing cgroup peak/fail count")
    found["scope"] = "one container over fresh, retained and reload experiments; includes only charged memory"
    return found


def validate_setup(setup, mode, expected_triangles, synthetic):
    """Validate scene representation and current upload-substage schema."""
    require(isinstance(setup, dict), "scene setup missing")
    phase(setup.get("load_and_geometry"), "setup.load_and_geometry")
    phase(setup.get("total_with_accounting"), "setup.total_with_accounting")
    if mode in ["cpu", "shared-cpu"]:
        expanded = setup.get("expanded_scene")
        require(isinstance(expanded, dict) and expanded.get("triangles") == expected_triangles, "expanded triangle count differs")
        require(setup.get("gpu_prepare_and_upload") is None and setup.get("explicit_gpu_allocation_bytes") is None,
                "CPU path claims GPU allocation/upload")
    if mode == "cpu":
        require(setup.get("compact_scene") is None and setup.get("expand_for_cpu") is None, "CPU import path differs")
    else:
        compact = setup.get("compact_scene")
        require(isinstance(compact, dict) and compact.get("logical_triangles") == expected_triangles,
                "compact logical triangle count differs")
        if synthetic:
            for key, expected in {"meshes": 1, "instances": 64, "source_triangles": 12288, "source_positions": 6146}.items():
                require(compact.get(key) == expected, f"synthetic compact {key} differs")
        phase(setup.get("drop_compact_source"), "setup.drop_compact_source")
    if mode == "shared-cpu":
        phase(setup.get("expand_for_cpu"), "setup.expand_for_cpu")
    if mode == "gpu":
        require(setup.get("expanded_scene") is None and setup.get("expand_for_cpu") is None, "GPU retained expanded CPU scene")
        phase(setup.get("gpu_prepare_and_upload"), "setup.gpu_prepare_and_upload")
        number(setup.get("cpu_geometry_preparation_wall_ms"), "setup.cpu_geometry_preparation_wall_ms")
        number(setup.get("gpu_resource_creation_wall_ms"), "setup.gpu_resource_creation_wall_ms")
        allocated = number(setup.get("explicit_gpu_allocation_bytes"), "setup.explicit_gpu_allocation_bytes")
        require(0 < allocated <= 256 * 1024 * 1024, "GPU explicit allocation outside recorded budget")
    return setup


def validate_process(report, mode, name, reload_scene, jobs, expected_triangles):
    """Check top-level adapter/device fields and per-job setup/CPU-fit timings."""
    require(isinstance(report, dict), "process report must be an object")
    operation = "same_process_reuse_device_reload_scene" if reload_scene else "same_process_reuse_scene_and_device"
    require(report.get("schema_version") == 1 and report.get("operation") == operation, "process schema/operation differs")
    require(report.get("mode") == mode and report.get("reload_scene") is reload_scene, "mode/reload policy differs")
    require(Path(report["input"]).name == name, "input differs")
    require(report.get("width") == report.get("height") == 512 and report.get("supersampling") == 1,
            "size/sampling differs")
    require(report.get("background_srgb") == [245, 247, 250]
            and report.get("fallback_color_srgb") == [100, 160, 220], "colors differ")
    require(isinstance(report.get("jobs"), list) and len(report["jobs"]) == jobs, "job count differs")
    ticks = report.get("cpu_clock_ticks_per_second")
    if ticks is not None:
        require(number(ticks, "cpu clock rate") > 0, "invalid CPU clock rate")
    if mode == "gpu":
        adapter = report.get("adapter")
        require(isinstance(adapter, dict) and adapter.get("vendor") == 0x8086
                and adapter.get("device_type") == "IntegratedGpu" and adapter.get("backend") == "Gl"
                and isinstance(adapter.get("name"), str)
                and "uhd graphics 600" in adapter["name"].lower(), "wrong/unverified hardware adapter")
        phase(report.get("setup_device"), "setup_device")
    else:
        require(report.get("adapter") is None and report.get("setup_device") is None, "CPU mode initialized GPU")
    if reload_scene:
        require(report.get("setup") is None, "reload mode retained a cached scene setup")
    else:
        validate_setup(report.get("setup"), mode, expected_triangles, name == "synthetic-repeated.step")
    for index, job in enumerate(report["jobs"]):
        require(isinstance(job, dict), "job must be an object")
        require(job.get("iteration") == index and job.get("first_render") is (index == 0), "iteration/first render differs")
        require(job.get("first_render_of_scene") is (reload_scene or index == 0), "scene reuse label differs")
        if reload_scene:
            validate_setup(job.get("scene_setup"), mode, expected_triangles, name == "synthetic-repeated.step")
            phase(job.get("scene_cleanup"), "job.scene_cleanup")
        else:
            require(job.get("scene_setup") is None and job.get("scene_cleanup") is None, "cached job unexpectedly reloaded scene")
        render = phase(job.get("render"), "job.render")
        for key in ["cpu_geometry_preparation_wall_ms", "cpu_rasterization_and_resolve_wall_ms"]:
            require(key in render, f"missing current CPU-render substage {key}")
            if mode == "gpu":
                require(render[key] is None, "GPU render mislabeled as CPU rasterization")
            else:
                number(render[key], "render." + key)
        phase(job.get("png_encode_to_memory"), "job.png_encode_to_memory")
        phase(job.get("job_with_cleanup_and_accounting"), "job.total")
        require(job.get("png_write") is None, "profiling unexpectedly wrote a PNG file")
        require(number(job.get("png_bytes"), "PNG bytes") > 0, "empty encoded PNG")
    for key in ["retained_cleanup", "device_cleanup", "setup_jobs_and_cleanup"]:
        phase(report.get(key), key)
    resources(report.get("after_jobs_before_retained_cleanup"), "after-jobs")
    number(report.get("process_entry_to_report_ms"), "process entry wall time")
    return report


def summarize_report(report, experiment, mode, name, expected_triangles):
    """Keep process scopes and measured-job scopes distinct in every summary series."""
    require(isinstance(report, dict), "report must be an object")
    processes = []
    child_stderr = []
    launches = []
    if experiment == "fresh":
        require(report.get("schema_version") == 1 and report.get("operation") == "fresh_process_per_job" and report.get("mode") == mode
                and report.get("reload_scene") is False and isinstance(report.get("jobs"), list)
                and len(report["jobs"]) == 3,
                "fresh wrapper schema/mode/count differs")
        require(Path(report["input"]).name == name, "fresh wrapper input differs")
        for index, child in enumerate(report["jobs"]):
            require(isinstance(child, dict), "fresh child must be an object")
            require(child.get("iteration") == index, "fresh iteration differs")
            launches.append(number(child.get("subprocess_launch_to_exit_ms"), "child launch wall time"))
            processes.append(validate_process(child["report"], mode, name, False, 1, expected_triangles))
            child_stderr.append(child.get("stderr"))
    else:
        processes.append(validate_process(report, mode, name, experiment == "reload", 3, expected_triangles))
        child_stderr.append(None)
    scenes = []
    jobs = []
    for process_index, process in enumerate(processes):
        if process["setup"] is not None:
            scenes.append(process["setup"])
        for job in process["jobs"]:
            if job["scene_setup"] is not None:
                scenes.append(job["scene_setup"])
            jobs.append({"process_index": process_index, **job})
    metrics = {
        "fresh_launch_to_exit_wall_ms": series(launches),
        "process_scope_wall_ms": series(p["setup_jobs_and_cleanup"]["wall_ms"] for p in processes),
        "process_scope_amortized_wall_ms_per_job": series(p["setup_jobs_and_cleanup"]["wall_ms"] / len(p["jobs"]) for p in processes),
        "process_scope_cpu_ms": series(cpu(p["setup_jobs_and_cleanup"]) for p in processes),
        "process_scope_amortized_cpu_ms_per_job": series(None if cpu(p["setup_jobs_and_cleanup"]) is None else cpu(p["setup_jobs_and_cleanup"]) / len(p["jobs"]) for p in processes),
        "job_total_wall_ms": series(j["job_with_cleanup_and_accounting"]["wall_ms"] for j in jobs),
        "job_cpu_ms": series(cpu(j["job_with_cleanup_and_accounting"]) for j in jobs),
        "render_wall_ms": series(j["render"]["wall_ms"] for j in jobs),
        "render_cpu_ms": series(cpu(j["render"]) for j in jobs),
        "cpu_render_geometry_fit_wall_ms": series(j["render"]["cpu_geometry_preparation_wall_ms"] for j in jobs),
        "cpu_rasterization_and_resolve_wall_ms": series(j["render"]["cpu_rasterization_and_resolve_wall_ms"] for j in jobs),
        "png_encoding_wall_ms": series(j["png_encode_to_memory"]["wall_ms"] for j in jobs),
        "png_encoding_cpu_ms": series(cpu(j["png_encode_to_memory"]) for j in jobs),
        "scene_setup_wall_ms": series(s["total_with_accounting"]["wall_ms"] for s in scenes),
        "scene_load_and_geometry_wall_ms": series(s["load_and_geometry"]["wall_ms"] for s in scenes),
        "scene_load_and_geometry_cpu_ms": series(cpu(s["load_and_geometry"]) for s in scenes),
        "cpu_shared_expansion_wall_ms": series(None if s.get("expand_for_cpu") is None else s["expand_for_cpu"]["wall_ms"] for s in scenes),
        "gpu_prepare_and_upload_wall_ms": series(None if s.get("gpu_prepare_and_upload") is None else s["gpu_prepare_and_upload"]["wall_ms"] for s in scenes),
        "gpu_upload_cpu_ms": series(cpu(s.get("gpu_prepare_and_upload")) for s in scenes),
        "gpu_upload_cpu_fitting_and_packing_wall_ms": series(s.get("cpu_geometry_preparation_wall_ms") for s in scenes),
        "gpu_resource_creation_wall_ms": series(s.get("gpu_resource_creation_wall_ms") for s in scenes),
        "explicit_gpu_allocation_bytes": series(s.get("explicit_gpu_allocation_bytes") for s in scenes),
        "sampled_process_lifetime_hwm_kib": series(high_water(p) for p in processes),
        "after_jobs_rss_kib": series(p["after_jobs_before_retained_cleanup"].get("rss_kib") for p in processes),
        "device_init_wall_ms": series(None if p["setup_device"] is None else p["setup_device"]["wall_ms"] for p in processes),
        "device_init_cpu_ms": series(cpu(p["setup_device"]) for p in processes),
        "device_cleanup_wall_ms": series(p["device_cleanup"]["wall_ms"] for p in processes),
        "cached_scene_cleanup_wall_ms": series(p["retained_cleanup"]["wall_ms"] for p in processes),
        "per_job_scene_cleanup_wall_ms": series(None if j["scene_cleanup"] is None else j["scene_cleanup"]["wall_ms"] for j in jobs),
    }
    for phase_name in ["load_and_geometry", "expand_for_cpu", "gpu_prepare_and_upload", "drop_compact_source"]:
        metrics[phase_name + "_after_rss_kib"] = series(None if s.get(phase_name) is None else s[phase_name]["after"]["rss_kib"] for s in scenes)
    resources_complete = all(p["cpu_clock_ticks_per_second"] is not None
                             and p["after_jobs_before_retained_cleanup"].get("rss_kib") is not None
                             and high_water(p) is not None and cpu(p["setup_jobs_and_cleanup"]) is not None for p in processes)
    return {
        "cpu_and_rss_observations_complete": resources_complete,
        "scene_preparation_count": len(scenes), "device_initialization_count": len(processes) if mode == "gpu" else 0,
        "metrics": metrics, "job_observations": jobs, "scene_preparations": scenes,
        "process_observations": [{"adapter": p["adapter"], "stderr_from_fresh_child": child_stderr[index],
            "cpu_clock_ticks_per_second": p["cpu_clock_ticks_per_second"],
            "cpu_tick_resolution_ms": None if p["cpu_clock_ticks_per_second"] is None else 1000 / p["cpu_clock_ticks_per_second"],
            "setup_device": p["setup_device"], "process_entry_to_report_ms": p["process_entry_to_report_ms"],
            "setup_jobs_and_cleanup": p["setup_jobs_and_cleanup"], "device_cleanup": p["device_cleanup"],
            "retained_cleanup": p["retained_cleanup"], "after_jobs_before_retained_cleanup": p["after_jobs_before_retained_cleanup"]}
            for index, p in enumerate(processes)],
    }


def main():
    """Summarize complete and failed cases; return nonzero for missing/invalid evidence."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bundle", type=Path)
    parser.add_argument("--results", type=Path)
    parser.add_argument("--manifest", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    results = args.results or args.bundle / "results"
    stage = "profile" if (results / "manifest-profile.json").exists() else "all"
    manifest_path = args.manifest or (args.bundle / "manifest.json" if (args.bundle / "manifest.json").exists()
                                     else results / f"manifest-{stage}.json")
    errors = []
    try:
        manifest = json.loads(manifest_path.read_text())
        verified = provenance(manifest, manifest_path, results, stage)
    except (OSError, ValueError, KeyError, TypeError) as error:
        verified = {"verified": False, "error": str(error)}
        errors.append(str(error))
        manifest = json.loads((BASE / "manifest-template.json").read_text())
    rows = []
    cases = []
    for name in manifest["workloads"]["supported_inputs"]:
        for mode in MODES:
            directory = results / "profile" / Path(name).stem / mode
            case_errors = []
            status_path = directory / "container.exit-status"
            try:
                container_status = exit_status(status_path)
            except (OSError, ValueError) as error:
                container_status = None
                case_errors.append(str(error))
            memory = None
            try:
                memory = container_memory(directory / "container-memory.txt")
            except (OSError, ValueError, KeyError, TypeError) as error:
                case_errors.append(str(error))
            case = {"input": name, "mode": mode, "container_exit_status": container_status,
                    "container_memory": memory, "memory_limit_hit_during_case": memory is not None and memory["failcnt"] > 0,
                    "container_stderr": (directory / "container.stderr").read_text() if (directory / "container.stderr").exists() else None,
                    "validation_errors": case_errors}
            cases.append(case)
            for experiment in EXPERIMENTS:
                path = directory / f"{experiment}.json"
                status_path = directory / f"{experiment}.exit-status"
                status_error = None
                try:
                    status = exit_status(status_path)
                except (OSError, ValueError) as error:
                    status = None
                    status_error = str(error)
                stderr_path = directory / f"{experiment}.stderr"
                row = {"input": name, "mode": mode, "experiment": experiment, "exit_status": status,
                       "report_path": str(path), "report_sha256": sha(path) if path.exists() else None,
                       "stderr": stderr_path.read_text() if stderr_path.exists() else None,
                       "validation_errors": [status_error] if status_error else [], "status": "failed_or_incomplete"}
                if status == 0 and path.exists():
                    try:
                        report = json.loads(path.read_text())
                        row.update(summarize_report(report, experiment, mode, name,
                                   manifest["workloads"]["expected_logical_triangles"][name]))
                        row["status"] = "passed"
                    except (OSError, ValueError, KeyError, TypeError) as error:
                        row["status"] = "invalid_report"
                        row["validation_errors"].append(str(error))
                row["valid_for_comparison"] = row["status"] == "passed" and verified["verified"] and not case_errors \
                    and row.get("cpu_and_rss_observations_complete", False)
                if container_status != 0:
                    row["case_container_failed"] = True
                rows.append(row)
    summary = {
        "schema_version": 1, "passed": all(r["valid_for_comparison"] for r in rows)
        and all(c["container_exit_status"] == 0 for c in cases),
        "provenance": verified, "global_validation_errors": errors, "summarizer_sha256": sha(Path(__file__)),
        "experiments_expected": 27, "jobs_expected": 81, "cases": cases, "experiments": rows,
        "scope_notes": [
            "Fresh launch-to-exit observations include startup and teardown. Cached/reload process scopes begin after argument/accounting setup and exclude JSON writing; these scopes are not identical.",
            "Cached scene jobs render the same retained scene. Reload jobs include new import/expansion-or-upload, render, encoding and per-job scene teardown with one reused device. Device initialization/teardown are additional once per process.",
            "Amortized process scopes divide the measured setup/jobs/cleanup interval by exactly three jobs (one for each fresh child), not by an assumed steady-state batch.",
            "Phase substages are nested walls; do not add them again to enclosing upload/render totals. CPU ticks are coarse (10ms at100Hz), can round to zero, and exclude GPU execution.",
            "HWM is the maximum sampled lifetime high-water mark within each process, not a per-job peak. Staged RSS, cgroup charges and explicit GPU estimates overlap and must not be added.",
            "Case cgroup peaks cover all three experiments in one container and are not attributed to individual experiments. Intel GPU/driver memory accounting is incomplete.",
            "Nonzero cgroup failcnt is retained as a case-wide memory-pressure flag; successful jobs can still be reported, but constrained timings must not be presented as unconstrained throughput.",
            "CPU versus shared-CPU separates importer representation/conversion costs. Shared-CPU versus GPU still includes expansion versus upload and differing retained representations; it is not GPU arithmetic alone.",
            "All ordered observations and raw report hashes are preserved; failures and missing fields are not replaced by zeros or CPU fallback results.",
            "A failed fresh wrapper can exit before emitting its JSON, leaving earlier child measurements unavailable. Such missing observations are not reconstructed; exit status and diagnostic text are retained."
        ],
    }
    output = args.output or results / "profile-summary.json"
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(summary, indent=2) + "\n")
    print(f"{sum(row['valid_for_comparison'] for row in rows)}/{len(rows)} valid experiment reports; summary: {output}")
    print("input | mode | experiment | render median ms | job median ms | max sampled HWM MiB | explicit GPU MiB")
    for row in rows:
        if row["status"] != "passed":
            print(f"{row['input']} | {row['mode']} | {row['experiment']} | {row['status']}")
            continue
        metrics = row["metrics"]
        hwm = metrics["sampled_process_lifetime_hwm_kib"]["maximum"]
        gpu = metrics["explicit_gpu_allocation_bytes"]["maximum"]
        print(f"{row['input']} | {row['mode']} | {row['experiment']} | {metrics['render_wall_ms']['median']:.3f} | "
              f"{metrics['job_total_wall_ms']['median']:.3f} | {None if hwm is None else round(hwm/1024,3)} | "
              f"{None if gpu is None else round(gpu/1048576,3)}")
    return 0 if summary["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
