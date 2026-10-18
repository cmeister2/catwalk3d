#!/usr/bin/env python3
"""Validate the fixed NAS CPU workload and summarize its raw reports."""

import hashlib
import json
from pathlib import Path
import statistics


BASE = Path(__file__).resolve().parent
MANIFEST = json.loads((BASE / "manifest.json").read_text())


def median(values):
    """Keep unavailable CPU observations absent instead of substituting zero."""
    values = list(values)
    if not values or any(value is None for value in values):
        return None
    return round(statistics.median(values), 3)


def cpu(phase):
    """Return measured user plus kernel CPU milliseconds for one phase."""
    user = phase["user_cpu_ms"]
    system = phase["system_cpu_ms"]
    return None if user is None or system is None else user + system


def cgroup_peak(path):
    """Return the container's lifetime memory peak when its cgroup exposes one."""
    lines = path.read_text().splitlines() if path.exists() else []
    for index, line in enumerate(lines[:-1]):
        if line.endswith("/memory.max_usage_in_bytes") or line.endswith("/memory.peak"):
            return int(lines[index + 1])
    return None


def main():
    """Validate source hashes, exit statuses and report modes before summarizing."""
    results = BASE / "results"
    deployed = {}
    for line in (results / "deployed-sha256.txt").read_text().splitlines():
        digest, path = line.split(maxsplit=1)
        deployed[Path(path).name] = digest
    assert deployed["profile_thumbnail"] == MANIFEST["build"]["binary_sha256"]
    rows = []
    for source in MANIFEST["inputs"]:
        name = source["packaged_name"]
        assert deployed[name] == source["sha256"], f"input hash mismatch: {name}"
        directory = results / Path(name).stem
        statuses = {
            key: int((directory / f"{key}.exit-status").read_text())
            for key in ["container", "fresh", "reuse"]
            if (directory / f"{key}.exit-status").exists()
        }
        row = {"input": name, "repository_path": source["repository_path"], "exit_statuses": statuses}
        if statuses != {"container": 0, "fresh": 0, "reuse": 0}:
            row["status"] = "failed_or_incomplete"
            rows.append(row)
            continue
        fresh = json.loads((directory / "fresh.json").read_text())
        reuse = json.loads((directory / "reuse.json").read_text())
        assert fresh["mode"] == "fresh_process_per_job"
        assert reuse["mode"] == "same_process_reuse_scene"
        assert len(fresh["jobs"]) == len(reuse["jobs"]) == 3
        fresh_jobs = []
        for child in fresh["jobs"]:
            report = child["report"]
            assert report["width"] == report["height"] == 512
            assert report["supersampling"] == 1
            assert len(report["jobs"]) == 1
            fresh_jobs.append(report["jobs"][0])
        assert reuse["width"] == reuse["height"] == 512
        assert reuse["supersampling"] == 1
        assert all(job["load_and_geometry"] is None for job in reuse["jobs"])
        expected_scene = fresh_jobs[0]["scene"]
        assert all(job["scene"] == expected_scene for job in fresh_jobs + reuse["jobs"])
        wall = [child["subprocess_launch_to_exit_ms"] for child in fresh["jobs"]]
        row.update({
            "status": "passed",
            "scene": expected_scene,
            "fresh_launch_to_exit_ms_median": median(wall),
            "fresh_launch_to_exit_ms_min": round(min(wall), 3),
            "fresh_launch_to_exit_ms_max": round(max(wall), 3),
            "fresh_load_and_geometry_ms_median": median(job["load_and_geometry"]["wall_ms"] for job in fresh_jobs),
            "fresh_render_ms_median": median(job["render"]["wall_ms"] for job in fresh_jobs),
            "fresh_png_encode_ms_median": median(job["png_encode_to_memory"]["wall_ms"] for job in fresh_jobs),
            "fresh_job_cpu_ms_median": median(cpu(job["job_with_cleanup_and_accounting"]) for job in fresh_jobs),
            "fresh_process_hwm_kib_max": max(job["job_with_cleanup_and_accounting"]["after"]["process_lifetime_hwm_kib"] for job in fresh_jobs),
            "reuse_one_time_load_and_geometry_ms": round(reuse["setup_load_and_geometry"]["wall_ms"], 3),
            "reuse_render_ms_median": median(job["render"]["wall_ms"] for job in reuse["jobs"]),
            "reuse_png_encode_ms_median": median(job["png_encode_to_memory"]["wall_ms"] for job in reuse["jobs"]),
            "reuse_job_cpu_ms_median": median(cpu(job["job_with_cleanup_and_accounting"]) for job in reuse["jobs"]),
            "reuse_process_lifetime_hwm_kib": max(job["job_with_cleanup_and_accounting"]["after"]["process_lifetime_hwm_kib"] for job in reuse["jobs"]),
            "container_lifetime_memory_peak_bytes": cgroup_peak(directory / "container-memory.txt"),
            "raw_report_sha256": {
                name: hashlib.sha256((directory / name).read_bytes()).hexdigest()
                for name in ["fresh.json", "reuse.json"]
            },
        })
        rows.append(row)
    summary = {
        "schema_version": 1,
        "scope": "NAS CPU-only geometry profiling; three jobs per mode; no GPU comparison",
        "options": MANIFEST["workload"],
        "inputs": rows,
    }
    (BASE / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print("| Input | Triangles | Import ms | Render ms | Encode ms | Fresh total ms | Fresh peak RSS MiB |")
    print("| --- | ---: | ---: | ---: | ---: | ---: | ---: |")
    for row in rows:
        if row["status"] != "passed":
            print(f"| {row['input']} | failed | | | | | |")
            continue
        print(
            f"| {row['input']} | {row['scene']['triangles']:,} "
            f"| {row['fresh_load_and_geometry_ms_median']:,.1f} "
            f"| {row['fresh_render_ms_median']:,.1f} "
            f"| {row['fresh_png_encode_ms_median']:,.1f} "
            f"| {row['fresh_launch_to_exit_ms_median']:,.1f} "
            f"| {row['fresh_process_hwm_kib_max'] / 1024:,.1f} |"
        )


if __name__ == "__main__":
    main()
