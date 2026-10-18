//! Independent geometry, corner-attribute and resource qualification for indexed instances.

use catwalk3d::{
    instanced::{IndexedMesh, InstancedScene, MAX_EXPANDED_TRIANGLES, MeshInstance},
    render::{RenderOptions, render},
    scene::{
        AlphaMode, Line, Point, Scene, Texture, TextureFilter, TextureInterpolationSpace,
        TextureWrap, Triangle, TriangleAppearance, Vec3,
    },
};
use image::{Rgba, RgbaImage};
use std::sync::Arc;

/// A single triangle with independently distinguishable material and corner channels.
fn surface() -> Scene {
    Scene {
        triangles: vec![Triangle {
            vertices: [
                Vec3::new(-0., 0., 0.),
                Vec3::new(1., 0., 0.),
                Vec3::new(0., 1., 0.),
            ],
            color: Some([12, 34, 56]),
        }],
        appearances: [(
            0,
            TriangleAppearance {
                base_color: [0.2, 0.4, 0.6, 0.8],
                colors: Some([[1., 0., 0., 1.], [0., 1., 0., 0.5], [0., 0., 1., 0.25]]),
                texcoords: Some([[0., 0.], [1., 0.], [0., 1.]]),
                normals: Some([
                    Vec3::new(1., 1., 1.),
                    Vec3::new(1., 0., 1.),
                    Vec3::new(0., 1., 1.),
                ]),
                texture: Some(Texture {
                    image: Arc::new(RgbaImage::from_pixel(2, 2, Rgba([255, 128, 64, 255]))),
                    wrap_s: TextureWrap::ClampToEdge,
                    wrap_t: TextureWrap::MirroredRepeat,
                    filter: TextureFilter::Nearest,
                    min_filter: TextureFilter::Linear,
                    interpolation_space: TextureInterpolationSpace::Srgb,
                }),
                alpha_mode: AlphaMode::Blend,
                unlit: true,
                double_sided: false,
            },
        )]
        .into_iter()
        .collect(),
        ..Scene::default()
    }
}

/// Compare an independently calculated direction with a transformed normal.
fn direction(actual: Vec3, expected: Vec3) {
    let expected = expected.normalized();
    assert!(
        (actual - expected).length() < 1e-12,
        "{actual:?} != {expected:?}"
    );
}

#[test]
fn exact_position_indexing_preserves_signed_zero_faces_and_identity_rendering() {
    let mut original = surface();
    let mut second = original.triangles[0].clone();
    second.vertices[0].x = 0.;
    second.color = Some([60, 70, 80]);
    original.triangles.push(second);
    original.points.push(Point {
        position: Vec3::new(2., 1., 0.),
        color: Some([100, 200, 50]),
    });
    original.lines.push(Line {
        vertices: [Vec3::new(-1., 0., 0.), Vec3::new(-1., 1., 0.)],
        color: None,
    });
    let shared = InstancedScene::from_scene(original.clone()).unwrap();
    assert_eq!(shared.meshes.len(), 1);
    assert_eq!(shared.instances.len(), 1);
    assert_eq!(shared.meshes[0].positions.len(), 4);
    assert_ne!(
        shared.meshes[0].faces[0].indices[0],
        shared.meshes[0].faces[1].indices[0]
    );
    let expanded = shared.to_scene().unwrap();
    for (actual, expected) in expanded.triangles.iter().zip(&original.triangles) {
        assert_eq!(actual.color, expected.color);
        for (actual, expected) in actual.vertices.into_iter().zip(expected.vertices) {
            assert_eq!(
                [actual.x.to_bits(), actual.y.to_bits(), actual.z.to_bits()],
                [
                    expected.x.to_bits(),
                    expected.y.to_bits(),
                    expected.z.to_bits()
                ]
            );
        }
    }
    let actual = &expanded.appearances[&0];
    let expected = &original.appearances[&0];
    assert_eq!(actual.normals, expected.normals);
    assert_eq!(actual.colors, expected.colors);
    assert_eq!(actual.texcoords, expected.texcoords);
    assert_eq!(actual.base_color, expected.base_color);
    assert_eq!(actual.alpha_mode, expected.alpha_mode);
    assert!(Arc::ptr_eq(
        &actual.texture.as_ref().unwrap().image,
        &expected.texture.as_ref().unwrap().image
    ));
    let options = RenderOptions {
        width: 64,
        height: 64,
        ..RenderOptions::default()
    };
    assert_eq!(
        render(&expanded, &options).unwrap(),
        render(&original, &options).unwrap()
    );
}

#[test]
fn row_major_placement_and_inverse_transpose_match_independent_expectations() {
    let original = surface();
    let mut shared = InstancedScene::from_scene(original.clone()).unwrap();
    shared.instances.push(MeshInstance {
        transform: [
            [0., -3., 0., 10.],
            [2., 0., 0., 20.],
            [0., 0., 4., 30.],
            [0., 0., 0., 1.],
        ],
        ..MeshInstance::default()
    });
    let expanded = shared.to_scene().unwrap();
    assert_eq!(expanded.triangles.len(), 2);
    assert_eq!(
        expanded.triangles[0].vertices,
        original.triangles[0].vertices
    );
    assert_eq!(
        expanded.triangles[1].vertices,
        [
            Vec3::new(10., 20., 30.),
            Vec3::new(10., 22., 30.),
            Vec3::new(7., 20., 30.)
        ]
    );
    let appearance = &expanded.appearances[&1];
    let normals = appearance.normals.unwrap();
    direction(normals[0], Vec3::new(-1. / 3., 1. / 2., 1. / 4.));
    direction(normals[1], Vec3::new(0., 1. / 2., 1. / 4.));
    direction(normals[2], Vec3::new(-1. / 3., 0., 1. / 4.));
    assert_eq!(appearance.colors, original.appearances[&0].colors);
    assert_eq!(appearance.texcoords, original.appearances[&0].texcoords);
    assert_eq!(appearance.base_color, [0.2, 0.4, 0.6, 0.8]);
    assert_eq!(appearance.alpha_mode, AlphaMode::Blend);
    assert!(appearance.unlit);
    assert!(!appearance.double_sided);
    let texture = appearance.texture.as_ref().unwrap();
    assert_eq!(texture.wrap_s, TextureWrap::ClampToEdge);
    assert_eq!(texture.wrap_t, TextureWrap::MirroredRepeat);
    assert_eq!(texture.filter, TextureFilter::Nearest);
    assert_eq!(texture.min_filter, TextureFilter::Linear);
    assert_eq!(texture.interpolation_space, TextureInterpolationSpace::Srgb);
}

#[test]
fn mirrored_occurrences_reverse_geometry_and_every_corner_attribute_only_when_requested() {
    let original = surface();
    let mut shared = InstancedScene::from_scene(original.clone()).unwrap();
    shared.instances[0].transform = [
        [-2., 0., 0., 5.],
        [0., 3., 0., 7.],
        [0., 0., 4., 11.],
        [0., 0., 0., 1.],
    ];
    shared.instances.push(MeshInstance {
        reverse_winding: true,
        ..shared.instances[0]
    });
    let expanded = shared.to_scene().unwrap();
    let placed = [
        Vec3::new(5., 7., 11.),
        Vec3::new(3., 7., 11.),
        Vec3::new(5., 10., 11.),
    ];
    assert_eq!(expanded.triangles[0].vertices, placed);
    assert_eq!(
        expanded.triangles[1].vertices,
        [placed[0], placed[2], placed[1]]
    );
    let mut colors = original.appearances[&0].colors.unwrap();
    colors.swap(1, 2);
    assert_eq!(expanded.appearances[&1].colors, Some(colors));
    let mut uvs = original.appearances[&0].texcoords.unwrap();
    uvs.swap(1, 2);
    assert_eq!(expanded.appearances[&1].texcoords, Some(uvs));
    let normals = expanded.appearances[&1].normals.unwrap();
    direction(normals[0], Vec3::new(-1. / 2., 1. / 3., 1. / 4.));
    direction(normals[1], Vec3::new(0., 1. / 3., 1. / 4.));
    direction(normals[2], Vec3::new(-1. / 2., 0., 1. / 4.));
    assert_eq!(expanded.triangles[0].color, expanded.triangles[1].color);
}

#[test]
fn singular_normals_fall_back_and_large_finite_transforms_remain_bounded() {
    let mut shared = InstancedScene::from_scene(surface()).unwrap();
    shared.instances[0].transform[2][2] = 0.;
    let expanded = shared.to_scene().unwrap();
    assert!(expanded.appearances[&0].normals.is_none());
    assert_eq!(
        expanded.triangles[0].vertices,
        surface().triangles[0].vertices
    );
    shared.instances[0] = MeshInstance::default();
    for axis in 0..3 {
        shared.instances[0].transform[axis][axis] = 1e200;
    }
    let expanded = shared.to_scene().unwrap();
    assert!(expanded.positions().all(Vec3::is_finite));
    direction(
        expanded.appearances[&0].normals.unwrap()[0],
        Vec3::new(1., 1., 1.),
    );
}

#[test]
fn point_and_line_only_scenes_are_retained_without_empty_meshes() {
    let source = Scene {
        points: vec![Point {
            position: Vec3::new(1., 2., 3.),
            color: Some([4, 5, 6]),
        }],
        lines: vec![Line {
            vertices: [Vec3::new(1., 0., 0.), Vec3::new(2., 0., 0.)],
            color: None,
        }],
        ..Scene::default()
    };
    assert!(IndexedMesh::from_scene(source.clone()).is_err());
    let shared = InstancedScene::from_scene(source.clone()).unwrap();
    assert!(shared.meshes.is_empty());
    assert!(shared.instances.is_empty());
    let expanded = shared.to_scene().unwrap();
    assert_eq!(expanded.points[0].position, source.points[0].position);
    assert_eq!(expanded.points[0].color, source.points[0].color);
    assert_eq!(expanded.lines[0].vertices, source.lines[0].vertices);
    assert!(InstancedScene::from_scene(Scene::default()).is_err());
    assert!(InstancedScene::default().validate().is_err());
}

#[test]
fn invalid_references_coordinates_appearances_and_matrices_are_rejected() {
    let valid = InstancedScene::from_scene(surface()).unwrap();
    let mut changed = valid.clone();
    changed.instances[0].mesh = usize::MAX;
    assert!(changed.to_scene().is_err());
    let mut changed = valid.clone();
    changed.meshes[0].faces[0].indices[0] = u32::MAX;
    assert!(changed.validate().is_err());
    let mut changed = valid.clone();
    changed.meshes[0].positions[0].x = f64::NAN;
    assert!(changed.validate().is_err());
    let mut changed = valid.clone();
    changed.meshes[0]
        .appearances
        .insert(99, TriangleAppearance::default());
    assert!(changed.validate().is_err());
    let mut changed = valid.clone();
    changed.meshes[0]
        .appearances
        .get_mut(&0)
        .unwrap()
        .base_color[1] = 1.1;
    assert!(changed.validate().is_err());
    let mut changed = valid.clone();
    changed.meshes[0].appearances.get_mut(&0).unwrap().normals = Some([Vec3::default(); 3]);
    assert!(changed.validate().is_err());
    let mut changed = valid.clone();
    changed.meshes[0].appearances.get_mut(&0).unwrap().texcoords = Some([[f64::NAN, 0.]; 3]);
    assert!(changed.validate().is_err());
    let mut changed = valid.clone();
    changed.meshes[0]
        .appearances
        .get_mut(&0)
        .unwrap()
        .alpha_mode = AlphaMode::Mask { cutoff: -1. };
    assert!(changed.validate().is_err());
    let mut changed = valid.clone();
    changed.meshes[0].appearances.get_mut(&0).unwrap().texcoords = None;
    assert!(changed.validate().is_err());
    for matrix in [
        [
            [1., 0., 0., 0.],
            [0., 1., 0., 0.],
            [0., 0., 1., 0.],
            [0.1, 0., 0., 1.],
        ],
        [
            [1., 0., 0., 0.],
            [0., 1., 0., 0.],
            [0., 0., 1., 0.],
            [0., 0., 0., 0.],
        ],
        [
            [f64::INFINITY, 0., 0., 0.],
            [0., 1., 0., 0.],
            [0., 0., 1., 0.],
            [0., 0., 0., 1.],
        ],
    ] {
        let mut changed = valid.clone();
        changed.instances[0].transform = matrix;
        assert!(changed.to_scene().is_err());
    }
    let mut changed = valid.clone();
    changed.meshes[0].positions[1].x = f64::MAX;
    changed.instances[0].transform[0][3] = f64::MAX;
    assert!(changed.to_scene().is_err());
    let mut changed = valid;
    changed.lines.push(Line {
        vertices: [Vec3::default(), Vec3::new(f64::NAN, 0., 0.)],
        color: None,
    });
    assert!(changed.validate().is_err());
}

#[test]
fn repeated_parts_count_logical_triangles_before_any_large_expansion() {
    let mut shared = InstancedScene::from_scene(surface()).unwrap();
    shared.meshes[0].faces = vec![shared.meshes[0].faces[0]; 10_000];
    shared.instances = vec![MeshInstance::default(); MAX_EXPANDED_TRIANGLES / 10_000 + 1];
    assert_eq!(shared.meshes[0].positions.len(), 3);
    assert!(shared.expanded_triangle_count().is_err());
    assert!(shared.to_scene().is_err());
}
