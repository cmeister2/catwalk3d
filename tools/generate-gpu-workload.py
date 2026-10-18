#!/usr/bin/env python3
"""Generate a synthetic AP242 assembly for shared-geometry GPU experiments.

This is a controlled repeated-geometry workload, not a production-representative
model. A tessellated asymmetric ellipsoid is stored once and placed in a regular
three-dimensional grid using STEP MAPPED_ITEM records. Alternate occurrences use
exact quarter-turns around Z. Geometry carries no materials or authored normals.

Example:
    python3 tools/generate-gpu-workload.py --output /tmp/repeated.step

The default 32 subdivisions yield 6,146 positions and 12,288 source triangles;
64 occurrences yield 786,432 logical triangles. A JSON witness is written beside
the STEP file by default, with counts, bounds, placements and a source SHA-256.
Only Python's standard library is required. IDs, header and ordering are fixed.
"""

import argparse
import hashlib
import json
import math
from pathlib import Path
import sys


MAX_LOGICAL_TRIANGLES = 5_000_000
MAX_INSTANCES = 4096
RADII_MM = (10.0, 7.0, 4.0)
SPACING_MM = 32.0


def source_mesh(segments):
    """Project a shared integer cube lattice to an outward tessellated ellipsoid.

    Integer lattice identities share cube-face edges and corners exactly. There
    is no geometric tolerance, coordinate rounding or approximate vertex welding.
    """
    positions = []
    triangles = []
    indices = {}

    def vertex(lattice):
        if lattice not in indices:
            length = math.hypot(*lattice)
            positions.append(tuple(radius * value / length
                                   for radius, value in zip(RADII_MM, lattice)))
            indices[lattice] = len(positions)
        return indices[lattice]

    # Each chart's increasing U cross increasing V points out of the cube.
    charts = (
        lambda u, v: (segments, u, v),
        lambda u, v: (-segments, -u, v),
        lambda u, v: (-u, segments, v),
        lambda u, v: (u, -segments, v),
        lambda u, v: (u, v, segments),
        lambda u, v: (-u, v, -segments),
    )
    for chart in charts:
        grid = [[vertex(chart(-segments + 2 * u, -segments + 2 * v))
                 for u in range(segments + 1)] for v in range(segments + 1)]
        for v in range(segments):
            for u in range(segments):
                a, b = grid[v][u], grid[v][u + 1]
                c, d = grid[v + 1][u + 1], grid[v + 1][u]
                triangles.extend(((a, b, c), (a, c, d)))
    if len(positions) != 6 * segments * segments + 2:
        raise RuntimeError("integer cube lattice has inconsistent shared positions")
    return positions, triangles


def reference_geometry(positions, triangles):
    """Measure the authored source facets independently of Catwalk's importer."""
    area = 0.0
    signed_volume = 0.0
    for indices in triangles:
        a, b, c = (positions[index - 1] for index in indices)
        ab = [b[axis] - a[axis] for axis in range(3)]
        ac = [c[axis] - a[axis] for axis in range(3)]
        cross = (ab[1] * ac[2] - ab[2] * ac[1],
                 ab[2] * ac[0] - ab[0] * ac[2],
                 ab[0] * ac[1] - ab[1] * ac[0])
        if sum(cross[axis] * a[axis] for axis in range(3)) <= 0:
            raise RuntimeError("source triangle has degenerate or inward winding")
        area += math.hypot(*cross) / 2
        signed_volume += sum(a[axis] * cross[axis] for axis in range(3)) / 6
    return {"faceted_area_mm2": area, "faceted_signed_volume_mm3": signed_volume}


def occurrences(count):
    """Return separated grid placements with exact integer-axis rotations."""
    edge = 1
    while edge ** 3 < count:
        edge += 1
    placements = []
    for index in range(count):
        translation = [SPACING_MM * value for value in
                       (index % edge, index // edge % edge, index // (edge * edge))]
        quarter_turn = index % 4
        xy = (((1, 0), (0, 1)), ((0, -1), (1, 0)),
              ((-1, 0), (0, -1)), ((0, 1), (-1, 0)))[quarter_turn]
        matrix = [[xy[0][0], xy[0][1], 0, translation[0]],
                  [xy[1][0], xy[1][1], 0, translation[1]],
                  [0, 0, 1, translation[2]], [0, 0, 0, 1]]
        radii = RADII_MM if quarter_turn % 2 == 0 else (
            RADII_MM[1], RADII_MM[0], RADII_MM[2])
        placements.append({
            "index": index,
            "translation_mm": translation,
            "quarter_turns_z": quarter_turn,
            "transform_row_major": matrix,
            "bounds_mm": [[translation[axis] + sign * radii[axis]
                           for axis in range(3)] for sign in (-1, 1)],
        })
    bounds = [[function(placement["bounds_mm"][side][axis]
                        for placement in placements)
               for axis in range(3)] for side, function in enumerate((min, max))]
    return edge, placements, bounds


def step_vector(values):
    """Format finite STEP real components without losing binary64 round trips."""
    return "(" + ",".join(repr(float(value)) for value in values) + ")"


def write_exchange(path, positions, triangles, placements):
    """Write supported AP242 tessellation, explicit product ownership and mappings."""
    with path.open("w", encoding="ascii", newline="\n") as output:
        output.write("ISO-10303-21;\nHEADER;\n")
        output.write("FILE_DESCRIPTION(('Synthetic repeated ellipsoid GPU workload'),'2;1');\n")
        output.write("FILE_NAME('synthetic-repeated-ellipsoid.step','2026-01-01T00:00:00',"
                     "('Catwalk'),('Catwalk'),'generate-gpu-workload.py','Catwalk','');\n")
        output.write("FILE_SCHEMA(('AP242_MANAGED_MODEL_BASED_3D_ENGINEERING_MIM_LF'));\n"
                     "ENDSEC;\nDATA;\n")
        output.write("#1=(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.));\n"
                     "#2=(GEOMETRIC_REPRESENTATION_CONTEXT(3) "
                     "GLOBAL_UNIT_ASSIGNED_CONTEXT((#1)) REPRESENTATION_CONTEXT('',''));\n")
        output.write(f"#10=COORDINATES_LIST('ellipsoid',{len(positions)},(\n")
        output.write(",\n".join(step_vector(position) for position in positions))
        output.write("));\n")
        output.write(f"#11=TRIANGULATED_FACE('ellipsoid',#10,{len(positions)},(),$,(),(\n")
        output.write(",\n".join("(" + ",".join(map(str, triangle)) + ")"
                                for triangle in triangles))
        output.write("));\n#12=TESSELLATED_SOLID('ellipsoid',(#11),$);\n"
                     "#20=CARTESIAN_POINT('',(0.,0.,0.));\n"
                     "#21=DIRECTION('',(0.,0.,1.));\n"
                     "#22=DIRECTION('',(1.,0.,0.));\n"
                     "#23=AXIS2_PLACEMENT_3D('',#20,#21,#22);\n"
                     "#24=TESSELLATED_SHAPE_REPRESENTATION('source part',(#12,#23),#2);\n"
                     "#25=REPRESENTATION_MAP(#23,#24);\n")
        for placement in placements:
            entity = 1000 + 4 * placement["index"]
            matrix = placement["transform_row_major"]
            # AXIS2_PLACEMENT reference direction is the world-space local X column.
            reference = (matrix[0][0], matrix[1][0], matrix[2][0])
            output.write(f"#{entity}=CARTESIAN_POINT('',{step_vector(placement['translation_mm'])});\n")
            output.write(f"#{entity + 1}=DIRECTION('',{step_vector(reference)});\n")
            output.write(f"#{entity + 2}=AXIS2_PLACEMENT_3D('',#{entity},#21,#{entity + 1});\n")
            output.write(f"#{entity + 3}=MAPPED_ITEM('occurrence {placement['index']}',#25,#{entity + 2});\n")
        items = ",".join(f"#{1003 + 4 * placement['index']}" for placement in placements)
        output.write(f"#100=SHAPE_REPRESENTATION('repeated ellipsoids',({items}),#2);\n")
        output.write("#101=APPLICATION_CONTEXT('mechanical');\n"
                     "#102=PRODUCT_CONTEXT('',#101,'mechanical');\n"
                     "#103=PRODUCT_DEFINITION_CONTEXT('',#101,'design');\n"
                     "#104=PRODUCT('synthetic assembly','','',(#102));\n"
                     "#105=PRODUCT_DEFINITION_FORMATION('','',#104);\n"
                     "#106=PRODUCT_DEFINITION('','',#105,#103);\n"
                     "#107=PRODUCT_DEFINITION_SHAPE('','',#106);\n"
                     "#108=SHAPE_DEFINITION_REPRESENTATION(#107,#100);\n"
                     "ENDSEC;\nEND-ISO-10303-21;\n")


def main(argv=None):
    """Validate bounded parameters and emit the exchange and measurement witness."""
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--output", type=Path, required=True, help="destination STEP file")
    parser.add_argument("--instances", type=int, default=64,
                        help=f"number of separated occurrences, 1..{MAX_INSTANCES} (default: 64)")
    parser.add_argument("--segments", type=int, default=32,
                        help="even subdivisions per cube-face edge, 4..128 (default: 32)")
    parser.add_argument("--manifest", type=Path,
                        help="JSON witness destination (default: OUTPUT.json)")
    args = parser.parse_args(argv)
    if not 1 <= args.instances <= MAX_INSTANCES:
        parser.error(f"--instances must be between 1 and {MAX_INSTANCES}")
    if not 4 <= args.segments <= 128 or args.segments % 2:
        parser.error("--segments must be even and between 4 and 128")
    logical_triangles = 12 * args.segments * args.segments * args.instances
    if logical_triangles > MAX_LOGICAL_TRIANGLES:
        parser.error(f"requested {logical_triangles:,} logical triangles exceeds the "
                     f"{MAX_LOGICAL_TRIANGLES:,} STEP preview ceiling")
    if args.output.suffix.lower() not in (".step", ".stp", ".stpnc", ".p21", ".210"):
        parser.error("--output needs a recognized STEP extension")
    manifest_path = args.manifest or Path(str(args.output) + ".json")
    if manifest_path.resolve() == args.output.resolve():
        parser.error("--manifest and --output must be different files")

    positions, triangles = source_mesh(args.segments)
    grid_edge, placements, bounds = occurrences(args.instances)
    measurements = reference_geometry(positions, triangles)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    write_exchange(args.output, positions, triangles, placements)
    source = args.output.read_bytes()
    manifest = {
        "schema_version": 1,
        "workload": "synthetic-repeated-ellipsoid",
        "purpose": "Control shared scene representation and GPU execution independently; "
                   "not a production-representative model or standalone speedup evidence.",
        "generator": "tools/generate-gpu-workload.py",
        "segments": args.segments,
        "source_mesh_count": 1,
        "source_position_count": len(positions),
        "source_triangle_count": len(triangles),
        "instance_count": args.instances,
        "logical_triangle_count": logical_triangles,
        "grid_edge_capacity": grid_edge,
        "grid_spacing_mm": SPACING_MM,
        "source_radii_mm": RADII_MM,
        "source_bounds_mm": [[sign * radius for radius in RADII_MM] for sign in (-1, 1)],
        "expected_world_bounds_mm": bounds,
        "source_reference_measurements": measurements,
        "expected_world_faceted_area_mm2": measurements["faceted_area_mm2"] * args.instances,
        "expected_world_faceted_signed_volume_mm3": measurements["faceted_signed_volume_mm3"] * args.instances,
        "appearance": {"materials": False, "textures": False, "authored_normals": False},
        "step_bytes": len(source),
        "step_sha256": hashlib.sha256(source).hexdigest(),
        "placements": placements,
    }
    manifest_path.parent.mkdir(parents=True, exist_ok=True)
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"output": str(args.output), "manifest": str(manifest_path),
                      "source_positions": len(positions), "source_triangles": len(triangles),
                      "instances": args.instances, "logical_triangles": logical_triangles,
                      "expected_world_bounds_mm": bounds,
                      "step_sha256": manifest["step_sha256"]}, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
