//! Double-precision fitting and bounded, reusable indexed GPU geometry preparation.

use crate::{
    Result,
    instanced::InstancedScene,
    render::RenderOptions,
    scene::{AlphaMode, TriangleAppearance, Vec3},
};
use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;

/// One vertex split at authored attribute seams, retaining shared indexed buffers.
#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
pub(super) struct Vertex {
    /// Rebased mesh-local coordinates.
    pub position: [f32; 3],
    /// Authored local normal, or local geometric normal.
    pub normal: [f32; 3],
    /// Local geometric normal used for winding and two-sided lighting.
    pub face_normal: [f32; 3],
    /// Legacy encoded RGB or linear material RGBA.
    pub color: [f32; 4],
    /// Linear material, unlit, double-sided and authored-normal flags.
    pub flags: [f32; 4],
}

/// Per-occurrence transform attributes; geometry remains in shared mesh buffers.
#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
pub(super) struct Instance {
    /// Three affine clip-space rows; homogeneous output W is one.
    pub clip_rows: [[f32; 4]; 3],
    /// Scaled inverse-transpose rows used for normal directions.
    pub normal_rows: [[f32; 4]; 3],
    /// Geometric winding sign, followed by unused padding.
    pub flags: [f32; 4],
}

/// Global camera direction and final-image resolve controls.
#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
pub(super) struct Uniforms {
    /// Direction toward the viewer in world axes.
    pub view: [f32; 4],
    /// Supersampling factor, output width, output height, linear resolve flag.
    pub settings: [u32; 4],
}

/// One reusable mesh's packed vertices and triangle indices.
pub(super) struct Mesh {
    /// Vertices indexed by complete position/normal/color/flag tuples.
    pub vertices: Vec<Vertex>,
    /// Corner order for nondegenerate source faces.
    pub indices: Vec<u32>,
}

/// One ordered indexed draw, preserving original occurrence/depth-tie order.
pub(super) struct Draw {
    /// Shared mesh-buffer index.
    pub mesh: usize,
    /// Index of the occurrence's instance attributes.
    pub instance: u32,
}

/// CPU-prepared scene, released after uploading its reusable GPU buffers.
pub(super) struct Prepared {
    /// Packed unique mesh buffers.
    pub meshes: Vec<Mesh>,
    /// Ordered occurrence transform attributes.
    pub instances: Vec<Instance>,
    /// Ordered mesh references for drawing.
    pub draws: Vec<Draw>,
    /// Camera and resolve controls.
    pub uniforms: Uniforms,
    /// Approximate explicit GPU allocation bytes, excluding drivers.
    pub allocation_bytes: u64,
    /// Supersampled texture width.
    pub width: u32,
    /// Supersampled texture height.
    pub height: u32,
    /// Encoded straight-alpha background for the initial render target.
    pub background: [f64; 4],
}

/// Evaluate an affine position using the shared scene's row-major placement convention.
fn world(matrix: [[f64; 4]; 4], p: Vec3) -> Vec3 {
    let a = [p.x, p.y, p.z, 1.0];
    let row = |i: usize| (0..4).map(|j| matrix[i][j] * a[j]).sum();
    Vec3::new(row(0), row(1), row(2))
}

/// Unit face normal with the same near-collinearity threshold as the CPU renderer.
fn normal(vertices: [Vec3; 3]) -> Option<Vec3> {
    let cross = (vertices[1] - vertices[0])
        .normalized()
        .cross((vertices[2] - vertices[0]).normalized());
    (cross.length() > 1e-14).then(|| cross.normalized())
}

/// Minimum and maximum finite coordinates.
#[derive(Clone, Copy)]
struct Bounds {
    /// Per-axis lower bound.
    min: Vec3,
    /// Per-axis upper bound.
    max: Vec3,
}

impl Bounds {
    /// Construct bounds populated by the first included position.
    fn empty() -> Self {
        Self {
            min: Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY),
            max: Vec3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY),
        }
    }
    /// Include one finite position.
    fn include(&mut self, p: Vec3) {
        self.min = Vec3::new(
            self.min.x.min(p.x),
            self.min.y.min(p.y),
            self.min.z.min(p.z),
        );
        self.max = Vec3::new(
            self.max.x.max(p.x),
            self.max.y.max(p.y),
            self.max.z.max(p.z),
        );
    }
    /// Compute the center after coordinate scaling has bounded the endpoints.
    fn center(self) -> Vec3 {
        self.min + (self.max - self.min) * 0.5
    }
    /// Largest per-axis extent.
    fn extent(self) -> f64 {
        let d = self.max - self.min;
        d.x.max(d.y).max(d.z)
    }
}

/// Coordinate normalization before loss of precision in GPU vertex buffers.
#[derive(Clone, Copy)]
struct Basis {
    /// Absolute coordinate scale applied before subtraction.
    scale: f64,
    /// Center in initially scaled coordinates.
    center: Vec3,
    /// Maximum scaled axis extent.
    extent: f64,
}

impl Basis {
    /// Normalize a coordinate in double precision.
    fn position(self, p: Vec3) -> Vec3 {
        (p / self.scale - self.center) / self.extent
    }
}

/// Fitted fixed camera, preserving the CPU renderer's exact projected bounds.
struct Fit {
    /// World normalization.
    basis: Basis,
    /// Camera horizontal direction.
    right: Vec3,
    /// Camera upward direction.
    up: Vec3,
    /// Camera direction toward the viewer.
    toward: Vec3,
    /// Projected bounding-box midpoint, with downward Y.
    center: Vec3,
    /// Pixels per normalized world coordinate.
    pixels: f64,
}

/// Visit placed faces without retaining expanded triangles.
fn faces(scene: &InstancedScene, mut visit: impl FnMut([Vec3; 3])) {
    for instance in &scene.instances {
        let mesh = &scene.meshes[instance.mesh];
        for face in &mesh.faces {
            visit(
                face.indices
                    .map(|i| world(instance.transform, mesh.positions[i as usize])),
            );
        }
    }
}

impl Fit {
    /// Fit actual placed vertices in f64; bounding-box corners do not substitute for geometry.
    fn new(scene: &InstancedScene, width: u32, height: u32) -> Result<Self> {
        let mut scale = 0.0_f64;
        faces(scene, |vertices| {
            for p in vertices {
                scale = scale.max(p.x.abs()).max(p.y.abs()).max(p.z.abs());
            }
        });
        let scale = if scale == 0.0 { 1.0 } else { scale };
        let mut bounds = Bounds::empty();
        faces(scene, |vertices| {
            let p = vertices.map(|p| p / scale);
            if normal(p).is_some() {
                for p in p {
                    bounds.include(p);
                }
            }
        });
        if !bounds.min.is_finite() {
            return Err("GPU scene contains only degenerate triangles".into());
        }
        let basis = Basis {
            scale,
            center: bounds.center(),
            extent: bounds.extent().max(f64::MIN_POSITIVE),
        };
        let directions = [
            Vec3::new(1.0, -1.0, 1.0),
            Vec3::new(1.0, -1.0, 0.6),
            Vec3::new(1.0, -0.6, 1.0),
        ];
        let mut chosen = directions[0];
        for direction in directions {
            let mut visible = false;
            faces(scene, |vertices| {
                visible |= normal(vertices.map(|p| p / scale))
                    .is_some_and(|n| n.dot(direction.normalized()).abs() > 1e-8);
            });
            if visible {
                chosen = direction;
                break;
            }
        }
        let toward = chosen.normalized();
        let right = Vec3::new(0.0, 0.0, 1.0).cross(toward).normalized();
        let up = toward.cross(right).normalized();
        let mut projected = Bounds::empty();
        faces(scene, |vertices| {
            if normal(vertices.map(|p| p / scale)).is_some() {
                for p in vertices {
                    let p = basis.position(p);
                    projected.include(Vec3::new(p.dot(right), -p.dot(up), p.dot(toward)));
                }
            }
        });
        let w = f64::from(width);
        let h = f64::from(height);
        let margin = (w.min(h) * 0.075).max(1.0).min(w.min(h) * 0.2);
        let size = projected.max - projected.min;
        let sx = if size.x > 1e-12 {
            (w - 2.0 * margin) / size.x
        } else {
            f64::INFINITY
        };
        let sy = if size.y > 1e-12 {
            (h - 2.0 * margin) / size.y
        } else {
            f64::INFINITY
        };
        let pixels = if sx.min(sy).is_finite() {
            sx.min(sy)
        } else {
            w.min(h)
        };
        Ok(Self {
            basis,
            right,
            up,
            toward,
            center: projected.center(),
            pixels,
        })
    }
}

/// Convert a finite value to single precision, refusing unrepresentable GPU data.
#[allow(
    clippy::cast_possible_truncation,
    reason = "Explicit finite checks bound the intentional GPU precision conversion"
)]
fn f32_value(value: f64) -> Result<f32> {
    let result = value as f32;
    if result.is_finite() {
        Ok(result)
    } else {
        Err("geometry is outside GPU single-precision range".into())
    }
}

/// Convert a finite vector after double-precision rebasing.
fn vector(p: Vec3) -> Result<[f32; 3]> {
    Ok([f32_value(p.x)?, f32_value(p.y)?, f32_value(p.z)?])
}

/// Decode one legacy encoded sRGB component into linear light.
fn linear(value: u8) -> f64 {
    let s = f64::from(value) / 255.0;
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

/// Resolve attribute-only white appearances against the configured legacy color.
#[allow(
    clippy::float_cmp,
    reason = "Exact default values identify the CPU attribute-only material contract"
)]
fn material_color(a: &TriangleAppearance, fallback: [u8; 3]) -> [f64; 4] {
    if (a.normals.is_some() || a.texcoords.is_some())
        && a.base_color == [1.0; 4]
        && a.colors.is_none()
        && a.texture.is_none()
        && matches!(a.alpha_mode, AlphaMode::Opaque)
        && !a.unlit
        && a.double_sided
    {
        let [r, g, b] = fallback.map(linear);
        [r, g, b, 1.0]
    } else {
        a.base_color
    }
}

/// Build a local normalization using only referenced mesh vertices.
fn mesh_basis(mesh: &crate::instanced::IndexedMesh) -> Basis {
    let mut scale = 0.0_f64;
    for face in &mesh.faces {
        for i in face.indices {
            let p = mesh.positions[i as usize];
            scale = scale.max(p.x.abs()).max(p.y.abs()).max(p.z.abs());
        }
    }
    let scale = if scale == 0.0 { 1.0 } else { scale };
    let mut bounds = Bounds::empty();
    for face in &mesh.faces {
        for i in face.indices {
            bounds.include(mesh.positions[i as usize] / scale);
        }
    }
    Basis {
        scale,
        center: bounds.center(),
        extent: if bounds.extent() > 0.0 {
            bounds.extent()
        } else {
            1.0
        },
    }
}

/// Pack a mesh, splitting shared positions only where complete corner attributes differ.
fn pack(
    mesh: &crate::instanced::IndexedMesh,
    basis: Basis,
    options: &RenderOptions,
    budget: u64,
) -> Result<Mesh> {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut lookup = HashMap::<[u32; 17], u32>::new();
    for (face_index, face) in mesh.faces.iter().enumerate() {
        let p = face
            .indices
            .map(|i| mesh.positions[i as usize] / basis.scale);
        let Some(geometric) = normal(p) else {
            continue;
        };
        let appearance = mesh.appearances.get(&face_index);
        let color = face.color.unwrap_or(options.color);
        for (corner, index) in face.indices.iter().enumerate() {
            let mut vertex = Vertex {
                position: vector(basis.position(mesh.positions[*index as usize]))?,
                normal: vector(geometric)?,
                face_normal: vector(geometric)?,
                color: [1.0; 4],
                flags: [0.0, 0.0, 1.0, 0.0],
            };
            if let Some(a) = appearance {
                let mut color = material_color(a, color);
                if let Some(colors) = a.colors {
                    for (component, c) in color.iter_mut().enumerate() {
                        *c *= colors[corner][component];
                    }
                }
                for (component, c) in color.iter().enumerate() {
                    vertex.color[component] = f32_value(*c)?;
                }
                vertex.flags = [
                    1.0,
                    f32::from(u8::from(a.unlit)),
                    f32::from(u8::from(a.double_sided)),
                    f32::from(u8::from(a.normals.is_some())),
                ];
                if let Some(normals) = a.normals {
                    vertex.normal = vector(normals[corner].normalized())?;
                }
            } else {
                vertex.color = [
                    f32::from(color[0]) / 255.0,
                    f32::from(color[1]) / 255.0,
                    f32::from(color[2]) / 255.0,
                    1.0,
                ];
            }
            let key: [u32; 17] = bytemuck::cast(vertex);
            let index = if let Some(index) = lookup.get(&key) {
                *index
            } else {
                let index = u32::try_from(vertices.len())?;
                vertices.push(vertex);
                lookup.insert(key, index);
                index
            };
            indices.push(index);
        }
        let bytes = u64::try_from(vertices.len())? * 68 + u64::try_from(indices.len())? * 4;
        if bytes > budget {
            return Err("GPU mesh exceeds the explicit allocation budget".into());
        }
    }
    Ok(Mesh { vertices, indices })
}

/// Compute double-precision scaled inverse-transpose rows and the determinant sign.
fn normal_reference(matrix: [[f64; 4]; 4]) -> Result<([[f64; 3]; 3], f64)> {
    let scale = matrix[..3]
        .iter()
        .flat_map(|r| &r[..3])
        .map(|v| v.abs())
        .fold(0.0_f64, f64::max);
    if scale == 0.0 {
        return Err("GPU path requires nonsingular instance transforms".into());
    }
    let a: [[f64; 3]; 3] = std::array::from_fn(|i| std::array::from_fn(|j| matrix[i][j] / scale));
    let cofactors = [
        [
            a[1][1] * a[2][2] - a[1][2] * a[2][1],
            a[1][2] * a[2][0] - a[1][0] * a[2][2],
            a[1][0] * a[2][1] - a[1][1] * a[2][0],
        ],
        [
            a[0][2] * a[2][1] - a[0][1] * a[2][2],
            a[0][0] * a[2][2] - a[0][2] * a[2][0],
            a[0][1] * a[2][0] - a[0][0] * a[2][1],
        ],
        [
            a[0][1] * a[1][2] - a[0][2] * a[1][1],
            a[0][2] * a[1][0] - a[0][0] * a[1][2],
            a[0][0] * a[1][1] - a[0][1] * a[1][0],
        ],
    ];
    let determinant: f64 = (0..3).map(|j| a[0][j] * cofactors[0][j]).sum();
    if determinant == 0.0 || !determinant.is_finite() {
        return Err("GPU path requires representable nonsingular normal transforms".into());
    }
    let sign = determinant.signum();
    let magnitude = cofactors
        .iter()
        .flatten()
        .map(|v| v.abs())
        .fold(0.0_f64, f64::max);
    let mut rows = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            rows[i][j] = cofactors[i][j] / magnitude * sign;
        }
    }
    Ok((rows, sign))
}

/// Convert the reference normal matrix to the shader's padded single-precision ABI.
fn normal_matrix(matrix: [[f64; 4]; 4]) -> Result<([[f32; 4]; 3], f32)> {
    let (reference, sign) = normal_reference(matrix)?;
    let mut rows = [[0.0; 4]; 3];
    for i in 0..3 {
        for j in 0..3 {
            rows[i][j] = f32_value(reference[i][j])?;
        }
    }
    Ok((rows, f32_value(sign)?))
}

/// Compare source packing and placed CPU degeneracy decisions before discarding a face.
fn placed_normals(
    vertices: [Vec3; 3],
    local: Basis,
    matrix: [[f64; 4]; 4],
    world_scale: f64,
    reverse: bool,
) -> Result<Option<(Vec3, Vec3)>> {
    let source = normal(vertices.map(|position| position / local.scale));
    let placed = normal(vertices.map(|position| world(matrix, position) / world_scale));
    match (source, placed) {
        (None, None) => Ok(None),
        (Some(source), Some(placed)) => Ok(Some((source, if reverse { -placed } else { placed }))),
        _ => Err("GPU placement changes the CPU face degeneracy decision; use CPU fallback".into()),
    }
}

/// Compare simulated shader normal arithmetic with an independently normalized f64 direction.
fn check_normal_precision(
    input: Vec3,
    rounded: [f32; 3],
    reference: [[f64; 3]; 3],
    packed: [[f32; 4]; 3],
) -> Result<Vec3> {
    // Retain the authored f64 direction for the reference; only the simulated
    // shader input uses the normalized, rounded values actually packed above.
    let scale = input.x.abs().max(input.y.abs()).max(input.z.abs());
    let precise = input / scale;
    let precise = [precise.x, precise.y, precise.z];
    let expected: [f64; 3] = std::array::from_fn(|row| {
        (0..3)
            .map(|column| reference[row][column] * precise[column])
            .sum()
    });
    let expected = Vec3::new(expected[0], expected[1], expected[2]).normalized();
    let rounded = [rounded[0], rounded[1], rounded[2], 0.0];
    let actual: [f32; 3] = std::array::from_fn(|row| {
        (0..4)
            .map(|column| packed[row][column] * rounded[column])
            .sum()
    });
    let length = actual.iter().map(|value| value * value).sum::<f32>().sqrt();
    // WGSL unit_or_zero discards lengths <=1e-14. Keep extra headroom so a
    // backend's dot-product rounding cannot turn an accepted normal into zero.
    if !length.is_finite() || length <= 2e-14 {
        return Err("GPU normal loses its direction at shader precision; use CPU fallback".into());
    }
    let actual = Vec3::new(
        f64::from(actual[0] / length),
        f64::from(actual[1] / length),
        f64::from(actual[2] / length),
    );
    if !actual.is_finite() || expected.length() == 0.0 || (actual - expected).length() > 1e-5 {
        return Err("GPU normal exceeds the direction error tolerance; use CPU fallback".into());
    }
    Ok(actual)
}

/// Compose rebased local placement and fitted projection before GPU precision conversion.
fn instance(
    matrix: [[f64; 4]; 4],
    reverse: bool,
    local: Basis,
    fit: &Fit,
    width: u32,
    height: u32,
) -> Result<Instance> {
    let ratio = local.scale / fit.basis.scale;
    let lc = [local.center.x, local.center.y, local.center.z];
    let wc = [fit.basis.center.x, fit.basis.center.y, fit.basis.center.z];
    let mut placed = [[0.0; 4]; 3];
    for i in 0..3 {
        for j in 0..3 {
            placed[i][j] = matrix[i][j] * ratio * local.extent / fit.basis.extent;
        }
        placed[i][3] = ((0..3).map(|j| matrix[i][j] * ratio * lc[j]).sum::<f64>()
            + matrix[i][3] / fit.basis.scale
            - wc[i])
            / fit.basis.extent;
    }
    let directions = [fit.right, fit.up, fit.toward];
    let factors = [
        2.0 * fit.pixels / f64::from(width),
        2.0 * fit.pixels / f64::from(height),
        -0.25,
    ];
    let offsets = [-fit.center.x * factors[0], fit.center.y * factors[1], 0.5];
    let mut rows = [[0.0; 4]; 3];
    for i in 0..3 {
        let direction = [directions[i].x, directions[i].y, directions[i].z];
        for j in 0..4 {
            rows[i][j] = f32_value(
                (0..3).map(|k| direction[k] * placed[k][j]).sum::<f64>() * factors[i]
                    + if j == 3 { offsets[i] } else { 0.0 },
            )?;
        }
    }
    let (normal_rows, sign) = normal_matrix(matrix)?;
    Ok(Instance {
        clip_rows: rows,
        normal_rows,
        flags: [if reverse { -sign } else { sign }, 0.0, 0.0, 0.0],
    })
}

/// Reject changed face validity, normal directions or placement errors beyond 0.05 sample pixels.
fn check_precision(
    mesh: &crate::instanced::IndexedMesh,
    local: Basis,
    matrix: [[f64; 4]; 4],
    reverse: bool,
    packed: &Instance,
    fit: &Fit,
    dimensions: [u32; 2],
) -> Result<()> {
    let [width, height] = dimensions;
    let (reference, _) = normal_reference(matrix)?;
    for (face_index, face) in mesh.faces.iter().enumerate() {
        let Some((source_normal, world_normal)) = placed_normals(
            face.indices.map(|index| mesh.positions[index as usize]),
            local,
            matrix,
            fit.basis.scale,
            reverse,
        )?
        else {
            // Neither renderer draws this face; its coordinates and unused
            // appearance normals must not cause spurious precision refusals.
            continue;
        };
        let geometric = check_normal_precision(
            source_normal,
            vector(source_normal)?,
            reference,
            packed.normal_rows,
        )? * f64::from(packed.flags[0]);
        if (geometric - world_normal).length() > 1e-5 {
            return Err(
                "GPU geometric normal differs from the placed CPU face; use CPU fallback".into(),
            );
        }
        if let Some(normals) = mesh
            .appearances
            .get(&face_index)
            .and_then(|appearance| appearance.normals)
        {
            for authored in normals {
                let _ = check_normal_precision(
                    authored,
                    vector(authored.normalized())?,
                    reference,
                    packed.normal_rows,
                )?;
            }
        }
        for index in face.indices {
            let position = mesh.positions[index as usize];
            let p = vector(local.position(position))?;
            let p = [p[0], p[1], p[2], 1.0];
            let gpu: [f32; 3] =
                std::array::from_fn(|i| (0..4).map(|j| packed.clip_rows[i][j] * p[j]).sum());
            let world = fit.basis.position(world(matrix, position));
            let expected = [
                (world.dot(fit.right) - fit.center.x) * fit.pixels * 2.0 / f64::from(width),
                (world.dot(fit.up) + fit.center.y) * fit.pixels * 2.0 / f64::from(height),
                0.5 - world.dot(fit.toward) * 0.25,
            ];
            let errors = [f64::from(width) * 0.5, f64::from(height) * 0.5, 1.0];
            for i in 0..3 {
                let tolerance = if i == 2 { 1e-5 } else { 0.05 };
                if !gpu[i].is_finite()
                    || (f64::from(gpu[i]) - expected[i]).abs() * errors[i] > tolerance
                {
                    return Err("GPU single-precision placement exceeds the image error tolerance; use CPU fallback".into());
                }
            }
        }
    }
    Ok(())
}

impl Prepared {
    /// Validate supported geometry and output settings, fit and pack within an explicit GPU budget.
    ///
    /// # Errors
    /// Returns an error for invalid/unsupported scenes, output settings, precision
    /// loss, unrepresentable transforms or exceeded explicit allocation budgets.
    #[allow(
        clippy::too_many_lines,
        reason = "Keep the checked per-scene allocation accounting and packing sequence together"
    )]
    pub fn new(scene: &InstancedScene, options: &RenderOptions, budget: u64) -> Result<Self> {
        scene.validate()?;
        if !scene.points.is_empty() || !scene.lines.is_empty() {
            return Err("GPU prototype does not support points or lines; use CPU fallback".into());
        }
        let mut used = vec![false; scene.meshes.len()];
        for instance in &scene.instances {
            used[instance.mesh] = true;
        }
        if scene
            .meshes
            .iter()
            .zip(&used)
            .filter(|(_, used)| **used)
            .map(|(mesh, _)| mesh)
            .flat_map(|m| m.appearances.values())
            .any(|a| a.texture.is_some() || !matches!(a.alpha_mode, AlphaMode::Opaque))
        {
            return Err("GPU prototype supports opaque untextured materials; use CPU fallback for textures or alpha coverage".into());
        }
        if options.width == 0
            || options.height == 0
            || options.width > 8192
            || options.height > 8192
            || !(1..=4).contains(&options.supersampling)
        {
            return Err("invalid GPU output dimensions or supersampling".into());
        }
        let width = options
            .width
            .checked_mul(options.supersampling)
            .ok_or("GPU output width overflow")?;
        let height = options
            .height
            .checked_mul(options.supersampling)
            .ok_or("GPU output height overflow")?;
        let pixels = u64::from(width) * u64::from(height);
        if pixels > 16 * 1024 * 1024 {
            return Err("GPU output exceeds 16 million sample pixels".into());
        }
        let final_pixels = u64::from(options.width) * u64::from(options.height);
        let padded_row = u64::from(options.width) * 4;
        let padded_row = padded_row.div_ceil(256) * 256;
        let mut allocation_bytes = pixels * 8
            + final_pixels * 4
            + padded_row * u64::from(options.height)
            + u64::try_from(scene.instances.len())? * 112
            + 32;
        if allocation_bytes > budget {
            return Err("GPU framebuffer exceeds the explicit allocation budget".into());
        }
        let fit = Fit::new(scene, width, height)?;
        let bases: Vec<_> = scene.meshes.iter().map(mesh_basis).collect();
        let mut meshes = Vec::new();
        for (mesh_index, (mesh, basis)) in scene.meshes.iter().zip(&bases).enumerate() {
            if !used[mesh_index] {
                meshes.push(Mesh {
                    vertices: Vec::new(),
                    indices: Vec::new(),
                });
                continue;
            }
            let packed = pack(mesh, *basis, options, budget - allocation_bytes)?;
            allocation_bytes += u64::try_from(packed.vertices.len())? * 68
                + u64::try_from(packed.indices.len())? * 4;
            meshes.push(packed);
        }
        let mut instances = Vec::new();
        let mut draws = Vec::new();
        for placed in &scene.instances {
            if meshes[placed.mesh].indices.is_empty() {
                // Packing can discard every source face. Still refuse a
                // placement that would make any of those faces valid on CPU.
                for face in &scene.meshes[placed.mesh].faces {
                    placed_normals(
                        face.indices
                            .map(|index| scene.meshes[placed.mesh].positions[index as usize]),
                        bases[placed.mesh],
                        placed.transform,
                        fit.basis.scale,
                        placed.reverse_winding,
                    )?;
                }
                continue;
            }
            let transform = instance(
                placed.transform,
                placed.reverse_winding,
                bases[placed.mesh],
                &fit,
                width,
                height,
            )?;
            check_precision(
                &scene.meshes[placed.mesh],
                bases[placed.mesh],
                placed.transform,
                placed.reverse_winding,
                &transform,
                &fit,
                [width, height],
            )?;
            let instance = u32::try_from(instances.len())?;
            instances.push(transform);
            draws.push(Draw {
                mesh: placed.mesh,
                instance,
            });
        }
        let linear_resolve = scene
            .meshes
            .iter()
            .zip(used)
            .any(|(m, used)| used && !m.appearances.is_empty());
        let [vx, vy, vz] = vector(fit.toward)?;
        let background = if options.transparent {
            [0.0; 4]
        } else {
            [
                f64::from(options.background[0]) / 255.0,
                f64::from(options.background[1]) / 255.0,
                f64::from(options.background[2]) / 255.0,
                1.0,
            ]
        };
        Ok(Self {
            meshes,
            instances,
            draws,
            uniforms: Uniforms {
                view: [vx, vy, vz, 0.0],
                settings: [
                    options.supersampling,
                    options.width,
                    options.height,
                    u32::from(linear_resolve),
                ],
            },
            allocation_bytes,
            width,
            height,
            background,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        instanced::MeshInstance,
        scene::{Line, Scene, Triangle},
    };

    /// Two coplanar faces share all corner attributes and four unique positions.
    fn square() -> InstancedScene {
        let a = Vec3::new(0., 0., 0.);
        let b = Vec3::new(1., 0., 0.);
        let c = Vec3::new(1., 1., 0.);
        let d = Vec3::new(0., 1., 0.);
        InstancedScene::from_scene(Scene {
            triangles: vec![
                Triangle {
                    vertices: [a, b, c],
                    color: None,
                },
                Triangle {
                    vertices: [a, c, d],
                    color: None,
                },
            ],
            ..Scene::default()
        })
        .unwrap()
    }

    /// A small framebuffer keeps resource-accounting tests independent of adapter limits.
    fn options() -> RenderOptions {
        RenderOptions {
            width: 64,
            height: 48,
            supersampling: 2,
            ..RenderOptions::default()
        }
    }

    #[test]
    fn shared_instances_retain_one_indexed_gpu_mesh() {
        let mut scene = square();
        let one = Prepared::new(&scene, &options(), 16 * 1024 * 1024).unwrap();
        for i in 1..100 {
            let mut instance = MeshInstance::default();
            instance.transform[0][3] = f64::from(i) * 2.;
            scene.instances.push(instance);
        }
        let many = Prepared::new(&scene, &options(), 16 * 1024 * 1024).unwrap();
        assert_eq!(many.meshes.len(), 1);
        assert_eq!(many.meshes[0].vertices.len(), 4);
        assert_eq!(many.meshes[0].indices.len(), 6);
        assert_eq!(many.instances.len(), 100);
        assert_eq!(many.allocation_bytes - one.allocation_bytes, 99 * 112);
    }

    #[test]
    fn reflected_nonuniform_normal_matrix_preserves_inverse_transpose_direction() {
        let matrix = [
            [-2., 0., 0., 0.],
            [0., 3., 0., 0.],
            [0., 0., 4., 0.],
            [0., 0., 0., 1.],
        ];
        let (rows, sign) = normal_matrix(matrix).unwrap();
        let actual = Vec3::new(
            f64::from(rows[0][0]),
            f64::from(rows[1][1]),
            f64::from(rows[2][2]),
        )
        .normalized();
        let expected = Vec3::new(-0.5, 1. / 3., 0.25).normalized();
        assert!((actual - expected).length() < 1e-7);
        assert!((sign + 1.).abs() < f32::EPSILON);
    }

    #[test]
    fn unsupported_primitives_and_budget_fail_before_device_allocation() {
        let mut scene = square();
        assert!(Prepared::new(&scene, &options(), 1024).is_err());
        scene.lines.push(Line {
            vertices: [Vec3::new(0., 0., 0.), Vec3::new(1., 0., 0.)],
            color: None,
        });
        assert!(Prepared::new(&scene, &options(), 16 * 1024 * 1024).is_err());
    }

    #[test]
    fn unused_appearance_meshes_do_not_change_resolve_or_allocate_buffers() {
        let mut scene = square();
        let mut unused = scene.meshes[0].clone();
        unused.appearances.insert(0, TriangleAppearance::default());
        let before = Prepared::new(&scene, &options(), 16 * 1024 * 1024).unwrap();
        scene.meshes.push(unused);
        let after = Prepared::new(&scene, &options(), 16 * 1024 * 1024).unwrap();
        assert_eq!(before.allocation_bytes, after.allocation_bytes);
        assert_eq!(after.uniforms.settings[3], 0);
        assert!(after.meshes[1].vertices.is_empty());
    }

    /// Build a scene from independently specified triangle coordinates.
    fn triangles(vertices: &[[Vec3; 3]]) -> InstancedScene {
        InstancedScene::from_scene(Scene {
            triangles: vertices
                .iter()
                .map(|&vertices| Triangle {
                    vertices,
                    color: None,
                })
                .collect(),
            ..Scene::default()
        })
        .unwrap()
    }

    /// Inspect preparation refusal without requiring the packed scene to implement Debug.
    fn refusal(scene: &InstancedScene) -> String {
        Prepared::new(scene, &options(), 16 * 1024 * 1024)
            .err()
            .unwrap()
            .to_string()
    }

    #[test]
    fn source_degenerate_faces_that_become_valid_cannot_bypass_empty_packed_mesh_checks() {
        let mut scene = triangles(&[[
            Vec3::new(0., 0., 0.),
            Vec3::new(1., 0., 0.),
            Vec3::new(1., 1e-15, 0.),
        ]]);
        scene.instances[0].transform[1][1] = 1e5;
        // The source would pack zero indices, while the independently placed
        // triangle's unit-edge cross is 1e-10 and is retained by the CPU.
        assert!(refusal(&scene).contains("degeneracy"));
    }

    #[test]
    fn source_valid_faces_that_become_degenerate_require_whole_scene_fallback() {
        let mut scene = triangles(&[
            [
                Vec3::new(0., 0., 0.),
                Vec3::new(1., 0., 0.),
                Vec3::new(1., 1e-10, 0.),
            ],
            [
                Vec3::new(0., 0., 0.),
                Vec3::new(1., 0., 0.),
                Vec3::new(0., 0., 1.),
            ],
        ]);
        scene.instances[0].transform[1][1] = 1e-5;
        // The XZ face remains valid, so a valid global fit cannot conceal the
        // other face crossing the CPU's 1e-14 degeneracy threshold.
        assert!(refusal(&scene).contains("degeneracy"));
    }

    #[test]
    fn anisotropic_geometric_normals_below_shader_normalization_threshold_are_refused() {
        let mut scene = triangles(&[[
            Vec3::new(0., 0., 0.),
            Vec3::new(0., 1., 0.),
            Vec3::new(0., 0., 1.),
        ]]);
        scene.instances[0].transform[0][0] = 1e16;
        // All vertices stay in YZ and are placed exactly. Scaling the inverse
        // transpose for the shader nevertheless reduces the X normal to 1e-16.
        assert!(refusal(&scene).contains("normal loses its direction"));
    }

    #[test]
    fn anisotropic_authored_normals_are_checked_even_when_face_normals_remain_safe() {
        let mut scene = square();
        scene.meshes[0].appearances.insert(
            0,
            TriangleAppearance {
                normals: Some([Vec3::new(1., 0., 0.); 3]),
                ..TriangleAppearance::default()
            },
        );
        scene.instances[0].transform[0][0] = 1e16;
        scene.instances[0].transform[1][1] = 1e16;
        // Geometric Z normals transform to unit Z; the authored X normals
        // would instead become zero in WGSL unit_or_zero.
        assert!(refusal(&scene).contains("normal loses its direction"));
    }

    #[test]
    fn cancellation_in_rounded_authored_normals_is_refused_by_direction_error() {
        let mut scene = square();
        scene.meshes[0].appearances.insert(
            0,
            TriangleAppearance {
                normals: Some([Vec3::new(1., 1. + 1e-8, 1.); 3]),
                ..TriangleAppearance::default()
            },
        );
        scene.instances[0].transform = [
            [1., 1., 0., 0.],
            [0., 1e-8, 0., 0.],
            [0., 0., 1., 0.],
            [0., 0., 0., 1.],
        ];
        // The transformed vector is comfortably above the shader's zero
        // threshold, but f32 rounding erases its meaningful Y component.
        assert!(refusal(&scene).contains("normal exceeds the direction error"));
    }

    #[test]
    fn ordinary_reflected_sheared_instances_keep_authored_and_geometric_normals() {
        let mut scene = square();
        for face in 0..2 {
            scene.meshes[0].appearances.insert(
                face,
                TriangleAppearance {
                    normals: Some([
                        Vec3::new(1., 1., 1.),
                        Vec3::new(1., 0., 1.),
                        Vec3::new(0., 1., 1.),
                    ]),
                    ..TriangleAppearance::default()
                },
            );
        }
        scene.instances[0].transform = [
            [-2., 0.5, 0., 3.],
            [0., 3., 0., -4.],
            [0., 0., 4., 5.],
            [0., 0., 0., 1.],
        ];
        scene.instances.push(MeshInstance {
            reverse_winding: true,
            ..scene.instances[0]
        });
        let prepared = Prepared::new(&scene, &options(), 16 * 1024 * 1024).unwrap();
        assert_eq!(prepared.instances.len(), 2);
        assert!((prepared.instances[0].flags[0] + 1.).abs() < f32::EPSILON);
        assert!((prepared.instances[1].flags[0] - 1.).abs() < f32::EPSILON);
    }

    #[test]
    fn faces_degenerate_in_both_spaces_do_not_trigger_unused_precision_checks() {
        let scene = square();
        let fit = Fit::new(&scene, 128, 96).unwrap();
        let degenerate = triangles(&[[
            Vec3::new(0., 0., 0.),
            Vec3::new(1., 0., 0.),
            Vec3::new(2., 0., 0.),
        ]]);
        let mesh = &degenerate.meshes[0];
        let unused = Instance {
            clip_rows: [[f32::NAN; 4]; 3],
            normal_rows: [[0.; 4]; 3],
            flags: [0.; 4],
        };
        check_precision(
            mesh,
            mesh_basis(mesh),
            MeshInstance::default().transform,
            false,
            &unused,
            &fit,
            [128, 96],
        )
        .unwrap();
    }
}
