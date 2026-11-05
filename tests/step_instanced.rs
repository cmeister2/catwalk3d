//! Shared STEP source meshes, retained placements and legacy geometry equivalence.

use catwalk3d::{
    Result, formats,
    scene::{Scene, Vec3},
};
use std::{collections::BTreeSet, fs, path::PathBuf};

/// Resolve a retained scientific/CAD fixture.
fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/formats/scientific_cad")
        .join(name)
}

/// Compare all geometry and appearances after deterministic instance expansion.
fn assert_matches_legacy(expected: &Scene, actual: &Scene) {
    assert_eq!(expected.triangles.len(), actual.triangles.len());
    for (expected, actual) in expected.triangles.iter().zip(&actual.triangles) {
        assert_eq!(expected.vertices, actual.vertices);
        assert_eq!(expected.color, actual.color);
    }
    assert_eq!(expected.points.len(), actual.points.len());
    assert_eq!(expected.lines.len(), actual.lines.len());
    for (expected, actual) in expected.points.iter().zip(&actual.points) {
        assert_eq!(expected.position, actual.position);
        assert_eq!(expected.color, actual.color);
    }
    for (expected, actual) in expected.lines.iter().zip(&actual.lines) {
        assert_eq!(expected.vertices, actual.vertices);
        assert_eq!(expected.color, actual.color);
    }
    assert_eq!(expected.appearances.len(), actual.appearances.len());
    for (index, expected) in &expected.appearances {
        let actual = &actual.appearances[index];
        assert_eq!(expected.base_color, actual.base_color);
        assert_eq!(expected.colors, actual.colors);
        assert_eq!(expected.texcoords, actual.texcoords);
        assert_eq!(expected.alpha_mode, actual.alpha_mode);
        assert_eq!(expected.unlit, actual.unlit);
        assert_eq!(expected.double_sided, actual.double_sided);
        assert_eq!(expected.texture.is_some(), actual.texture.is_some());
        if let (Some(expected), Some(actual)) = (&expected.texture, &actual.texture) {
            assert_eq!(expected.image, actual.image);
            assert_eq!(expected.wrap_s, actual.wrap_s);
            assert_eq!(expected.wrap_t, actual.wrap_t);
            assert_eq!(expected.filter, actual.filter);
            assert_eq!(expected.min_filter, actual.min_filter);
            assert_eq!(expected.interpolation_space, actual.interpolation_space);
        }
        match (expected.normals, actual.normals) {
            (Some(expected), Some(actual)) => {
                for (expected, actual) in expected.into_iter().zip(actual) {
                    assert!((expected - actual).length() < 1e-12);
                }
            }
            (None, None) => {}
            _ => panic!("authored corner normals were lost"),
        }
    }
}

/// Rounded authored fixture coordinates suitable for exact set comparison.
#[allow(
    clippy::cast_possible_truncation,
    reason = "Small authored coordinates are compared in integer micro-units."
)]
fn coordinates(point: Vec3) -> [i64; 3] {
    [point.x, point.y, point.z].map(|value| (value * 1e6).round() as i64)
}

/// Eight authored corners of the source cuboid, independent of either importer.
fn box_corners() -> impl Iterator<Item = Vec3> {
    [2., 4.].into_iter().flat_map(|x| {
        [3., 6.]
            .into_iter()
            .flat_map(move |y| [4., 8.].into_iter().map(move |z| Vec3::new(x, y, z)))
    })
}

#[test]
fn repeated_product_occurrences_retain_one_local_indexed_mesh() -> Result<()> {
    let path = fixture("step-repeated-product-assembly.step");
    let shared = formats::load_instanced(&path)?;
    assert_eq!(shared.meshes.len(), 1);
    assert_eq!(shared.instances.len(), 2);
    assert_eq!(shared.meshes[0].positions.len(), 8);
    assert!(shared.instances.iter().all(|instance| instance.mesh == 0));
    let expanded = shared.to_scene()?;
    assert_eq!(expanded.triangles.len(), shared.meshes[0].faces.len() * 2);
    assert_matches_legacy(&formats::load(&path)?, &expanded);

    let expected: BTreeSet<_> = box_corners()
        .flat_map(|point| {
            let local = point - Vec3::new(2., 3., 4.);
            [
                coordinates(Vec3::new(10. - local.y, 20. + local.x, 30. + local.z)),
                coordinates(point - Vec3::new(12., 23., 34.)),
            ]
        })
        .collect();
    assert_eq!(
        expanded
            .positions()
            .map(coordinates)
            .collect::<BTreeSet<_>>(),
        expected
    );
    Ok(())
}

#[test]
fn nested_mappings_compose_all_four_placements_without_copying_source_meshes() -> Result<()> {
    let path = fixture("step-nested-mapped-assembly.step");
    let shared = formats::load_instanced(&path)?;
    assert_eq!(shared.meshes.len(), 1);
    assert_eq!(shared.instances.len(), 4);
    assert_eq!(shared.meshes[0].positions.len(), 8);
    let expanded = shared.to_scene()?;
    assert_matches_legacy(&formats::load(&path)?, &expanded);
    let mut expected = BTreeSet::new();
    for outer in 0..2 {
        for inner in 0..2 {
            for point in box_corners() {
                let local = point - Vec3::new(2., 3., 4.);
                let placed = if inner == 0 {
                    Vec3::new(10. - local.y, local.x, local.z)
                } else {
                    Vec3::new(local.x, 8. - local.y, 1. - local.z)
                } - Vec3::new(5., 2., 1.);
                expected.insert(coordinates(if outer == 0 {
                    placed
                } else {
                    Vec3::new(-placed.y, 20. + placed.x, 5. + placed.z)
                }));
            }
        }
    }
    assert_eq!(
        expanded
            .positions()
            .map(coordinates)
            .collect::<BTreeSet<_>>(),
        expected
    );
    let area: f64 = expanded
        .triangles
        .iter()
        .map(|triangle| {
            (triangle.vertices[1] - triangle.vertices[0])
                .cross(triangle.vertices[2] - triangle.vertices[0])
                .length()
                / 2.
        })
        .sum();
    assert!((area - 208.).abs() < 1e-9);
    Ok(())
}

#[test]
fn analytic_faceted_ap242_and_aliased_step_paths_match_legacy_geometry() -> Result<()> {
    for name in [
        "step-step-translated-box.step",
        "step-stp-cylinder.stp",
        "step-stpnc-translated-box.stpnc",
        "step-p21-translated-box.p21",
        "step-210-translated-box.210",
        "step-cadquery-placed-bodies.step",
        "step-jumper-faceted.stp",
        "step-nist-ap242-tessellated.stp",
    ] {
        let path = fixture(name);
        assert_matches_legacy(
            &formats::load(&path)?,
            &formats::load_instanced(&path)?.to_scene()?,
        );
    }
    Ok(())
}

#[test]
fn non_step_identity_adapters_retain_materials_textures_and_lines() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/formats");
    for name in [
        "obj_appearance/obj-textured-concave.obj",
        "mesh_scene/obj-colored-polyline.obj",
    ] {
        let path = root.join(name);
        assert_matches_legacy(
            &formats::load(&path)?,
            &formats::load_instanced(&path)?.to_scene()?,
        );
    }
    Ok(())
}

/// Temporary malformed STEP documents used to verify common importer refusals.
struct TemporaryDirectory(PathBuf);

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn shared_import_rejects_the_same_missing_and_cyclic_mapping_references() -> Result<()> {
    let source = fs::read_to_string(fixture("step-nested-mapped-assembly.step"))?;
    let directory = TemporaryDirectory(
        std::env::temp_dir().join(format!("catwalk-instanced-refusals-{}", std::process::id())),
    );
    fs::create_dir(&directory.0)?;
    for (index, (old, new)) in [
        (
            "REPRESENTATION_MAP(#1032,#1030)",
            "REPRESENTATION_MAP(#1032,#1060)",
        ),
        (
            "REPRESENTATION_MAP(#1003,#10)",
            "REPRESENTATION_MAP(#1003,#999999)",
        ),
        (
            "AXIS2_PLACEMENT_3D('',#1010,#1001,#1011)",
            "AXIS2_PLACEMENT_3D('',#1010,#1001,#1001)",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert!(
            source.contains(old),
            "malformed case must modify the authored fixture"
        );
        let path = directory.0.join(format!("invalid-{index}.step"));
        fs::write(&path, source.replace(old, new))?;
        assert!(formats::load(&path).is_err(), "legacy case {index}");
        assert!(
            formats::load_instanced(&path).is_err(),
            "shared case {index}"
        );
    }
    Ok(())
}
