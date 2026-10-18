#!/usr/bin/env python3
"""Compare unchanged PNG coordinates using Pillow and a recorded tolerance policy."""

import argparse
import hashlib
import json
import math
from pathlib import Path
import statistics
import sys

from PIL import Image, ImageChops, ImageFilter, ImageStat, __version__ as PILLOW_VERSION


def digest(path):
    """Hash encoded bytes so exact fallback and comparison provenance are explicit."""
    return hashlib.sha256(path.read_bytes()).hexdigest()


def foreground(image, transparent, background, threshold):
    """Identify non-background coverage without fitting or aligning either image."""
    if transparent:
        return image.getchannel("A").point(lambda alpha: 255 if alpha > 0 else 0)
    values = bytearray()
    for red, green, blue, alpha in image.getdata():
        distance = max(abs(red - background[0]), abs(green - background[1]), abs(blue - background[2]))
        values.append(255 if alpha != 255 or distance > threshold else 0)
    return Image.frombytes("L", image.size, bytes(values))


def centroid(mask):
    """Return the binary coverage centroid in original pixel coordinates."""
    count = horizontal = vertical = 0
    width = mask.width
    for index, value in enumerate(mask.getdata()):
        if value:
            count += 1
            horizontal += index % width
            vertical += index // width
    return None if not count else [horizontal / count, vertical / count]


def errors(difference, mask=None):
    """Summarize straight RGBA byte errors globally or inside a binary mask."""
    count = difference.width * difference.height if mask is None else mask.histogram()[255]
    if not count:
        return {"pixels": 0, "max_abs_channel_error": None, "mean_abs_channel_error": None}
    stats = ImageStat.Stat(difference, mask=mask)
    maximum = ImageChops.lighter(
        ImageChops.lighter(difference.getchannel("R"), difference.getchannel("G")),
        ImageChops.lighter(difference.getchannel("B"), difference.getchannel("A")),
    )
    histogram = maximum.histogram(mask=mask)
    return {
        "pixels": count,
        "changed_pixels": count - histogram[0],
        "max_abs_channel_error": max(high for _, high in stats.extrema),
        "max_abs_rgba": [high for _, high in stats.extrema],
        "mean_abs_channel_error": statistics.mean(stats.mean),
        "mean_abs_rgba": stats.mean,
        "rmse_rgba": [math.sqrt(value / count) for value in stats.sum2],
    }


def compare(reference_path, candidate_path, policy_path, exact=False, transparent=False, background=(245, 247, 250)):
    """Return all metrics and independent pass/fail gates; never optimize alignment."""
    policy = json.loads(policy_path.read_text())
    with Image.open(reference_path) as source:
        if source.format != "PNG":
            raise ValueError("reference must be PNG")
        reference = source.convert("RGBA")
    with Image.open(candidate_path) as source:
        if source.format != "PNG":
            raise ValueError("candidate must be PNG")
        candidate = source.convert("RGBA")
    report = {
        "schema_version": 1,
        "policy": "exact_fallback" if exact else "gpu_tolerance",
        "policy_sha256": digest(policy_path),
        "comparator_sha256": digest(Path(__file__)),
        "pillow_version": PILLOW_VERSION,
        "reference": str(reference_path),
        "candidate": str(candidate_path),
        "reference_sha256": digest(reference_path),
        "candidate_sha256": digest(candidate_path),
        "reference_size": list(reference.size),
        "candidate_size": list(candidate.size),
        "transparent_background": transparent,
        "opaque_background_rgb": list(background),
        "alignment": "original coordinates; no resize, translation, crop or registration",
        "criteria": {"dimensions_match": reference.size == candidate.size},
    }
    if reference.size != candidate.size:
        report["passed"] = False
        return report
    difference = ImageChops.difference(reference, candidate)
    report["global_rgba_errors"] = errors(difference)
    encoded_equal = report["reference_sha256"] == report["candidate_sha256"]
    report["encoded_png_identical"] = encoded_equal
    report["decoded_rgba_identical"] = reference.tobytes() == candidate.tobytes()
    if exact:
        report["criteria"]["decoded_rgba_identical"] = report["decoded_rgba_identical"]
        if policy["exact_fallback_requires_encoded_png_identity"]:
            report["criteria"]["encoded_png_identical"] = encoded_equal
        report["passed"] = all(report["criteria"].values())
        return report
    threshold = policy["foreground_rgb_background_difference_threshold"]
    reference_mask = foreground(reference, transparent, background, threshold)
    candidate_mask = foreground(candidate, transparent, background, threshold)
    intersection = ImageChops.darker(reference_mask, candidate_mask)
    union = ImageChops.lighter(reference_mask, candidate_mask)
    reference_area = reference_mask.histogram()[255]
    candidate_area = candidate_mask.histogram()[255]
    intersect_area = intersection.histogram()[255]
    union_area = union.histogram()[255]
    iou = intersect_area / union_area if union_area else None
    reference_bounds = reference_mask.getbbox()
    candidate_bounds = candidate_mask.getbbox()
    bounds_error = None if not reference_bounds or not candidate_bounds else max(
        abs(a - b) for a, b in zip(reference_bounds, candidate_bounds)
    )
    reference_centroid = centroid(reference_mask)
    candidate_centroid = centroid(candidate_mask)
    center_distance = None if reference_centroid is None or candidate_centroid is None else math.dist(
        reference_centroid, candidate_centroid
    )
    relative_area_error = abs(candidate_area - reference_area) / reference_area if reference_area else None
    radius = policy["interior_erosion_radius_pixels"]
    if radius != 1:
        raise ValueError("this recorded protocol requires exactly a one-pixel erosion")
    interior = intersection.filter(ImageFilter.MinFilter(3))
    exterior = ImageChops.invert(union.filter(ImageFilter.MaxFilter(3)))
    interior_errors = errors(difference, interior)
    exterior_errors = errors(difference, exterior)
    report.update({
        "coverage": {
            "reference_pixels": reference_area, "candidate_pixels": candidate_area,
            "intersection_pixels": intersect_area, "union_pixels": union_area, "iou": iou,
            "reference_bounds_xyxy_exclusive": reference_bounds,
            "candidate_bounds_xyxy_exclusive": candidate_bounds,
            "bounds_max_coordinate_error_pixels": bounds_error,
            "reference_centroid_xy": reference_centroid, "candidate_centroid_xy": candidate_centroid,
            "centroid_distance_pixels": center_distance, "relative_area_error": relative_area_error,
        },
        "interior_rgba_errors": interior_errors,
        "background_beyond_one_pixel_edge_rgba_errors": exterior_errors,
    })
    report["criteria"].update({
        "nonempty_foreground": reference_area > 0 and candidate_area > 0,
        "coverage_iou": iou is not None and iou >= policy["coverage_iou_min"],
        "bounds": bounds_error is not None and bounds_error <= policy["bounds_max_error_pixels"],
        "centroid": center_distance is not None and center_distance <= policy["centroid_distance_max_pixels"],
        "coverage_area": relative_area_error is not None and relative_area_error <= policy["coverage_relative_area_error_max"],
        "interior_support": interior_errors["pixels"] >= max(
            policy["interior_min_pixels"], reference_area * policy["interior_min_reference_coverage_fraction"]
        ),
        "interior_max_error": interior_errors["max_abs_channel_error"] is not None
        and interior_errors["max_abs_channel_error"] <= policy["interior_max_abs_channel_error"],
        "interior_mean_error": interior_errors["mean_abs_channel_error"] is not None
        and interior_errors["mean_abs_channel_error"] <= policy["interior_mean_abs_channel_error"],
        "background_error": exterior_errors["pixels"] > 0
        and exterior_errors["max_abs_channel_error"] <= policy["background_far_max_abs_channel_error"],
    })
    report["passed"] = all(report["criteria"].values())
    return report


def main():
    """Write machine-readable metrics and return nonzero when any gate fails."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reference", type=Path)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("--policy-file", type=Path, default=Path(__file__).with_name("policy.json"))
    parser.add_argument("--exact", action="store_true", help="require decoded RGBA and encoded PNG identity")
    parser.add_argument("--transparent", action="store_true", help="derive coverage from nonzero alpha")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    report = compare(args.reference, args.candidate, args.policy_file, args.exact, args.transparent)
    encoded = json.dumps(report, indent=2) + "\n"
    if args.output:
        args.output.write_text(encoded)
    else:
        print(encoded, end="")
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
