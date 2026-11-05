//! Reusable indexed triangle meshes and affine instances, independent of a graphics backend.
//!
//! Positions remain double precision. Appearances stay attached to triangle corners,
//! so indexing positions never welds material, normal or texture-coordinate seams.

use crate::Result;
use crate::scene::{AlphaMode, Line, Point, Scene, Triangle, TriangleAppearance, Vec3};
use image::RgbaImage;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

/// Maximum logical triangle count after expanding all instances into a CPU scene.
pub const MAX_EXPANDED_TRIANGLES: usize = 5_000_000;

/// An indexed triangle retaining its optional legacy sRGB flat color.
#[derive(Clone, Copy, Debug)]
pub struct IndexedTriangle {
    /// Three indices into the containing mesh's position array, in authored order.
    pub indices: [u32; 3],
    /// Optional sRGB flat color; absent colors use the renderer's fallback.
    pub color: Option<[u8; 3]>,
}

/// A reusable surface mesh with independent corner appearances.
#[derive(Clone, Debug, Default)]
pub struct IndexedMesh {
    /// Finite local-space positions, shared only when all coordinate bits agree.
    pub positions: Vec<Vec3>,
    /// Indexed triangles in deterministic authored order.
    pub faces: Vec<IndexedTriangle>,
    /// Face indices with appearances; all attributes remain in face-corner order.
    pub appearances: BTreeMap<usize, TriangleAppearance>,
}

/// One occurrence of a reusable mesh, using a row-major affine transform.
#[derive(Clone, Copy, Debug)]
pub struct MeshInstance {
    /// Index into the containing scene's mesh array.
    pub mesh: usize,
    /// Row-major local-to-world affine matrix; the final row must be `[0, 0, 0, 1]`.
    pub transform: [[f64; 4]; 4],
    /// Swap corners one and two, including corner attributes, after placement.
    /// This is an explicit source policy, not inferred from the matrix determinant.
    pub reverse_winding: bool,
}

impl Default for MeshInstance {
    fn default() -> Self {
        Self {
            mesh: 0,
            transform: [
                [1., 0., 0., 0.],
                [0., 1., 0., 0.],
                [0., 0., 1., 0.],
                [0., 0., 0., 1.],
            ],
            reverse_winding: false,
        }
    }
}

/// Reusable surface geometry plus ordered occurrences and world-space points/lines.
#[derive(Clone, Debug, Default)]
pub struct InstancedScene {
    /// Unique reusable meshes; coincident but distinct source objects may stay separate.
    pub meshes: Vec<IndexedMesh>,
    /// Surface occurrences in their original scene traversal order.
    pub instances: Vec<MeshInstance>,
    /// World-space points retained from legacy scenes, without instance placement.
    pub points: Vec<Point>,
    /// World-space line segments retained from legacy scenes, without instance placement.
    pub lines: Vec<Line>,
}

/// Shared accounting for unique decoded images across every mesh in one scene.
#[derive(Default)]
struct TextureBudget {
    /// Shared image identities already charged to the scene's texture budget.
    images: HashSet<*const RgbaImage>,
    /// Sum of pixels in distinct retained images.
    pixels: u64,
}

impl IndexedMesh {
    /// Index an owned legacy triangle scene by exact position bits, preserving face order.
    ///
    /// This constructor rejects points and lines; use [`InstancedScene::from_scene`]
    /// to preserve a mixed scene. It never welds approximately equal coordinates.
    ///
    /// # Errors
    /// Returns an error for invalid or empty geometry, any point/line primitives,
    /// excessive triangle counts, unrepresentable position indices or allocation failure.
    pub fn from_scene(scene: Scene) -> Result<Self> {
        scene.validate()?;
        if !scene.points.is_empty() || !scene.lines.is_empty() {
            return Err(
                "indexed meshes accept triangles only; use InstancedScene for points/lines".into(),
            );
        }
        if scene.triangles.len() > MAX_EXPANDED_TRIANGLES {
            return Err("indexed mesh exceeds the five million triangle limit".into());
        }
        let mut positions = Vec::new();
        let mut indices = HashMap::new();
        let mut faces = Vec::new();
        faces.try_reserve_exact(scene.triangles.len())?;
        for triangle in scene.triangles {
            let mut face = [0; 3];
            for (corner, position) in triangle.vertices.into_iter().enumerate() {
                let key = [
                    position.x.to_bits(),
                    position.y.to_bits(),
                    position.z.to_bits(),
                ];
                face[corner] = if let Some(&index) = indices.get(&key) {
                    index
                } else {
                    let index = u32::try_from(positions.len())?;
                    positions.try_reserve(1)?;
                    indices.try_reserve(1)?;
                    positions.push(position);
                    indices.insert(key, index);
                    index
                };
            }
            faces.push(IndexedTriangle {
                indices: face,
                color: triangle.color,
            });
        }
        Ok(Self {
            positions,
            faces,
            appearances: scene.appearances,
        })
    }

    /// Validate local positions, face references, corner appearances and texture limits.
    ///
    /// # Errors
    /// Returns an error for empty geometry, non-finite coordinates, missing positions
    /// or faces, invalid appearances, or exceeded triangle/texture limits.
    pub fn validate(&self) -> Result<()> {
        self.validate_with_textures(&mut TextureBudget::default())
    }

    /// Validate a mesh while charging unique images to a shared scene budget.
    fn validate_with_textures(&self, textures: &mut TextureBudget) -> Result<()> {
        if self.faces.is_empty() {
            return Err("indexed mesh contains no triangles".into());
        }
        if self.faces.len() > MAX_EXPANDED_TRIANGLES {
            return Err("indexed mesh exceeds the five million triangle limit".into());
        }
        if self.positions.iter().any(|position| !position.is_finite()) {
            return Err("indexed mesh contains non-finite coordinates".into());
        }
        for face in &self.faces {
            if face
                .indices
                .iter()
                .any(|&index| index as usize >= self.positions.len())
            {
                return Err("indexed triangle refers to a missing position".into());
            }
        }
        for (&face, appearance) in &self.appearances {
            if face >= self.faces.len() {
                return Err("indexed appearance refers to a missing face".into());
            }
            validate_appearance(appearance, textures)?;
        }
        Ok(())
    }
}

impl InstancedScene {
    /// Adapt an owned legacy scene to one identity surface instance and its world-space primitives.
    ///
    /// Point-only and line-only scenes need no mesh or surface instance. Triangle
    /// coordinates and corner attributes retain their exact values on expansion.
    ///
    /// # Errors
    /// Returns an error for an invalid or empty source scene, excessive triangle
    /// counts, unrepresentable position indices or allocation failure.
    pub fn from_scene(scene: Scene) -> Result<Self> {
        scene.validate()?;
        let Scene {
            triangles,
            points,
            lines,
            appearances,
        } = scene;
        let mut output = Self {
            points,
            lines,
            ..Self::default()
        };
        if !triangles.is_empty() {
            let mesh = IndexedMesh::from_scene(Scene {
                triangles,
                appearances,
                ..Scene::default()
            })?;
            output.meshes.try_reserve_exact(1)?;
            output.instances.try_reserve_exact(1)?;
            output.meshes.push(mesh);
            output.instances.push(MeshInstance::default());
        }
        Ok(output)
    }

    /// Count logical triangles with checked references, arithmetic and expansion limits.
    ///
    /// # Errors
    /// Returns an error for a missing mesh reference or more than five million
    /// logical triangles. This does not validate coordinates or appearances.
    pub fn expanded_triangle_count(&self) -> Result<usize> {
        self.instances.iter().try_fold(0_usize, |count, instance| {
            let mesh = self
                .meshes
                .get(instance.mesh)
                .ok_or("instance refers to a missing mesh")?;
            count
                .checked_add(mesh.faces.len())
                .filter(|&total| total <= MAX_EXPANDED_TRIANGLES)
                .ok_or_else(|| {
                    "instanced scene exceeds the five million expanded triangle limit".into()
                })
        })
    }

    /// Validate source meshes, world primitives, affine placements and logical expansion limits.
    ///
    /// Singular affine transforms are permitted; expansion omits their authored
    /// normals so the CPU renderer can use geometric normals instead.
    ///
    /// # Errors
    /// Returns an error for invalid geometry/appearances, empty visible geometry,
    /// missing meshes, non-affine or non-finite matrices, non-finite placed
    /// coordinates, or exceeded triangle/texture limits.
    pub fn validate(&self) -> Result<()> {
        let triangles = self.expanded_triangle_count()?;
        if triangles == 0 && self.points.is_empty() && self.lines.is_empty() {
            return Err("instanced scene contains no renderable geometry".into());
        }
        let mut textures = TextureBudget::default();
        for mesh in &self.meshes {
            mesh.validate_with_textures(&mut textures)?;
        }
        if self.points.iter().any(|point| !point.position.is_finite())
            || self
                .lines
                .iter()
                .flat_map(|line| line.vertices)
                .any(|position| !position.is_finite())
        {
            return Err("instanced scene contains non-finite world-space coordinates".into());
        }
        for instance in &self.instances {
            validate_transform(instance.transform)?;
            for position in &self.meshes[instance.mesh].positions {
                if !transform_position(instance.transform, *position).is_finite() {
                    return Err("instance placement produces non-finite coordinates".into());
                }
            }
        }
        Ok(())
    }

    /// Expand surface occurrences in order into the existing CPU scene representation.
    ///
    /// Positions use the row-major affine transform. Authored normals use its
    /// inverse transpose and are normalized after nonidentity placement. Singular
    /// normal matrices omit the entire face's authored normals. Reversed winding
    /// swaps triangle corners and all associated colors, UVs and normals together.
    /// Identity placement bypasses arithmetic, preserving exact source values.
    ///
    /// # Errors
    /// Returns an error for any invalid scene data, exceeded expansion limits,
    /// non-finite placed coordinates/normals or failed vector allocation.
    pub fn to_scene(&self) -> Result<Scene> {
        self.validate()?;
        let mut output = Scene::default();
        output
            .triangles
            .try_reserve_exact(self.expanded_triangle_count()?)?;
        output.points.try_reserve_exact(self.points.len())?;
        output.lines.try_reserve_exact(self.lines.len())?;
        output.points.extend(self.points.iter().cloned());
        output.lines.extend(self.lines.iter().cloned());
        for instance in &self.instances {
            let mesh = &self.meshes[instance.mesh];
            let normal_matrix = NormalMatrix::new(instance.transform);
            for (index, face) in mesh.faces.iter().enumerate() {
                let mut vertices = face.indices.map(|corner| {
                    transform_position(instance.transform, mesh.positions[corner as usize])
                });
                if instance.reverse_winding {
                    vertices.swap(1, 2);
                }
                if let Some(appearance) = mesh.appearances.get(&index) {
                    let mut appearance = appearance.clone();
                    if let Some(normals) = appearance.normals {
                        appearance.normals = normal_matrix.normals(normals)?;
                    }
                    if instance.reverse_winding {
                        reverse_corners(&mut appearance);
                    }
                    output
                        .appearances
                        .insert(output.triangles.len(), appearance);
                }
                output.triangles.push(Triangle {
                    vertices,
                    color: face.color,
                });
            }
        }
        Ok(output)
    }
}

/// Require a finite affine row-major matrix without perspective division.
#[allow(
    clippy::float_cmp,
    reason = "Affine homogeneous coordinates require the exact final row, not an approximate projective matrix"
)]
fn validate_transform(matrix: [[f64; 4]; 4]) -> Result<()> {
    if matrix.iter().flatten().any(|value| !value.is_finite()) {
        return Err("instance transform contains non-finite components".into());
    }
    if matrix[3] != [0., 0., 0., 1.] {
        return Err(
            "instance transform must be row-major affine with final row [0, 0, 0, 1]".into(),
        );
    }
    Ok(())
}

/// Apply a row-major affine placement, preserving exact bits for identity.
fn transform_position(matrix: [[f64; 4]; 4], position: Vec3) -> Vec3 {
    if matrix == MeshInstance::default().transform {
        return position;
    }
    let values = [position.x, position.y, position.z, 1.];
    let output: [f64; 3] = std::array::from_fn(|row| {
        (0..4)
            .map(|column| matrix[row][column] * values[column])
            .sum()
    });
    Vec3::new(output[0], output[1], output[2])
}

/// A scaled inverse-transpose matrix, or identity/singular special cases.
enum NormalMatrix {
    /// Preserve authored values exactly under identity placement.
    Identity,
    /// A singular affine transform has no normal inverse.
    Singular,
    /// Scaled inverse-transpose columns, with reflection sign retained.
    Cofactors([Vec3; 3]),
}

impl NormalMatrix {
    /// Build scaled cofactors without overflowing large finite matrix components.
    fn new(matrix: [[f64; 4]; 4]) -> Self {
        if matrix == MeshInstance::default().transform {
            return Self::Identity;
        }
        let largest = matrix[..3]
            .iter()
            .flat_map(|row| &row[..3])
            .fold(0_f64, |value, &component| value.max(component.abs()));
        if largest == 0. {
            return Self::Singular;
        }
        let columns: [Vec3; 3] = std::array::from_fn(|column| {
            Vec3::new(
                matrix[0][column] / largest,
                matrix[1][column] / largest,
                matrix[2][column] / largest,
            )
        });
        let mut cofactors = [
            columns[1].cross(columns[2]),
            columns[2].cross(columns[0]),
            columns[0].cross(columns[1]),
        ];
        let determinant = columns[0].dot(cofactors[0]);
        if determinant == 0. {
            return Self::Singular;
        }
        for column in &mut cofactors {
            *column = *column * determinant.signum();
        }
        Self::Cofactors(cofactors)
    }

    /// Transform three corner normals, dropping singular or collapsed normal sets.
    fn normals(&self, normals: [Vec3; 3]) -> Result<Option<[Vec3; 3]>> {
        let columns = match self {
            Self::Identity => return Ok(Some(normals)),
            Self::Singular => return Ok(None),
            Self::Cofactors(columns) => columns,
        };
        let mut output = [Vec3::default(); 3];
        for (index, normal) in normals.into_iter().enumerate() {
            let scale = normal.x.abs().max(normal.y.abs()).max(normal.z.abs());
            let normal = normal / scale;
            let value = columns[0] * normal.x + columns[1] * normal.y + columns[2] * normal.z;
            let largest = value.x.abs().max(value.y.abs()).max(value.z.abs());
            if largest == 0. {
                return Ok(None);
            }
            let direction = (value / largest).normalized();
            if !direction.is_finite() || direction.length() == 0. {
                return Err("instance normal transform produces an invalid direction".into());
            }
            output[index] = direction;
        }
        Ok(Some(output))
    }
}

/// Reverse the two trailing corners of every authored per-corner attribute.
fn reverse_corners(appearance: &mut TriangleAppearance) {
    if let Some(colors) = &mut appearance.colors {
        colors.swap(1, 2);
    }
    if let Some(texcoords) = &mut appearance.texcoords {
        texcoords.swap(1, 2);
    }
    if let Some(normals) = &mut appearance.normals {
        normals.swap(1, 2);
    }
}

/// Validate one face appearance with the same values and decoded image budgets as `Scene`.
fn validate_appearance(
    appearance: &TriangleAppearance,
    textures: &mut TextureBudget,
) -> Result<()> {
    let valid_color = |color: &[f64; 4]| {
        color
            .iter()
            .all(|value| value.is_finite() && (0.0..=1.).contains(value))
    };
    if !valid_color(&appearance.base_color)
        || appearance
            .colors
            .is_some_and(|colors| colors.iter().any(|color| !valid_color(color)))
    {
        return Err("indexed appearance colors must be finite values between zero and one".into());
    }
    if appearance
        .texcoords
        .is_some_and(|uvs| uvs.iter().flatten().any(|value| !value.is_finite()))
    {
        return Err("indexed texture coordinates must be finite".into());
    }
    if appearance.normals.is_some_and(|normals| {
        normals.iter().any(|normal| {
            !normal.is_finite() || !normal.length().is_finite() || normal.length() == 0.
        })
    }) {
        return Err("indexed appearance normals must be finite nonzero directions".into());
    }
    if let AlphaMode::Mask { cutoff } = appearance.alpha_mode
        && (!cutoff.is_finite() || cutoff < 0.)
    {
        return Err("indexed alpha cutoff must be finite and nonnegative".into());
    }
    if let Some(texture) = &appearance.texture {
        if appearance.texcoords.is_none() {
            return Err("indexed color texture requires texture coordinates".into());
        }
        let width = texture.image.width();
        let height = texture.image.height();
        let pixels = u64::from(width) * u64::from(height);
        if width == 0 || height == 0 || width > 8192 || height > 8192 || pixels > 16 * 1024 * 1024 {
            return Err("indexed texture exceeds the dimension or 16 million pixel limit".into());
        }
        if textures.images.insert(Arc::as_ptr(&texture.image)) {
            textures.pixels = textures
                .pixels
                .checked_add(pixels)
                .filter(|&total| total <= 64 * 1024 * 1024)
                .ok_or("instanced color textures exceed the 64 million pixel limit")?;
        }
    }
    Ok(())
}
