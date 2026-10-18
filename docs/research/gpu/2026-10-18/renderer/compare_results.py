#!/usr/bin/env python3
"""Validate the complete native-CLI matrix and compare its downloaded PNG pairs."""

import argparse
import csv
import json
from pathlib import Path
import sys

from compare_png import compare


def main():
    """Keep command failures separate from pixel mismatches; neither may be skipped."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("results", type=Path, help="downloaded bundle/results directory")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    base = Path(__file__).resolve().parent
    rows = []
    for name, support in [("gear", "supported"), ("ap242-tessellated", "supported"),
                          ("synthetic-repeated", "supported"), ("jpeg-textured", "unsupported"),
                          ("alpha-modes", "unsupported")]:
        directory = args.results / "cli" / name / support
        status_path = directory / "status.tsv"
        status = {}
        if status_path.exists():
            with status_path.open() as source:
                for row in csv.DictReader(source, delimiter="\t"):
                    status[row["renderer"], int(row["samples"]), row["background"]] = row
        for samples in [1, 2]:
            for background in ["opaque", "transparent"]:
                candidate_renderer = "auto" if support == "unsupported" else "gpu"
                renderers = ["cpu", candidate_renderer]
                if support == "unsupported":
                    renderers.append("gpu")
                execution_passed = all(
                    (renderer, samples, background) in status
                    and status[renderer, samples, background]["expectation_met"] == "1"
                    for renderer in renderers
                )
                cpu = directory / f"cpu-s{samples}-{background}.png"
                candidate = directory / f"{candidate_renderer}-s{samples}-{background}.png"
                row = {"input": name, "samples": samples, "background": background,
                       "candidate_renderer": candidate_renderer, "execution_passed": execution_passed}
                if execution_passed and cpu.exists() and candidate.exists():
                    row["comparison"] = compare(cpu, candidate, base / "policy.json",
                                                exact=support == "unsupported", transparent=background == "transparent")
                    row["passed"] = row["comparison"]["passed"]
                else:
                    row["passed"] = False
                    row["reason"] = "missing/failed command, incorrect strict-GPU rejection, or absent image"
                rows.append(row)
    report = {"schema_version": 1, "passed": all(row["passed"] for row in rows),
              "comparisons_expected": 20, "comparisons": rows}
    output = args.output or args.results / "comparisons.json"
    output.write_text(json.dumps(report, indent=2) + "\n")
    print(f"{sum(row['passed'] for row in rows)}/{len(rows)} CLI comparisons passed; report: {output}")
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
