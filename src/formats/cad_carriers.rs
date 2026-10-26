//! Validate native BREP carrier admission and exact planar boundaries.
//!
//! The upstream reader materializes source carrier placements and binds
//! parameter curves to face occurrences; preview admission remains bounded.
use crate::Result;
use ogeom_core::Tolerances;
use ogeom_geom::{Curve, Curve3d, SurfaceGeometry, Transformable};
use ogeom_math::{Matrix3, Plane, Transform, Vector};
use ogeom_topo::{EdgeRepr, Filter, Model, NodeData, Shape, ShapeType, SurfaceId, explore};
use std::collections::HashMap;

/// Existing preview work ceiling, also enforced before upstream import expands references.
const MAX_IMPORT_WORK: usize = 10_000_000;
/// Charge repeated placement copying and hashing along each imported occurrence path.
const CHAIN_TERM_WORK: usize = 8;

/// Cumulative copying and traversal work admitted before calling the kernel reader.
#[derive(Default)]
struct ImportBudget {
    /// Conservatively counted placement copies and topology visits.
    work: usize,
}

impl ImportBudget {
    /// Charge expansion work before upstream can allocate or walk it.
    ///
    /// # Errors
    /// The preview's cumulative work ceiling is exceeded.
    fn charge(&mut self, units: usize) -> Result<()> {
        self.work = self.work.saturating_add(units);
        if self.work > MAX_IMPORT_WORK {
            return Err("BREP import expansion exceeds work limit".into());
        }
        Ok(())
    }
}

/// Bounded occurrence counts and placement-copy work for one native topology record.
#[derive(Clone, Copy)]
struct Expansion {
    /// Expanded visits, retaining repeated child occurrences rather than deduplicating them.
    occurrences: usize,
    /// Maximum number of parent/child links below this record.
    depth: usize,
    /// Visits plus the sum of relative placement-chain lengths over those visits.
    work: usize,
    /// Longest relative placement chain below this record.
    chain: usize,
}

/// Read a bounded native integer without trusting table offsets or counts.
///
/// # Errors
/// The token is missing or is not an unsigned native integer.
fn count_at(tokens: &[&str], at: usize) -> Result<usize> {
    Ok(tokens
        .get(at)
        .ok_or("truncated BREP import preflight")?
        .parse()?)
}

/// Find the first token of a required native table.
///
/// # Errors
/// The requested table is missing.
fn table_at(tokens: &[&str], name: &str) -> Result<usize> {
    tokens
        .iter()
        .position(|token| *token == name)
        .ok_or_else(|| format!("BREP lacks {name} table").into())
}

/// Bound location-chain expansion without constructing the chains themselves.
///
/// # Errors
/// References, powers or records are malformed, or expanded storage/copying exceeds preview limits.
fn location_expansion(tokens: &[&str], budget: &mut ImportBudget) -> Result<Vec<usize>> {
    let table = table_at(tokens, "Locations")?;
    let count = count_at(tokens, table + 1)?;
    if count > super::MAX_TOPOLOGY {
        return Err("too many BREP carrier locations".into());
    }
    let mut cursor = table + 2;
    let mut lengths = vec![0usize];
    let mut retained = 0usize;
    for _ in 0..count {
        let kind = count_at(tokens, cursor)?;
        cursor += 1;
        budget.charge(1)?;
        let mut length = 0usize;
        match kind {
            1 => {
                let mut identity = true;
                for expected in [1.0_f64, 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0.] {
                    let value = tokens
                        .get(cursor)
                        .ok_or("truncated BREP carrier transform")?
                        .parse::<f64>()?;
                    cursor += 1;
                    if !value.is_finite() {
                        return Err("non-finite BREP carrier transform".into());
                    }
                    // Treat either signed zero as exact identity, without
                    // rounding a small but genuinely nonzero placement away.
                    let bits = if value == 0. {
                        0.0_f64.to_bits()
                    } else {
                        value.to_bits()
                    };
                    identity &= bits == expected.to_bits();
                }
                length = usize::from(!identity);
            }
            2 => {
                let mut links = 0usize;
                loop {
                    let index = count_at(tokens, cursor)?;
                    cursor += 1;
                    if index == 0 {
                        break;
                    }
                    let power = tokens
                        .get(cursor)
                        .ok_or("truncated BREP carrier power")?
                        .parse::<i64>()?;
                    cursor += 1;
                    links += 1;
                    if index >= lengths.len() || links > 64 || power.unsigned_abs() > 64 {
                        return Err("BREP carrier composition exceeds limits".into());
                    }
                    let factor = lengths[index];
                    // The upstream reader clones the referenced chain before
                    // examining its power, including when that power is zero.
                    budget.charge(factor)?;
                    if power < 0 {
                        // Inverting that cloned chain allocates another copy.
                        budget.charge(factor)?;
                    }
                    for _ in 0..power.unsigned_abs() {
                        // Ignore adjacent-datum cancellation: this is a conservative
                        // bound, not another implementation of kernel placement algebra.
                        length = length.saturating_add(factor);
                        if length > super::MAX_TOPOLOGY {
                            return Err("BREP location expansion exceeds 100000 terms".into());
                        }
                        budget.charge(length.saturating_add(factor).saturating_add(1))?;
                    }
                }
            }
            _ => return Err("unsupported BREP carrier transform record".into()),
        }
        retained = retained.saturating_add(length);
        if retained > super::MAX_TOPOLOGY {
            return Err("BREP location expansion exceeds 100000 retained terms".into());
        }
        lengths.push(length);
    }
    Ok(lengths)
}

/// Parse an oriented backward shape reference, or identify the preceding flag word.
///
/// # Errors
/// An oriented reference contains a malformed integer.
fn reference_number(token: &str) -> Result<Option<usize>> {
    match token.as_bytes().first() {
        Some(b'+' | b'-' | b'i' | b'e') => Ok(Some(token[1..].parse()?)),
        _ => Ok(None),
    }
}

/// Bound native topology expansion, including chain copies for repeated occurrences.
///
/// # Errors
/// Shape records/references are malformed, point forward or exceed occurrence, depth or work ceilings.
fn topology_expansion(
    tokens: &[&str],
    locations: &[usize],
    budget: &mut ImportBudget,
) -> Result<()> {
    let table = table_at(tokens, "TShapes")?;
    let count = count_at(tokens, table + 1)?;
    if count > super::MAX_TOPOLOGY {
        return Err("too many BREP topology records".into());
    }
    let mut cursor = table + 2;
    let mut records: Vec<Expansion> = Vec::with_capacity(count);
    for _ in 0..count {
        if !matches!(
            tokens.get(cursor),
            Some(&("Ve" | "Ed" | "Wi" | "Fa" | "Sh" | "So" | "CS" | "Co"))
        ) {
            return Err("unsupported BREP topology record during preflight".into());
        }
        let end = tokens[cursor..]
            .iter()
            .position(|token| *token == "*")
            .map(|at| cursor + at)
            .ok_or("unterminated BREP topology record")?;
        let mut tail = end;
        let mut expansion = Expansion {
            occurrences: 1,
            depth: 0,
            work: 1,
            chain: 0,
        };
        while tail >= cursor + 3 {
            let Some(number) = reference_number(tokens[tail - 2])? else {
                break;
            };
            let child = count
                .checked_sub(number)
                .and_then(|at| records.get(at))
                .ok_or("BREP topology references missing or forward records")?;
            let location = *locations
                .get(count_at(tokens, tail - 1)?)
                .ok_or("BREP topology references missing location")?;
            expansion.occurrences = expansion.occurrences.saturating_add(child.occurrences);
            expansion.depth = expansion.depth.max(child.depth + 1);
            expansion.chain = expansion.chain.max(child.chain.saturating_add(location));
            expansion.work = expansion.work.saturating_add(child.work).saturating_add(
                child
                    .occurrences
                    .saturating_mul(location)
                    .saturating_mul(CHAIN_TERM_WORK),
            );
            if expansion.occurrences > super::MAX_TOPOLOGY || expansion.depth > 64 {
                return Err(
                    "BREP topology expansion exceeds 100000 occurrences or 64 levels".into(),
                );
            }
            if expansion.chain > super::MAX_TOPOLOGY || expansion.work > MAX_IMPORT_WORK {
                return Err("BREP import expansion exceeds work limit".into());
            }
            tail -= 2;
        }
        let flags = tokens
            .get(tail.saturating_sub(1))
            .ok_or("missing BREP topology flags")?;
        if tail <= cursor + 1
            || flags.len() != 7
            || !flags.bytes().all(|bit| matches!(bit, b'0' | b'1'))
        {
            return Err("malformed BREP topology flags or child references".into());
        }
        // Bound all native records, including faces rebuilt while importing,
        // rather than only the root that the final numbering pass traverses.
        budget.charge(expansion.work)?;
        records.push(expansion);
        cursor = end + 1;
    }
    let number = reference_number(tokens.get(cursor).ok_or("missing BREP root reference")?)?
        .ok_or("malformed BREP root reference")?;
    let root = count
        .checked_sub(number)
        .and_then(|at| records.get(at))
        .ok_or("BREP root references missing topology")?;
    let location = *locations
        .get(count_at(tokens, cursor + 1)?)
        .ok_or("BREP root references missing location")?;
    if root.chain.saturating_add(location) > super::MAX_TOPOLOGY {
        return Err("BREP root placement expansion exceeds 100000 terms".into());
    }
    budget.charge(
        root.work.saturating_add(
            root.occurrences
                .saturating_mul(location)
                .saturating_mul(CHAIN_TERM_WORK),
        ),
    )
}

/// Bound native placement and topology amplification before the kernel reader expands either.
///
/// # Errors
/// Native references are malformed or expanded storage, occurrences, depth or cumulative work is excessive.
pub(super) fn preflight_expansion(tokens: &[&str]) -> Result<()> {
    let mut budget = ImportBudget::default();
    let locations = location_expansion(tokens, &mut budget)?;
    topology_expansion(tokens, &locations, &mut budget)
}

/// Native topology kind and carrier-placement references for one BREP record.
struct Record {
    /// Native schema or topology kind selecting the record interpretation.
    kind: ShapeType,
    /// Representation-kind tags paired with native carrier-location indices.
    placements: Vec<(u8, usize)>,
}

/// Native carrier transforms and record associations needed to restore imported BREP geometry.
pub(super) struct Metadata {
    /// Decoded native carrier transforms; index zero is identity.
    transforms: Vec<Transform>,
    /// Native topology/carrier records in authored import order.
    records: Vec<Record>,
}

impl Metadata {
    /// Read native carrier placements and topology-record associations.
    ///
    /// # Errors
    ///
    /// Missing tables, invalid locations, scaled carriers or malformed representation references are rejected.
    #[allow(
        clippy::too_many_lines,
        reason = "Keep the format record dispatch, validation and its resource checks together without changing geometry evaluation."
    )]
    pub(super) fn parse(text: &str) -> Result<Self> {
        let tokens: Vec<_> = text.split_whitespace().collect();
        preflight_expansion(&tokens)?;
        let number = |at: usize| -> Result<usize> {
            Ok(tokens
                .get(at)
                .ok_or("truncated BREP carrier metadata")?
                .parse()?)
        };
        let table = |name: &str| -> Result<usize> {
            tokens
                .iter()
                .position(|word| *word == name)
                .ok_or_else(|| format!("BREP lacks {name} table").into())
        };
        let location_table = table("Locations")?;
        let location_count = number(location_table + 1)?;
        if location_count > super::MAX_TOPOLOGY {
            return Err("too many BREP carrier locations".into());
        }
        let mut cursor = location_table + 2;
        let mut transforms = vec![Transform::IDENTITY];
        for _ in 0..location_count {
            let kind = number(cursor)?;
            cursor += 1;
            let transform = match kind {
                1 => {
                    let mut rows = [[0.; 3]; 3];
                    let mut translation = [0.; 3];
                    for (row, offset) in rows.iter_mut().zip(&mut translation) {
                        for value in row.iter_mut().chain(std::iter::once(offset)) {
                            *value = tokens
                                .get(cursor)
                                .ok_or("truncated BREP carrier transform")?
                                .parse::<f64>()?;
                            cursor += 1;
                            if !value.is_finite() {
                                return Err("non-finite BREP carrier transform".into());
                            }
                        }
                    }
                    let scale = Matrix3::new(rows).determinant().cbrt();
                    if !scale.is_finite() || scale == 0. {
                        return Err("singular BREP carrier transform".into());
                    }
                    let normalized =
                        std::array::from_fn(|r| std::array::from_fn(|c| rows[r][c] / scale));
                    Transform::from_parts(
                        Matrix3::new(normalized),
                        scale,
                        Vector::new(translation[0], translation[1], translation[2]),
                        1e-9,
                    )?
                }
                2 => {
                    let mut composed = Transform::IDENTITY;
                    let mut links = 0;
                    loop {
                        let index = number(cursor)?;
                        cursor += 1;
                        if index == 0 {
                            break;
                        }
                        links += 1;
                        let power = tokens
                            .get(cursor)
                            .ok_or("truncated BREP carrier power")?
                            .parse::<i64>()?;
                        cursor += 1;
                        if index >= transforms.len() || links > 64 || power.unsigned_abs() > 64 {
                            return Err("BREP carrier composition exceeds limits".into());
                        }
                        let base = transforms[index];
                        let step = if power < 0 { base.inverse()? } else { base };
                        for _ in 0..power.unsigned_abs() {
                            composed = composed * step;
                        }
                    }
                    composed
                }
                _ => return Err("unsupported BREP carrier transform record".into()),
            };
            transforms.push(transform);
        }
        let curves = number(table("Curves")? + 1)?;
        let pcurves = number(table("Curve2ds")? + 1)?;
        let surfaces = number(table("Surfaces")? + 1)?;
        let geometry_id = |at: usize, total: usize| -> Result<()> {
            let index = number(at)?;
            if index == 0 || index > total {
                return Err("BREP carrier references missing geometry".into());
            }
            Ok(())
        };
        let carrier = |at: usize| -> Result<usize> {
            let index = number(at)?;
            let transform = transforms
                .get(index)
                .ok_or("BREP carrier references missing location")?;
            if (transform.scale_factor().abs() - 1.).abs() > 1e-9 {
                return Err("scaled BREP carrier locations are unsupported; occurrence scale remains supported".into());
            }
            Ok(index)
        };
        let topology = table("TShapes")?;
        let count = number(topology + 1)?;
        if count > super::MAX_TOPOLOGY {
            return Err("too many BREP topology records".into());
        }
        let mut records = Vec::with_capacity(count);
        for (at, word) in tokens.iter().enumerate().skip(topology + 2) {
            let kind = match *word {
                "Ve" => ShapeType::Vertex,
                "Ed" => ShapeType::Edge,
                "Wi" => ShapeType::Wire,
                "Fa" => ShapeType::Face,
                "Sh" => ShapeType::Shell,
                "So" => ShapeType::Solid,
                "CS" => ShapeType::CompSolid,
                "Co" => ShapeType::Compound,
                _ => continue,
            };
            let mut placements = Vec::new();
            if kind == ShapeType::Face {
                geometry_id(at + 3, surfaces)?;
                placements.push((0, carrier(at + 4)?));
            } else if kind == ShapeType::Edge {
                let mut cursor = at + 5;
                loop {
                    let repr = number(cursor)?;
                    cursor += 1;
                    let fields = match repr {
                        0 => break,
                        1 => {
                            geometry_id(cursor, curves)?;
                            placements.push((1, carrier(cursor + 1)?));
                            4
                        }
                        2 => {
                            geometry_id(cursor, pcurves)?;
                            geometry_id(cursor + 1, surfaces)?;
                            placements.push((2, carrier(cursor + 2)?));
                            5
                        }
                        3 => {
                            geometry_id(cursor, pcurves)?;
                            geometry_id(cursor + 1, pcurves)?;
                            geometry_id(cursor + 3, surfaces)?;
                            placements.push((3, carrier(cursor + 4)?));
                            7
                        }
                        // Continuity and cached mesh metadata do not produce
                        // typed curve/surface representations in the reader.
                        4 => 5,
                        5 => 2,
                        6 => 3,
                        7 => 4,
                        _ => return Err("unsupported BREP edge carrier representation".into()),
                    };
                    cursor = cursor
                        .checked_add(fields)
                        .ok_or("BREP carrier record overflow")?;
                    if cursor > tokens.len() {
                        return Err("truncated BREP carrier record".into());
                    }
                }
            }
            records.push(Record { kind, placements });
        }
        if records.len() != count {
            return Err("BREP carrier metadata disagrees with topology count".into());
        }
        Ok(Self {
            transforms,
            records,
        })
    }

    /// Validate the upstream-completed carriers and exact planar boundaries.
    ///
    /// # Errors
    /// Imported topology loses native records, exceeds preview work/geometry limits,
    /// or a planar edge cannot be proved to lie on its face.
    pub(super) fn restore(
        &self,
        model: &mut Model,
        root: &Shape,
        tolerances: Tolerances,
    ) -> Result<()> {
        // Ogeom now applies carrier transforms and can add placed geometry and
        // rebuilt wires/faces. Reapplying the native matrices would move those
        // carriers twice; matching exact insertion ordinals is no longer valid.
        let mut admitted = HashMap::new();
        let mut admitted_representations = 0usize;
        for record in &self.records {
            *admitted.entry(record.kind).or_insert(0usize) += 1;
            if record.kind == ShapeType::Edge {
                admitted_representations =
                    admitted_representations.saturating_add(record.placements.len());
            }
        }
        let mut imported = HashMap::new();
        let mut imported_representations = 0usize;
        for (_, node) in model.nodes() {
            *imported.entry(node.kind()).or_insert(0usize) += 1;
            if let NodeData::Edge(edge) = node.data() {
                imported_representations =
                    imported_representations.saturating_add(edge.representations.len());
            }
        }
        if admitted
            .iter()
            .any(|(kind, count)| imported.get(kind).copied().unwrap_or(0) < *count)
        {
            return Err("BREP import lost native topology records".into());
        }
        // Count charts synthesized by the reader against the same output
        // ceiling previously enforced while attaching them in this adapter.
        if imported_representations.saturating_sub(admitted_representations) > super::MAX_TOPOLOGY {
            return Err("BREP exceeds 100000 placed pcurve representations".into());
        }
        let mut work = self.transforms.len();
        for record in &self.records {
            work = work.saturating_add(record.placements.len());
        }
        let mut attached = 0usize;
        for face in explore(model, root, Filter::OfType(ShapeType::Face))? {
            let surface = model
                .node(&face)
                .and_then(|n| n.data().as_face())
                .ok_or("missing BREP face")?
                .surface;
            for edge in explore(model, &face, Filter::OfType(ShapeType::Edge))? {
                let count = model
                    .node(&edge)
                    .and_then(|n| n.data().as_edge())
                    .ok_or("missing BREP edge")?
                    .representations
                    .len();
                work = work.saturating_add(count);
                if work > MAX_IMPORT_WORK {
                    return Err("BREP pcurve validation exceeds work limit".into());
                }
                if restore_planar_pcurve(model, &face, &edge, surface, tolerances)? {
                    attached += 1;
                    if attached > super::MAX_TOPOLOGY {
                        return Err("BREP exceeds 100000 placed pcurve representations".into());
                    }
                }
            }
        }
        Ok(())
    }
}

// OCCT intentionally omits pcurves on planes and derives them lazily.
// The exact projection must preserve the original edge parameterization;
// proving coplanarity avoids flattening a malformed off-surface boundary.
/// Prove exact coplanarity and derive an omitted planar pcurve when needed.
///
/// # Errors
///
/// Required geometry is missing, relative scale is unsupported or the boundary cannot be projected exactly.
fn restore_planar_pcurve(
    model: &mut Model,
    face: &Shape,
    edge: &Shape,
    surface: SurfaceId,
    tolerances: Tolerances,
) -> Result<bool> {
    let data = model
        .node(edge)
        .and_then(|n| n.data().as_edge())
        .ok_or("missing BREP edge")?;
    let has_pcurve = data.pcurve_for(surface, edge.location()).is_some();
    let Some(SurfaceGeometry::Plane(plane)) = model.geometry().surface(surface) else {
        return Ok(false);
    };
    let plane = plane.plane();
    let Some(EdgeRepr::Curve3d { curve, range, .. }) = data.curve3d() else {
        return Ok(false);
    };
    let range = *range;
    let tolerance = data.tolerance.get().max(tolerances.confusion());
    let placement = face.transform(model.datums())?.inverse()? * edge.transform(model.datums())?;
    if (placement.scale_factor().abs() - 1.).abs() > 1e-9 {
        return Err(
            "scaled relative edge placement cannot preserve omitted planar pcurve parameters"
                .into(),
        );
    }
    let curve = model
        .geometry()
        .curve(*curve)
        .ok_or("missing BREP edge curve")?
        .transformed(&placement, tolerances)?;
    if !lies_in_plane(&curve, range, plane, tolerance, 0) {
        return Err("BREP omitted planar pcurve has an off-plane or unsupported boundary".into());
    }
    // The reader may already have synthesized a projected planar chart. Keep
    // the exact coplanarity proof even then, so off-plane input is refused.
    if has_pcurve {
        return Ok(false);
    }
    let geometry = model
        .geometry()
        .surface(surface)
        .ok_or("missing BREP face surface")?;
    let pcurve = ogeom_intersect::exact_pcurve_over(&curve, range, geometry, tolerances)
        .ok_or("BREP omitted planar pcurve cannot be derived exactly")?;
    let curve = model.geometry_mut().add_pcurve(pcurve);
    let NodeData::Edge(data) = model.node_mut(edge).ok_or("missing BREP edge")?.data_mut() else {
        return Err("missing BREP edge data".into());
    };
    data.representations.push(EdgeRepr::PCurve {
        curve,
        surface,
        location: edge.location().clone(),
        range,
    });
    Ok(true)
}

/// Prove supported curve bounds lie in a plane within the supplied tolerance.
fn lies_in_plane(
    curve: &Curve,
    range: (f64, f64),
    plane: Plane,
    tolerance: f64,
    depth: usize,
) -> bool {
    if depth > 64 {
        return false;
    }
    let frame = plane.frame();
    let distance = |point| frame.to_local(point).z.abs();
    let normal = frame.z().vector();
    match curve {
        Curve::Line(line) => [range.0, range.1].into_iter().all(|t| {
            line.point_at(t, super::TOLERANCES)
                .is_ok_and(|p| distance(p) <= tolerance)
        }),
        Curve::Circle(circle) => {
            let c = circle.circle();
            distance(c.centre())
                + c.radius()
                    * (normal.dot(c.frame().x().vector()).abs()
                        + normal.dot(c.frame().y().vector()).abs())
                <= tolerance
        }
        Curve::Ellipse(ellipse) => {
            let e = ellipse.ellipse();
            distance(e.centre())
                + e.major_radius() * normal.dot(e.frame().x().vector()).abs()
                + e.minor_radius() * normal.dot(e.frame().y().vector()).abs()
                <= tolerance
        }
        Curve::BSpline(spline) => spline
            .control_points()
            .iter()
            .all(|p| p.weight > 0. && distance(p.point()) <= tolerance),
        Curve::Trimmed(trim) => lies_in_plane(trim.basis(), range, plane, tolerance, depth + 1),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as _;

    /// Construct only native table syntax; amplification tests never invoke the kernel.
    fn document(locations: &str, location_count: usize, records: &str, count: usize) -> String {
        format!(
            "DBRep_DrawableShape\nCASCADE Topology V1\nLocations {location_count}\n{locations}\
             Curve2ds 0\nCurves 0\nPolygon3D 0\nPolygonOnTriangulations 0\n\
             Surfaces 0\nTriangulations 0\nTShapes {count}\n{records}+1 0\n"
        )
    }

    /// A small finite vertex record usable below shared compounds.
    const VERTEX: &str = "Ve\n0.0000001 0 0 0 0 0\n0001000\n*\n";
    /// Two distinct rigid datums followed by an alternating two-term composition.
    const LOCATIONS: &str = "1 1 0 0 1 0 1 0 0 0 0 1 0\n1 1 0 0 0 0 1 0 1 0 0 1 0\n2 1 1 2 1 0\n";

    /// Evaluate native counts without creating geometry, location chains or expanded occurrences.
    fn preflight(text: &str) -> Result<()> {
        preflight_expansion(&text.split_whitespace().collect::<Vec<_>>())
    }

    #[test]
    fn preflight_rejects_location_chain_amplification_before_import() {
        let small = format!("{LOCATIONS}2 3 64 0\n");
        assert!(preflight(&document(&small, 4, VERTEX, 1)).is_ok());
        let amplified = format!("{small}2 4 64 0\n2 5 64 0\n");
        let error = preflight(&document(&amplified, 6, VERTEX, 1))
            .unwrap_err()
            .to_string();
        assert!(error.contains("location expansion"), "{error}");
    }

    #[test]
    fn preflight_charges_zero_power_factor_clones() {
        let locations = format!("{LOCATIONS}2 3 64 0\n2 4 64 0\n");
        let zero_power = format!("2 {}0\n", "5 0 ".repeat(64));
        let small = format!("{locations}{zero_power}");
        assert!(preflight(&document(&small, 6, VERTEX, 1)).is_ok());
        // Every added location is identity and retains no chain terms. The
        // reader still clones the 8192-term factor for each zero-power link.
        let excessive = format!("{locations}{}", zero_power.repeat(32));
        let error = preflight(&document(&excessive, 37, VERTEX, 1))
            .unwrap_err()
            .to_string();
        assert!(error.contains("work limit"), "{error}");
    }

    /// Build a native shared-child DAG using backward record numbering.
    fn compounds(count: usize, copies: usize) -> String {
        let mut records = String::from(VERTEX);
        for position in 1..count {
            records.push_str("Co\n0001000\n");
            for _ in 0..copies {
                let _ = write!(records, "+{} 0 ", count - position + 1);
            }
            records.push_str("*\n");
        }
        records
    }

    #[test]
    fn preflight_rejects_exponential_occurrences_and_overdeep_topology() {
        let error = preflight(&document("", 0, &compounds(20, 2), 20))
            .unwrap_err()
            .to_string();
        assert!(error.contains("occurrences"), "{error}");
        assert!(preflight(&document("", 0, &compounds(65, 1), 65)).is_ok());
        let error = preflight(&document("", 0, &compounds(66, 1), 66))
            .unwrap_err()
            .to_string();
        assert!(error.contains("64 levels"), "{error}");
    }

    #[test]
    fn preflight_bounds_combined_chain_and_occurrence_work() {
        let locations = format!("{LOCATIONS}2 3 64 0\n2 4 64 0\n");
        let small = format!("{VERTEX}Co\n0001000\n+2 5 *\n");
        assert!(preflight(&document(&locations, 5, &small, 2)).is_ok());
        // Both independent caps are respected: 1001 occurrences, depth one,
        // and an 8192-term location chain. Their repeated copying is excessive.
        let repeated = format!("{VERTEX}Co\n0001000\n{}*\n", "+2 5 ".repeat(1000));
        let error = preflight(&document(&locations, 5, &repeated, 2))
            .unwrap_err()
            .to_string();
        assert!(error.contains("work limit"), "{error}");
        // A root placement is inherited by every descendant even when each
        // individual child reference uses identity.
        let inherited = format!("{VERTEX}Co\n0001000\n{}*\n", "+2 0 ".repeat(1000));
        let inherited = document(&locations, 5, &inherited, 2).replace("+1 0\n", "+1 5\n");
        let error = preflight(&inherited).unwrap_err().to_string();
        assert!(error.contains("work limit"), "{error}");
    }
}
