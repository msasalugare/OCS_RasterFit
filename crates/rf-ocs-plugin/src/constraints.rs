use std::collections::HashMap;

use ocs_plugin_api::host::acadrust::objects::{
    Assoc2dConstraintGroup, AssocConstraintNode, AssocConstraintNodeData, AssocEvalValue,
    AssociativeData, AssociativeObject, ObjectType,
};
use ocs_plugin_api::host::acadrust::types::Vector3;
use ocs_plugin_api::host::{
    Document, EntityType, Handle, HostApi, PluginConstraint, PluginConstraintPick,
};
use rf_core::sketch::{Constraint, LineId, PointId, Sketch};

use crate::native::NativeSelection;

#[derive(Debug, Default)]
pub(crate) struct ConstraintRead {
    pub constraints: Vec<Constraint>,
    pub reference_constraints: Vec<usize>,
    pub mapped: usize,
    pub skipped: usize,
}

impl ConstraintRead {
    pub(crate) fn apply_to_sketch(&self, sketch: &mut Sketch) {
        let offset = sketch.constraints.len();
        sketch.constraints.extend(self.constraints.iter().cloned());
        sketch.reference_constraints.extend(
            self.reference_constraints
                .iter()
                .filter_map(|index| index.checked_add(offset)),
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrimitiveKind {
    Line,
    Circle,
    Arc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Primitive {
    kind: PrimitiveKind,
    index: usize,
}

#[derive(Debug, Clone, Copy)]
struct Reference {
    primitive: Primitive,
    point: Option<PointId>,
}

impl Reference {
    fn whole(self) -> Option<Primitive> {
        self.point.is_none().then_some(self.primitive)
    }

    fn point(self) -> Option<PointId> {
        self.point
    }
}

#[derive(Debug)]
struct Candidate {
    handle: Handle,
    primitive: Primitive,
    segment: Option<usize>,
    native_start: Vector3,
    native_end: Vector3,
    scale: Option<f64>,
}

struct GeometryMap<'a> {
    sketch: &'a Sketch,
    candidates: Vec<Candidate>,
}

impl<'a> GeometryMap<'a> {
    fn new(selection: &'a NativeSelection) -> Result<Self, String> {
        let sketch = &selection.sketch;
        let expected = sketch.lines.len() + sketch.circles.len() + sketch.arcs.len();
        if selection.primitive_origins.len() != expected {
            return Err(format!(
                "native selection has {} primitive origins for {} sketch primitives",
                selection.primitive_origins.len(),
                expected
            ));
        }
        let mut candidates = Vec::with_capacity(expected);
        let mut origin = 0;
        for (index, _) in sketch.lines.iter().enumerate() {
            candidates.push(Candidate {
                handle: selection.primitive_origins[origin],
                primitive: Primitive {
                    kind: PrimitiveKind::Line,
                    index,
                },
                segment: None,
                native_start: Vector3::ZERO,
                native_end: Vector3::ZERO,
                scale: None,
            });
            origin += 1;
        }
        for (index, _) in sketch.circles.iter().enumerate() {
            candidates.push(Candidate {
                handle: selection.primitive_origins[origin],
                primitive: Primitive {
                    kind: PrimitiveKind::Circle,
                    index,
                },
                segment: None,
                native_start: Vector3::ZERO,
                native_end: Vector3::ZERO,
                scale: None,
            });
            origin += 1;
        }
        for (index, _) in sketch.arcs.iter().enumerate() {
            candidates.push(Candidate {
                handle: selection.primitive_origins[origin],
                primitive: Primitive {
                    kind: PrimitiveKind::Arc,
                    index,
                },
                segment: None,
                native_start: Vector3::ZERO,
                native_end: Vector3::ZERO,
                scale: None,
            });
            origin += 1;
        }
        let mut map = Self { sketch, candidates };
        map.fill_native_geometry(&selection.originals);
        Ok(map)
    }

    fn contains_handle(&self, handle: Handle) -> bool {
        self.candidates
            .iter()
            .any(|candidate| candidate.handle == handle)
    }

    fn indices(&self, handle: Handle, kind: PrimitiveKind) -> Vec<usize> {
        self.candidates
            .iter()
            .enumerate()
            .filter_map(|(index, candidate)| {
                (candidate.handle == handle && candidate.primitive.kind == kind).then_some(index)
            })
            .collect()
    }

    fn fill_native_geometry(&mut self, originals: &[EntityType]) {
        for entity in originals {
            let handle = entity.common().handle;
            match entity {
                EntityType::Line(line) => {
                    if let [index] = self.indices(handle, PrimitiveKind::Line).as_slice() {
                        let index = *index;
                        let native_length = (line.end - line.start).length();
                        let scale = self.line_scale(native_length, index);
                        let candidate = &mut self.candidates[index];
                        candidate.native_start = line.start;
                        candidate.native_end = line.end;
                        candidate.scale = scale;
                    }
                }
                EntityType::Circle(circle) => {
                    if let [index] = self.indices(handle, PrimitiveKind::Circle).as_slice() {
                        let scale = self.circle_scale(circle.radius, *index);
                        self.candidates[*index].scale = scale;
                    }
                }
                EntityType::Arc(arc) => {
                    if let [index] = self.indices(handle, PrimitiveKind::Arc).as_slice() {
                        let scale = self.arc_scale(arc.radius, *index);
                        self.candidates[*index].scale = scale;
                    }
                }
                EntityType::LwPolyline(polyline) => {
                    self.fill_polyline(handle, polyline);
                }
                _ => {}
            }
        }
    }

    fn fill_polyline(
        &mut self,
        handle: Handle,
        polyline: &ocs_plugin_api::host::acadrust::entities::LwPolyline,
    ) {
        let vertex_count = if polyline.is_closed
            && polyline.vertices.len() > 1
            && polyline.vertices.first().map(|vertex| vertex.location)
                == polyline.vertices.last().map(|vertex| vertex.location)
        {
            polyline.vertices.len() - 1
        } else {
            polyline.vertices.len()
        };
        if vertex_count < 2 {
            return;
        }
        let segment_count = if polyline.is_closed {
            vertex_count
        } else {
            vertex_count - 1
        };
        let line_indices = self.indices(handle, PrimitiveKind::Line);
        let arc_indices = self.indices(handle, PrimitiveKind::Arc);
        let mut line_index = 0;
        let mut arc_index = 0;
        for segment in 0..segment_count {
            let next = (segment + 1) % vertex_count;
            let start = polyline.vertices[segment].location;
            let end = polyline.vertices[next].location;
            let start = Vector3::new(start.x, start.y, 0.0);
            let end = Vector3::new(end.x, end.y, 0.0);
            let bulge = polyline.vertices[segment].bulge;
            let (occurrence, scale) = if bulge == 0.0 {
                let native_length = (end - start).length();
                let candidate_index = line_indices.get(line_index).copied();
                let scale = candidate_index.and_then(|index| self.line_scale(native_length, index));
                line_index += 1;
                (candidate_index, scale)
            } else {
                let candidate_index = arc_indices.get(arc_index).copied();
                let chord = (end - start).length();
                let radius = chord * (1.0 + bulge * bulge) / (4.0 * bulge.abs());
                let scale = candidate_index.and_then(|index| self.arc_scale(radius, index));
                arc_index += 1;
                (candidate_index, scale)
            };
            if let Some(index) = occurrence {
                let candidate = &mut self.candidates[index];
                candidate.segment = Some(segment);
                candidate.native_start = start;
                candidate.native_end = end;
                candidate.scale = scale;
            }
        }
    }

    fn line_scale(&self, native_length: f64, candidate_index: usize) -> Option<f64> {
        let primitive = self.candidates.get(candidate_index)?.primitive;
        let line = self.sketch.lines.get(primitive.index)?;
        let a = self.point(line.a)?;
        let b = self.point(line.b)?;
        let raster_length = (b.x - a.x).hypot(b.y - a.y);
        positive_ratio(native_length, raster_length)
    }

    fn circle_scale(&self, native_radius: f64, candidate_index: usize) -> Option<f64> {
        let primitive = self.candidates.get(candidate_index)?.primitive;
        let circle = self.sketch.circles.get(primitive.index)?;
        positive_ratio(native_radius, circle.radius)
    }

    fn arc_scale(&self, native_radius: f64, candidate_index: usize) -> Option<f64> {
        let primitive = self.candidates.get(candidate_index)?.primitive;
        let arc = self.sketch.arcs.get(primitive.index)?;
        positive_ratio(native_radius, arc.radius)
    }

    fn point(&self, id: PointId) -> Option<rf_core::sketch::SketchPoint> {
        self.sketch
            .points
            .iter()
            .find(|point| point.id == id)
            .copied()
    }

    fn world_per_raster(&self) -> Option<f64> {
        let mut scales = self
            .candidates
            .iter()
            .filter_map(|candidate| candidate.scale);
        let first = scales.next()?;
        if scales
            .all(|scale| (scale - first).abs() <= 1.0e-7 * scale.abs().max(first.abs()).max(1.0))
        {
            Some(first)
        } else {
            None
        }
    }

    fn line(&self, primitive: Primitive) -> Option<&rf_core::sketch::SketchLine> {
        (primitive.kind == PrimitiveKind::Line)
            .then(|| self.sketch.lines.get(primitive.index))
            .flatten()
    }

    fn circle(&self, primitive: Primitive) -> Option<&rf_core::sketch::SketchCircle> {
        (primitive.kind == PrimitiveKind::Circle)
            .then(|| self.sketch.circles.get(primitive.index))
            .flatten()
    }

    fn arc(&self, primitive: Primitive) -> Option<&rf_core::sketch::SketchArc> {
        (primitive.kind == PrimitiveKind::Arc)
            .then(|| self.sketch.arcs.get(primitive.index))
            .flatten()
    }

    fn resolve_geometry(
        &self,
        document: &Document,
        node: &AssocConstraintNode,
        work_plane: &[Vector3; 3],
    ) -> NodeResolution {
        let Some(dependency) = geometry_dependency(&node.data) else {
            return NodeResolution::None;
        };
        let Some(ObjectType::Associative(AssociativeObject {
            data: AssociativeData::GeomDependency(dependency),
            ..
        })) = document.objects.get(&dependency)
        else {
            return NodeResolution::Invalid(false);
        };
        let handle = dependency.dependency.dependent_on;
        if !self.contains_handle(handle) {
            return NodeResolution::Foreign;
        }
        let Some(kind) = geometry_kind(&node.data) else {
            return NodeResolution::Invalid(true);
        };
        let indices = self.indices(handle, kind);
        let is_polyline = matches!(document.get_entity(handle), Some(EntityType::LwPolyline(_)));
        let resolved = if is_polyline {
            let Some((start, end)) = geometry_endpoints(&node.data) else {
                return NodeResolution::Invalid(true);
            };
            let start = work_plane_point(work_plane, start);
            let end = work_plane_point(work_plane, end);
            indices
                .into_iter()
                .filter(|index| self.candidates[*index].segment.is_some())
                .map(|index| {
                    let candidate = &self.candidates[index];
                    let forward = squared_distance(candidate.native_start, start)
                        + squared_distance(candidate.native_end, end);
                    let reverse = squared_distance(candidate.native_start, end)
                        + squared_distance(candidate.native_end, start);
                    (index, forward.min(reverse))
                })
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .filter(|(_, score)| {
                    score.is_finite() && *score <= 1.0e-12 * geometry_scale(start, end).powi(2)
                })
                .map(|(index, _)| index)
        } else {
            match indices.as_slice() {
                [index] => Some(*index),
                _ => None,
            }
        };
        resolved.map_or(NodeResolution::Invalid(true), |index| {
            NodeResolution::Resolved(Reference {
                primitive: self.candidates[index].primitive,
                point: None,
            })
        })
    }

    fn resolve_implicit_point(&self, curve: NodeResolution, point_type: u8) -> NodeResolution {
        let NodeResolution::Resolved(curve) = curve else {
            return curve;
        };
        let point = match curve.primitive.kind {
            PrimitiveKind::Line => match point_type {
                0 => self.line(curve.primitive).map(|line| line.a),
                1 => self.line(curve.primitive).map(|line| line.b),
                _ => None,
            },
            PrimitiveKind::Circle if point_type == 3 => {
                self.circle(curve.primitive).map(|circle| circle.center)
            }
            PrimitiveKind::Arc if point_type == 3 => {
                self.arc(curve.primitive).map(|arc| arc.center)
            }
            _ => None,
        };
        point.map_or(NodeResolution::Invalid(true), |point| {
            NodeResolution::Resolved(Reference {
                primitive: curve.primitive,
                point: Some(point),
            })
        })
    }

    fn fixed_point(&self, id: PointId) -> Option<Constraint> {
        let point = self.point(id)?;
        (point.x.is_finite() && point.y.is_finite())
            .then_some(Constraint::FixedPoint(id, point.x, point.y))
    }

    fn external_tangency(&self, first: Primitive, second: Primitive) -> bool {
        let Some((first_center, first_radius)) = self.center_radius(first) else {
            return false;
        };
        let Some((second_center, second_radius)) = self.center_radius(second) else {
            return false;
        };
        let distance = (second_center.x - first_center.x).hypot(second_center.y - first_center.y);
        let target = first_radius + second_radius;
        distance.is_finite()
            && target.is_finite()
            && (distance - target).abs() <= 1.0e-6 * distance.abs().max(target).max(1.0)
    }

    fn center_radius(&self, primitive: Primitive) -> Option<(rf_core::sketch::SketchPoint, f64)> {
        match primitive.kind {
            PrimitiveKind::Circle => {
                let circle = self.circle(primitive)?;
                Some((self.point(circle.center)?, circle.radius))
            }
            PrimitiveKind::Arc => {
                let arc = self.arc(primitive)?;
                Some((self.point(arc.center)?, arc.radius))
            }
            PrimitiveKind::Line => None,
        }
    }
}

fn positive_ratio(native: f64, raster: f64) -> Option<f64> {
    let ratio = native / raster;
    (native.is_finite() && raster.is_finite() && native > 0.0 && raster > 0.0 && ratio.is_finite())
        .then_some(ratio)
}

fn squared_distance(a: Vector3, b: Vector3) -> f64 {
    (a - b).length_squared()
}

fn geometry_scale(a: Vector3, b: Vector3) -> f64 {
    a.length().max(b.length()).max((a - b).length()).max(1.0)
}

fn work_plane_point(work_plane: &[Vector3; 3], local: Vector3) -> Vector3 {
    let [origin, axis_x, axis_y] = *work_plane;
    let normal = Vector3::new(
        axis_x.y * axis_y.z - axis_x.z * axis_y.y,
        axis_x.z * axis_y.x - axis_x.x * axis_y.z,
        axis_x.x * axis_y.y - axis_x.y * axis_y.x,
    );
    origin + axis_x * local.x + axis_y * local.y + normal * local.z
}

fn geometry_dependency(data: &AssocConstraintNodeData) -> Option<Handle> {
    match data {
        AssocConstraintNodeData::Point {
            geometry_dependency,
            ..
        }
        | AssocConstraintNodeData::Line {
            geometry_dependency,
            ..
        }
        | AssocConstraintNodeData::BoundedLine {
            geometry_dependency,
            ..
        }
        | AssocConstraintNodeData::Circle {
            geometry_dependency,
            ..
        }
        | AssocConstraintNodeData::Arc {
            geometry_dependency,
            ..
        }
        | AssocConstraintNodeData::Ellipse {
            geometry_dependency,
            ..
        }
        | AssocConstraintNodeData::BoundedEllipse {
            geometry_dependency,
            ..
        }
        | AssocConstraintNodeData::Spline {
            geometry_dependency,
            ..
        } if !geometry_dependency.is_null() => Some(*geometry_dependency),
        _ => None,
    }
}

fn geometry_kind(data: &AssocConstraintNodeData) -> Option<PrimitiveKind> {
    match data {
        AssocConstraintNodeData::BoundedLine { is_ray: false, .. } => Some(PrimitiveKind::Line),
        AssocConstraintNodeData::Circle { .. } => Some(PrimitiveKind::Circle),
        AssocConstraintNodeData::Arc { .. } => Some(PrimitiveKind::Arc),
        _ => None,
    }
}

fn geometry_endpoints(data: &AssocConstraintNodeData) -> Option<(Vector3, Vector3)> {
    match data {
        AssocConstraintNodeData::BoundedLine {
            start_point,
            end_point,
            is_ray: false,
            ..
        }
        | AssocConstraintNodeData::Arc {
            start_point,
            end_point,
            ..
        } => Some((*start_point, *end_point)),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy)]
enum NodeResolution {
    None,
    Resolved(Reference),
    Foreign,
    Invalid(bool),
}

fn resolve_nodes(
    document: &Document,
    group: &Assoc2dConstraintGroup,
    geometry: &GeometryMap<'_>,
) -> Vec<NodeResolution> {
    let mut resolutions = Vec::with_capacity(group.nodes.len());
    let mut indices = HashMap::new();
    for (index, node) in group.nodes.iter().enumerate() {
        indices.insert(node.node_id, index);
        resolutions.push(geometry.resolve_geometry(document, node, &group.work_plane));
    }
    for (index, node) in group.nodes.iter().enumerate() {
        let AssocConstraintNodeData::ImplicitPoint {
            point_type,
            curve_id,
            ..
        } = node.data
        else {
            continue;
        };
        let curve = indices
            .get(&curve_id)
            .and_then(|index| resolutions.get(*index))
            .copied()
            .unwrap_or(NodeResolution::Invalid(false));
        resolutions[index] = geometry.resolve_implicit_point(curve, point_type);
    }
    resolutions
}

fn constraint_status(data: &AssocConstraintNodeData) -> (i32, bool, bool) {
    match data {
        AssocConstraintNodeData::Geometrical {
            owner_id,
            is_implied,
            is_active,
        }
        | AssocConstraintNodeData::Composite {
            owner_id,
            is_implied,
            is_active,
            ..
        }
        | AssocConstraintNodeData::Angle {
            owner_id,
            is_implied,
            is_active,
            ..
        }
        | AssocConstraintNodeData::Parallel {
            owner_id,
            is_implied,
            is_active,
            ..
        }
        | AssocConstraintNodeData::Distance {
            owner_id,
            is_implied,
            is_active,
            ..
        }
        | AssocConstraintNodeData::RadiusDiameter {
            owner_id,
            is_implied,
            is_active,
            ..
        } => (*owner_id, *is_implied, *is_active),
        _ => (0, false, true),
    }
}

pub(crate) fn read_native_constraints(
    document: &Document,
    selection: &NativeSelection,
) -> Result<ConstraintRead, String> {
    let geometry = GeometryMap::new(selection)?;
    let scale = geometry.world_per_raster();
    let mut result = ConstraintRead::default();
    let mut groups: Vec<_> = document
        .objects
        .iter()
        .filter_map(|(handle, object)| {
            let ObjectType::Associative(AssociativeObject {
                data: AssociativeData::ConstraintGroup(group),
                ..
            }) = object
            else {
                return None;
            };
            Some((*handle, group))
        })
        .collect();
    groups.sort_by_key(|(handle, _)| handle.value());

    for (_, group) in groups {
        let resolutions = resolve_nodes(document, group, &geometry);
        let indices: HashMap<_, _> = group
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.node_id, index))
            .collect();
        for node in &group.nodes {
            if !node.class_name.to_ascii_uppercase().ends_with("CONSTRAINT") {
                continue;
            }
            let (owner, implied, active) = constraint_status(&node.data);
            if owner != 0 || implied {
                continue;
            }
            let mut references = Vec::new();
            let mut touches_selection = false;
            let mut invalid = false;
            let mut foreign = false;
            for connection in &node.connections {
                match indices
                    .get(connection)
                    .and_then(|index| resolutions.get(*index))
                    .copied()
                    .unwrap_or(NodeResolution::Invalid(false))
                {
                    NodeResolution::None => {}
                    NodeResolution::Resolved(reference) => {
                        touches_selection = true;
                        references.push(reference);
                    }
                    NodeResolution::Foreign => foreign = true,
                    NodeResolution::Invalid(selected) => {
                        touches_selection |= selected;
                        invalid = true;
                    }
                }
            }
            if !touches_selection {
                continue;
            }
            if invalid || foreign {
                result.skipped += 1;
                continue;
            }
            let Some(mapped) = map_constraint(document, group, node, &references, &geometry, scale)
            else {
                result.skipped += 1;
                continue;
            };
            result.mapped += 1;
            if active {
                result.constraints.extend(mapped);
            } else {
                let start = result.constraints.len();
                result.constraints.extend(mapped);
                result
                    .reference_constraints
                    .extend(start..result.constraints.len());
            }
        }
    }
    Ok(result)
}

fn map_constraint(
    document: &Document,
    group: &Assoc2dConstraintGroup,
    node: &AssocConstraintNode,
    references: &[Reference],
    geometry: &GeometryMap<'_>,
    scale: Option<f64>,
) -> Option<Vec<Constraint>> {
    let class = node.class_name.to_ascii_uppercase();
    match class.as_str() {
        "ACPOINTCOINCIDENCECONSTRAINT" => {
            let [first, second] = point_pair(references)?;
            Some(vec![Constraint::Coincident(first, second)])
        }
        "ACHORIZONTALCONSTRAINT" | "ACVERTICALCONSTRAINT" => {
            let horizontal = class == "ACHORIZONTALCONSTRAINT";
            if !standard_axis(group, node, horizontal) {
                return None;
            }
            match references {
                [reference] => {
                    let line = geometry.line(reference.whole()?)?;
                    Some(vec![if horizontal {
                        Constraint::Horizontal(line.id)
                    } else {
                        Constraint::Vertical(line.id)
                    }])
                }
                [first, second] => {
                    let (first, second) = (first.point()?, second.point()?);
                    Some(vec![if horizontal {
                        Constraint::HorizontalPoints(first, second)
                    } else {
                        Constraint::VerticalPoints(first, second)
                    }])
                }
                _ => None,
            }
        }
        "ACPARALLELCONSTRAINT" | "ACPERPENDICULARCONSTRAINT" => {
            let [first, second] = line_pair(references, geometry)?;
            Some(vec![if class == "ACPARALLELCONSTRAINT" {
                Constraint::Parallel(first, second)
            } else {
                Constraint::Perpendicular(first, second)
            }])
        }
        "ACEQUALLENGTHCONSTRAINT" => {
            let [first, second] = line_pair(references, geometry)?;
            Some(vec![Constraint::EqualLength(first, second)])
        }
        "ACEQUALRADIUSCONSTRAINT" => {
            let [first, second] = references else {
                return None;
            };
            equal_radius(first.whole()?, second.whole()?, geometry)
                .map(|constraint| vec![constraint])
        }
        "ACDISTANCECONSTRAINT" => {
            let AssocConstraintNodeData::Distance {
                value_dependency,
                direction_type,
                distance,
                ..
            } = &node.data
            else {
                return None;
            };
            let direction = match (*direction_type, distance) {
                (0, _) => DistanceDirection::Undirected,
                (1, Some(direction)) if axis_direction(*direction, true) => DistanceDirection::X,
                (1, Some(direction)) if axis_direction(*direction, false) => DistanceDirection::Y,
                _ => return None,
            };
            let value = scaled_value(driving_value(document, *value_dependency)?, scale?)?;
            match (direction, references) {
                (DistanceDirection::Undirected, [first, second]) => {
                    if let (Some(first), Some(second)) = (first.point(), second.point()) {
                        Some(vec![Constraint::Distance(first, second, value)])
                    } else if let Some((point, line)) = point_line(*first, *second, geometry) {
                        Some(vec![Constraint::DistancePointLine(point, line, value)])
                    } else {
                        None
                    }
                }
                (DistanceDirection::Undirected, [reference]) => {
                    let line = geometry.line(reference.whole()?)?;
                    Some(vec![Constraint::LineLength(line.id, value)])
                }
                (DistanceDirection::X, [first, second]) => {
                    Some(vec![Constraint::HorizontalDistance(
                        first.point()?,
                        second.point()?,
                        value,
                    )])
                }
                (DistanceDirection::Y, [first, second]) => {
                    Some(vec![Constraint::VerticalDistance(
                        first.point()?,
                        second.point()?,
                        value,
                    )])
                }
                _ => None,
            }
        }
        "ACRADIUSDIAMETERCONSTRAINT" => {
            let AssocConstraintNodeData::RadiusDiameter {
                value_dependency,
                mode,
                ..
            } = &node.data
            else {
                return None;
            };
            let value = scaled_value(driving_value(document, *value_dependency)?, scale?)?;
            if value <= 0.0 {
                return None;
            }
            let [reference] = references else {
                return None;
            };
            let primitive = reference.whole()?;
            match (*mode, primitive.kind) {
                (0, PrimitiveKind::Circle) => Some(vec![Constraint::Radius(
                    geometry.circle(primitive)?.id,
                    value,
                )]),
                (1, PrimitiveKind::Circle) => Some(vec![Constraint::Diameter(
                    geometry.circle(primitive)?.id,
                    value,
                )]),
                (0, PrimitiveKind::Arc) => Some(vec![Constraint::ArcRadius(
                    geometry.arc(primitive)?.id,
                    value,
                )]),
                (1, PrimitiveKind::Arc) => Some(vec![Constraint::ArcRadius(
                    geometry.arc(primitive)?.id,
                    value / 2.0,
                )]),
                _ => None,
            }
        }
        "ACANGLECONSTRAINT" => {
            let AssocConstraintNodeData::Angle {
                value_dependency,
                sector_type,
                ..
            } = &node.data
            else {
                return None;
            };
            if *sector_type != 0 {
                return None;
            }
            let [first, second] = line_pair(references, geometry)?;
            let angle = -driving_value(document, *value_dependency)?.to_radians();
            angle
                .is_finite()
                .then_some(vec![Constraint::Angle(first, second, angle)])
        }
        "ACFIXEDCONSTRAINT" => {
            let [reference] = references else {
                return None;
            };
            if let Some(point) = reference.point {
                return geometry
                    .fixed_point(point)
                    .map(|constraint| vec![constraint]);
            }
            match reference.primitive.kind {
                PrimitiveKind::Line => {
                    let line = geometry.line(reference.primitive)?;
                    Some(vec![
                        geometry.fixed_point(line.a)?,
                        geometry.fixed_point(line.b)?,
                    ])
                }
                PrimitiveKind::Circle => {
                    let circle = geometry.circle(reference.primitive)?;
                    if !circle.radius.is_finite() || circle.radius <= 0.0 {
                        return None;
                    }
                    Some(vec![
                        geometry.fixed_point(circle.center)?,
                        Constraint::Radius(circle.id, circle.radius),
                    ])
                }
                PrimitiveKind::Arc => None,
            }
        }
        "ACPOINTCURVECONSTRAINT" => {
            let [first, second] = references else {
                return None;
            };
            let (point, curve) = if let (Some(point), None) = (first.point, second.point) {
                (point, second.primitive)
            } else if let (None, Some(point)) = (first.point, second.point) {
                (point, first.primitive)
            } else {
                return None;
            };
            if first.primitive == second.primitive {
                return None;
            }
            match curve.kind {
                PrimitiveKind::Line => Some(vec![Constraint::PointOnLine(
                    point,
                    geometry.line(curve)?.id,
                )]),
                PrimitiveKind::Circle => Some(vec![Constraint::PointOnCircle(
                    point,
                    geometry.circle(curve)?.id,
                )]),
                PrimitiveKind::Arc => {
                    Some(vec![Constraint::PointOnArc(point, geometry.arc(curve)?.id)])
                }
            }
        }
        "ACTANGENTCONSTRAINT" => {
            let [first, second] = references else {
                return None;
            };
            tangent(first.whole()?, second.whole()?, geometry).map(|constraint| vec![constraint])
        }
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum DistanceDirection {
    Undirected,
    X,
    Y,
}

fn point_pair(references: &[Reference]) -> Option<[PointId; 2]> {
    let [first, second] = references else {
        return None;
    };
    Some([first.point?, second.point?])
}

fn line_pair(references: &[Reference], geometry: &GeometryMap<'_>) -> Option<[LineId; 2]> {
    let [first, second] = references else {
        return None;
    };
    Some([
        geometry.line(first.whole()?)?.id,
        geometry.line(second.whole()?)?.id,
    ])
}

fn point_line(
    first: Reference,
    second: Reference,
    geometry: &GeometryMap<'_>,
) -> Option<(PointId, LineId)> {
    if let (Some(point), None) = (first.point, second.point) {
        Some((point, geometry.line(second.primitive)?.id))
    } else if let (None, Some(point)) = (first.point, second.point) {
        Some((point, geometry.line(first.primitive)?.id))
    } else {
        None
    }
}

fn equal_radius(
    first: Primitive,
    second: Primitive,
    geometry: &GeometryMap<'_>,
) -> Option<Constraint> {
    match (first.kind, second.kind) {
        (PrimitiveKind::Circle, PrimitiveKind::Circle) => Some(Constraint::EqualRadius(
            geometry.circle(first)?.id,
            geometry.circle(second)?.id,
        )),
        (PrimitiveKind::Arc, PrimitiveKind::Arc) => Some(Constraint::EqualRadiusArc(
            geometry.arc(first)?.id,
            geometry.arc(second)?.id,
        )),
        (PrimitiveKind::Circle, PrimitiveKind::Arc) => Some(Constraint::EqualRadiusCircleArc(
            geometry.circle(first)?.id,
            geometry.arc(second)?.id,
        )),
        (PrimitiveKind::Arc, PrimitiveKind::Circle) => Some(Constraint::EqualRadiusCircleArc(
            geometry.circle(second)?.id,
            geometry.arc(first)?.id,
        )),
        _ => None,
    }
}

fn tangent(first: Primitive, second: Primitive, geometry: &GeometryMap<'_>) -> Option<Constraint> {
    match (first.kind, second.kind) {
        (PrimitiveKind::Line, PrimitiveKind::Circle) => Some(Constraint::TangentLineCircle(
            geometry.line(first)?.id,
            geometry.circle(second)?.id,
        )),
        (PrimitiveKind::Circle, PrimitiveKind::Line) => Some(Constraint::TangentLineCircle(
            geometry.line(second)?.id,
            geometry.circle(first)?.id,
        )),
        (PrimitiveKind::Line, PrimitiveKind::Arc) => Some(Constraint::TangentLineArc(
            geometry.line(first)?.id,
            geometry.arc(second)?.id,
        )),
        (PrimitiveKind::Arc, PrimitiveKind::Line) => Some(Constraint::TangentLineArc(
            geometry.line(second)?.id,
            geometry.arc(first)?.id,
        )),
        (PrimitiveKind::Circle, PrimitiveKind::Circle) => geometry
            .external_tangency(first, second)
            .then_some(Constraint::TangentCircleCircle(
                geometry.circle(first)?.id,
                geometry.circle(second)?.id,
            )),
        (PrimitiveKind::Arc, PrimitiveKind::Arc) => geometry
            .external_tangency(first, second)
            .then_some(Constraint::TangentArcArc(
                geometry.arc(first)?.id,
                geometry.arc(second)?.id,
            )),
        _ => None,
    }
}

fn standard_axis(group: &Assoc2dConstraintGroup, node: &AssocConstraintNode, x: bool) -> bool {
    let AssocConstraintNodeData::Parallel {
        datum_line_index, ..
    } = &node.data
    else {
        return false;
    };
    let Some(datum) = datum_line_index else {
        return true;
    };
    group
        .nodes
        .iter()
        .find(|node| node.node_id == *datum)
        .is_some_and(|node| match node.data {
            AssocConstraintNodeData::Line { direction, .. } => axis_direction(direction, x),
            _ => false,
        })
}

fn axis_direction(direction: Vector3, x: bool) -> bool {
    let length = direction.x.hypot(direction.y);
    if !length.is_finite() || length <= f64::EPSILON || direction.z.abs() > 1.0e-9 {
        return false;
    }
    let (x_component, y_component) = (direction.x / length, direction.y / length);
    if x {
        x_component.abs() >= 1.0 - 1.0e-9 && y_component.abs() <= 1.0e-9
    } else {
        x_component.abs() <= 1.0e-9 && y_component.abs() >= 1.0 - 1.0e-9
    }
}

fn scaled_value(value: f64, scale: f64) -> Option<f64> {
    let value = value.abs() / scale;
    (value.is_finite() && scale.is_finite() && scale > 0.0).then_some(value)
}

fn numeric_value(value: &ocs_plugin_api::host::acadrust::objects::AssocEvalVariant) -> Option<f64> {
    let value = match value.value {
        AssocEvalValue::Real(value) => value,
        AssocEvalValue::Long(value) => value as f64,
        AssocEvalValue::Short(value) => value as f64,
        AssocEvalValue::Byte(value) => value as f64,
        _ => return None,
    };
    value.is_finite().then_some(value)
}

fn driving_value(document: &Document, dependency: Handle) -> Option<f64> {
    let Some(ObjectType::Associative(AssociativeObject {
        data: AssociativeData::ValueDependency(dependency),
        ..
    })) = document.objects.get(&dependency)
    else {
        return None;
    };
    if let Some(value) = numeric_value(&dependency.value) {
        return Some(value);
    }
    let Some(ObjectType::Associative(AssociativeObject {
        data: AssociativeData::Variable(variable),
        ..
    })) = document.objects.get(&dependency.dependency.dependent_on)
    else {
        return None;
    };
    numeric_value(&variable.value).or_else(|| {
        variable
            .expression
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
    })
}

pub(crate) fn create_constraint_from_selection(
    host: &mut dyn HostApi,
    tangent: bool,
) -> Result<(), String> {
    let handles = host.selection();
    if handles.len() != 2 {
        return Err("select exactly two native entities".to_string());
    }
    let entities = host.entities_batch(&handles)?;
    let first = entities[0]
        .as_ref()
        .ok_or_else(|| "first selected entity is unavailable".to_string())?;
    let second = entities[1]
        .as_ref()
        .ok_or_else(|| "second selected entity is unavailable".to_string())?;
    let first = pick_for_entity(first, tangent, false);
    let second = pick_for_entity(second, tangent, true);
    if tangent {
        create_native_tangent(host, first, second)
    } else {
        create_native_coincident(host, first, second)
    }
}

fn pick_for_entity(entity: &EntityType, tangent: bool, second: bool) -> PluginConstraintPick {
    let (handle, point) = match entity {
        EntityType::Line(line) => {
            let point = if tangent {
                line.midpoint()
            } else if second {
                line.start
            } else {
                line.end
            };
            (Some(entity.common().handle), point)
        }
        EntityType::Circle(circle) => (Some(entity.common().handle), circle.center),
        EntityType::Arc(arc) => (Some(entity.common().handle), arc.center),
        EntityType::LwPolyline(polyline) => {
            let point = polyline
                .vertices
                .first()
                .map(|vertex| Vector3::new(vertex.location.x, vertex.location.y, 0.0))
                .unwrap_or(Vector3::ZERO);
            (Some(entity.common().handle), point)
        }
        _ => (Some(entity.common().handle), Vector3::ZERO),
    };
    PluginConstraintPick {
        handle,
        point: [point.x, point.y, point.z],
        whole_curve: tangent,
    }
}

pub(crate) fn create_native_coincident(
    host: &mut dyn HostApi,
    first: PluginConstraintPick,
    second: PluginConstraintPick,
) -> Result<(), String> {
    host.constraint_create(PluginConstraint::Coincident { first, second })
}

pub(crate) fn create_native_tangent(
    host: &mut dyn HostApi,
    first: PluginConstraintPick,
    second: PluginConstraintPick,
) -> Result<(), String> {
    host.constraint_create(PluginConstraint::Tangent { first, second })
}

#[cfg(test)]
mod tests {
    use ocs_plugin_api::host::acadrust::entities::{Arc, Circle, Line, LwPolyline, LwVertex};
    use ocs_plugin_api::host::acadrust::objects::{
        AssocDependency, AssocEvalVariant, AssocGeomDependency,
    };
    use rf_core::sketch::{
        ArcId, CircleId, LineId, PointId, SketchArc, SketchCircle, SketchLine, SketchPoint,
    };

    use super::*;

    fn point(id: u32, x: f64, y: f64) -> SketchPoint {
        SketchPoint {
            id: PointId(id),
            x,
            y,
        }
    }

    fn standalone_selection(entities: Vec<EntityType>, scale: f64) -> (Document, NativeSelection) {
        let mut document = Document::new();
        let mut originals = Vec::new();
        for entity in entities {
            let handle = document.add_entity(entity).unwrap();
            originals.push(document.get_entity(handle).unwrap().clone());
        }
        let mut sketch = Sketch::default();
        let mut line_origins = Vec::new();
        let mut circle_origins = Vec::new();
        let mut arc_origins = Vec::new();
        for entity in &originals {
            let handle = entity.common().handle;
            match entity {
                EntityType::Line(line) => {
                    let a = PointId(sketch.points.len() as u32 + 1);
                    sketch.points.push(point(
                        a.0,
                        line.start.x / scale,
                        100.0 - line.start.y / scale,
                    ));
                    let b = PointId(sketch.points.len() as u32 + 1);
                    sketch
                        .points
                        .push(point(b.0, line.end.x / scale, 100.0 - line.end.y / scale));
                    sketch.lines.push(SketchLine {
                        id: LineId(sketch.lines.len() as u32 + 1),
                        a,
                        b,
                        construction: false,
                    });
                    line_origins.push(handle);
                }
                EntityType::Circle(circle) => {
                    let center = PointId(sketch.points.len() as u32 + 1);
                    sketch.points.push(point(
                        center.0,
                        circle.center.x / scale,
                        100.0 - circle.center.y / scale,
                    ));
                    sketch.circles.push(SketchCircle {
                        id: CircleId(sketch.circles.len() as u32 + 1),
                        center,
                        radius: circle.radius / scale,
                        construction: false,
                    });
                    circle_origins.push(handle);
                }
                EntityType::Arc(arc) => {
                    let center = PointId(sketch.points.len() as u32 + 1);
                    sketch.points.push(point(
                        center.0,
                        arc.center.x / scale,
                        100.0 - arc.center.y / scale,
                    ));
                    sketch.arcs.push(SketchArc {
                        id: ArcId(sketch.arcs.len() as u32 + 1),
                        center,
                        radius: arc.radius / scale,
                        start: -arc.end_angle,
                        end: -arc.start_angle,
                        construction: false,
                    });
                    arc_origins.push(handle);
                }
                _ => unreachable!(),
            }
        }
        let primitive_origins = line_origins
            .into_iter()
            .chain(circle_origins)
            .chain(arc_origins)
            .collect();
        (
            document,
            NativeSelection {
                sketch,
                originals,
                primitive_origins,
            },
        )
    }

    fn associative(document: &mut Document, handle: u64, data: AssociativeData) {
        let handle = Handle::new(handle);
        document.objects.insert(
            handle,
            ObjectType::Associative(AssociativeObject {
                handle,
                data,
                ..Default::default()
            }),
        );
    }

    fn dependency(document: &mut Document, handle: u64, entity: Handle) {
        associative(
            document,
            handle,
            AssociativeData::GeomDependency(AssocGeomDependency {
                dependency: AssocDependency {
                    dependent_on: entity,
                    ..Default::default()
                },
                enabled: true,
                ..Default::default()
            }),
        );
    }

    fn value(document: &mut Document, handle: u64, number: f64) {
        associative(
            document,
            handle,
            AssociativeData::ValueDependency(
                ocs_plugin_api::host::acadrust::objects::AssocValueDependency {
                    value: AssocEvalVariant {
                        code: 40,
                        value: AssocEvalValue::Real(number),
                    },
                    ..Default::default()
                },
            ),
        );
    }

    fn group() -> Assoc2dConstraintGroup {
        Assoc2dConstraintGroup {
            work_plane: [Vector3::ZERO, Vector3::UNIT_X, Vector3::UNIT_Y],
            ..Default::default()
        }
    }

    fn node(id: i32, class_name: &str, data: AssocConstraintNodeData) -> AssocConstraintNode {
        AssocConstraintNode {
            node_id: id,
            class_name: class_name.to_string(),
            data,
            ..Default::default()
        }
    }

    fn line_node(id: i32, dependency: Handle, line: &Line) -> AssocConstraintNode {
        node(
            id,
            "AcConstrainedBoundedLine",
            AssocConstraintNodeData::BoundedLine {
                geometry_dependency: dependency,
                geometry_node_id: id,
                point: line.start,
                direction: (line.end - line.start).normalize(),
                is_ray: false,
                start_point: line.start,
                end_point: line.end,
            },
        )
    }

    fn circle_node(id: i32, dependency: Handle, circle: &Circle) -> AssocConstraintNode {
        node(
            id,
            "AcConstrainedCircle",
            AssocConstraintNodeData::Circle {
                geometry_dependency: dependency,
                geometry_node_id: id,
                center: circle.center,
                normal: circle.normal,
                direction: Vector3::UNIT_X,
                radius: circle.radius,
                start_parameter: 0.0,
                end_parameter: std::f64::consts::TAU,
                reserved: 0.0,
            },
        )
    }

    fn arc_node(id: i32, dependency: Handle, arc: &Arc) -> AssocConstraintNode {
        node(
            id,
            "AcConstrainedArc",
            AssocConstraintNodeData::Arc {
                geometry_dependency: dependency,
                geometry_node_id: id,
                center: arc.center,
                normal: arc.normal,
                direction: Vector3::UNIT_X,
                radius: arc.radius,
                start_parameter: arc.start_angle,
                end_parameter: arc.end_angle,
                reserved: 0.0,
                start_point: Vector3::ZERO,
                end_point: Vector3::ZERO,
            },
        )
    }

    fn implicit(id: i32, curve_id: i32, point_type: u8) -> AssocConstraintNode {
        node(
            id,
            "AcConstrainedImplicitPoint",
            AssocConstraintNodeData::ImplicitPoint {
                geometry_dependency: Handle::NULL,
                geometry_node_id: id,
                point: None,
                point_type,
                point_index: -1,
                curve_id,
            },
        )
    }

    fn geometrical(
        id: i32,
        class_name: &str,
        connections: Vec<i32>,
        active: bool,
    ) -> AssocConstraintNode {
        AssocConstraintNode {
            connections,
            ..node(
                id,
                class_name,
                AssocConstraintNodeData::Geometrical {
                    owner_id: 0,
                    is_implied: false,
                    is_active: active,
                },
            )
        }
    }

    fn dimensional(
        id: i32,
        class_name: &str,
        connections: Vec<i32>,
        data: AssocConstraintNodeData,
    ) -> AssocConstraintNode {
        AssocConstraintNode {
            connections,
            ..node(id, class_name, data)
        }
    }

    fn distance(id: i32, connections: Vec<i32>, dependency: u64) -> AssocConstraintNode {
        dimensional(
            id,
            "AcDistanceConstraint",
            connections,
            AssocConstraintNodeData::Distance {
                owner_id: 0,
                is_implied: false,
                is_active: true,
                value_dependency: Handle::new(dependency),
                dimension_dependency: Handle::NULL,
                direction_type: 0,
                distance: None,
            },
        )
    }

    #[test]
    fn reads_real_coincident_graph_and_applies_reference_offset() {
        let (mut document, selection) = standalone_selection(
            vec![
                EntityType::Line(Line::from_coords(0.0, 0.0, 0.0, 10.0, 0.0, 0.0)),
                EntityType::Line(Line::from_coords(10.0, 0.0, 0.0, 20.0, 0.0, 0.0)),
            ],
            2.0,
        );
        let first = selection.primitive_origins[0];
        let second = selection.primitive_origins[1];
        dependency(&mut document, 1001, first);
        dependency(&mut document, 1002, second);
        let mut graph = group();
        graph.nodes = vec![
            line_node(
                1,
                Handle::new(1001),
                &Line::from_coords(0.0, 0.0, 0.0, 10.0, 0.0, 0.0),
            ),
            line_node(
                2,
                Handle::new(1002),
                &Line::from_coords(10.0, 0.0, 0.0, 20.0, 0.0, 0.0),
            ),
            implicit(3, 1, 1),
            implicit(4, 2, 0),
            geometrical(5, "AcPointCoincidenceConstraint", vec![3, 4], false),
        ];
        associative(&mut document, 900, AssociativeData::ConstraintGroup(graph));

        let read = read_native_constraints(&document, &selection).unwrap();
        assert_eq!(read.mapped, 1);
        assert_eq!(read.skipped, 0);
        assert_eq!(
            read.constraints,
            vec![Constraint::Coincident(PointId(2), PointId(3))]
        );
        assert_eq!(read.reference_constraints, vec![0]);

        let mut sketch = Sketch {
            constraints: vec![Constraint::Horizontal(LineId(99))],
            ..Default::default()
        };
        read.apply_to_sketch(&mut sketch);
        assert_eq!(sketch.constraints.len(), 2);
        assert_eq!(sketch.reference_constraints, vec![1]);
    }

    #[test]
    fn maps_supported_families_and_scales_native_values() {
        let line_a = Line::from_coords(0.0, 0.0, 0.0, 10.0, 0.0, 0.0);
        let line_b = Line::from_coords(20.0, 0.0, 0.0, 20.0, 10.0, 0.0);
        let circle = Circle::from_coords(30.0, 30.0, 0.0, 12.0);
        let arc = Arc::from_coords(50.0, 30.0, 0.0, 10.0, 0.0, 1.0);
        let (mut document, selection) = standalone_selection(
            vec![
                EntityType::Line(line_a.clone()),
                EntityType::Line(line_b.clone()),
                EntityType::Circle(circle.clone()),
                EntityType::Arc(arc.clone()),
            ],
            2.0,
        );
        for (index, handle) in selection.primitive_origins.iter().copied().enumerate() {
            dependency(&mut document, 1001 + index as u64, handle);
        }
        for (index, number) in [20.0, 8.0, 6.0, 12.0, 24.0, 10.0, 20.0, 90.0]
            .into_iter()
            .enumerate()
        {
            value(&mut document, 2001 + index as u64, number);
        }
        let mut graph = group();
        graph.nodes = vec![
            line_node(1, Handle::new(1001), &line_a),
            line_node(2, Handle::new(1002), &line_b),
            implicit(3, 1, 0),
            implicit(4, 1, 1),
            implicit(5, 2, 0),
            circle_node(6, Handle::new(1003), &circle),
            arc_node(7, Handle::new(1004), &arc),
            distance(10, vec![3, 4], 2001),
            dimensional(
                11,
                "AcDistanceConstraint",
                vec![4, 5],
                AssocConstraintNodeData::Distance {
                    owner_id: 0,
                    is_implied: false,
                    is_active: true,
                    value_dependency: Handle::new(2002),
                    dimension_dependency: Handle::NULL,
                    direction_type: 1,
                    distance: Some(Vector3::UNIT_X),
                },
            ),
            dimensional(
                12,
                "AcDistanceConstraint",
                vec![3, 5],
                AssocConstraintNodeData::Distance {
                    owner_id: 0,
                    is_implied: false,
                    is_active: true,
                    value_dependency: Handle::new(2003),
                    dimension_dependency: Handle::NULL,
                    direction_type: 1,
                    distance: Some(Vector3::UNIT_Y),
                },
            ),
            dimensional(
                13,
                "AcRadiusDiameterConstraint",
                vec![6],
                AssocConstraintNodeData::RadiusDiameter {
                    owner_id: 0,
                    is_implied: false,
                    is_active: true,
                    value_dependency: Handle::new(2004),
                    dimension_dependency: Handle::NULL,
                    mode: 0,
                },
            ),
            dimensional(
                14,
                "AcRadiusDiameterConstraint",
                vec![6],
                AssocConstraintNodeData::RadiusDiameter {
                    owner_id: 0,
                    is_implied: false,
                    is_active: true,
                    value_dependency: Handle::new(2005),
                    dimension_dependency: Handle::NULL,
                    mode: 1,
                },
            ),
            dimensional(
                15,
                "AcRadiusDiameterConstraint",
                vec![7],
                AssocConstraintNodeData::RadiusDiameter {
                    owner_id: 0,
                    is_implied: false,
                    is_active: true,
                    value_dependency: Handle::new(2006),
                    dimension_dependency: Handle::NULL,
                    mode: 0,
                },
            ),
            dimensional(
                16,
                "AcRadiusDiameterConstraint",
                vec![7],
                AssocConstraintNodeData::RadiusDiameter {
                    owner_id: 0,
                    is_implied: false,
                    is_active: true,
                    value_dependency: Handle::new(2007),
                    dimension_dependency: Handle::NULL,
                    mode: 1,
                },
            ),
            dimensional(
                17,
                "AcAngleConstraint",
                vec![1, 2],
                AssocConstraintNodeData::Angle {
                    owner_id: 0,
                    is_implied: false,
                    is_active: true,
                    value_dependency: Handle::new(2008),
                    dimension_dependency: Handle::NULL,
                    sector_type: 0,
                },
            ),
            geometrical(18, "AcFixedConstraint", vec![3], true),
            geometrical(19, "AcPointCurveConstraint", vec![5, 1], true),
            geometrical(20, "AcTangentConstraint", vec![2, 6], true),
            geometrical(21, "AcEqualRadiusConstraint", vec![6, 7], true),
            dimensional(
                22,
                "AcHorizontalConstraint",
                vec![1],
                AssocConstraintNodeData::Parallel {
                    owner_id: 0,
                    is_implied: false,
                    is_active: true,
                    datum_line_index: None,
                },
            ),
            dimensional(
                23,
                "AcVerticalConstraint",
                vec![2],
                AssocConstraintNodeData::Parallel {
                    owner_id: 0,
                    is_implied: false,
                    is_active: true,
                    datum_line_index: None,
                },
            ),
            dimensional(
                24,
                "AcParallelConstraint",
                vec![1, 2],
                AssocConstraintNodeData::Parallel {
                    owner_id: 0,
                    is_implied: false,
                    is_active: true,
                    datum_line_index: None,
                },
            ),
            geometrical(25, "AcPerpendicularConstraint", vec![1, 2], true),
            geometrical(26, "AcEqualLengthConstraint", vec![1, 2], true),
            geometrical(27, "AcPointCurveConstraint", vec![5, 6], true),
            geometrical(28, "AcPointCurveConstraint", vec![5, 7], true),
            geometrical(29, "AcTangentConstraint", vec![1, 7], true),
        ];
        associative(&mut document, 900, AssociativeData::ConstraintGroup(graph));

        let read = read_native_constraints(&document, &selection).unwrap();
        assert_eq!(read.mapped, 20);
        assert_eq!(read.skipped, 0);
        assert_eq!(read.reference_constraints, Vec::<usize>::new());
        assert_eq!(read.constraints.len(), 20);
        assert_eq!(
            read.constraints[0],
            Constraint::Distance(PointId(1), PointId(2), 10.0)
        );
        assert_eq!(
            read.constraints[1],
            Constraint::HorizontalDistance(PointId(2), PointId(3), 4.0)
        );
        assert_eq!(
            read.constraints[2],
            Constraint::VerticalDistance(PointId(1), PointId(3), 3.0)
        );
        assert_eq!(read.constraints[3], Constraint::Radius(CircleId(1), 6.0));
        assert_eq!(read.constraints[4], Constraint::Diameter(CircleId(1), 12.0));
        assert_eq!(read.constraints[5], Constraint::ArcRadius(ArcId(1), 5.0));
        assert_eq!(read.constraints[6], Constraint::ArcRadius(ArcId(1), 5.0));
        assert_eq!(
            read.constraints[7],
            Constraint::Angle(LineId(1), LineId(2), -std::f64::consts::FRAC_PI_2)
        );
        assert_eq!(
            read.constraints[8],
            Constraint::FixedPoint(PointId(1), 0.0, 100.0)
        );
        assert_eq!(
            read.constraints[9],
            Constraint::PointOnLine(PointId(3), LineId(1))
        );
        assert_eq!(
            read.constraints[10],
            Constraint::TangentLineCircle(LineId(2), CircleId(1))
        );
        assert_eq!(
            read.constraints[11],
            Constraint::EqualRadiusCircleArc(CircleId(1), ArcId(1))
        );
        assert_eq!(read.constraints[12], Constraint::Horizontal(LineId(1)));
        assert_eq!(read.constraints[13], Constraint::Vertical(LineId(2)));
        assert_eq!(
            read.constraints[14],
            Constraint::Parallel(LineId(1), LineId(2))
        );
        assert_eq!(
            read.constraints[15],
            Constraint::Perpendicular(LineId(1), LineId(2))
        );
        assert_eq!(
            read.constraints[16],
            Constraint::EqualLength(LineId(1), LineId(2))
        );
        assert_eq!(
            read.constraints[17],
            Constraint::PointOnCircle(PointId(3), CircleId(1))
        );
        assert_eq!(
            read.constraints[18],
            Constraint::PointOnArc(PointId(3), ArcId(1))
        );
        assert_eq!(
            read.constraints[19],
            Constraint::TangentLineArc(LineId(1), ArcId(1))
        );
    }

    #[test]
    fn resolves_polyline_segments_and_endpoint_markers() {
        let mut polyline = LwPolyline::new();
        polyline.vertices = vec![
            LwVertex::from_coords(0.0, 0.0),
            LwVertex::from_coords(10.0, 0.0),
            LwVertex::from_coords(10.0, 10.0),
        ];
        let mut document = Document::new();
        let handle = document
            .add_entity(EntityType::LwPolyline(polyline.clone()))
            .unwrap();
        let original = document.get_entity(handle).unwrap().clone();
        let selection = NativeSelection {
            sketch: Sketch {
                points: vec![
                    point(1, 0.0, 100.0),
                    point(2, 5.0, 100.0),
                    point(3, 5.0, 95.0),
                ],
                lines: vec![
                    SketchLine {
                        id: LineId(1),
                        a: PointId(1),
                        b: PointId(2),
                        construction: false,
                    },
                    SketchLine {
                        id: LineId(2),
                        a: PointId(2),
                        b: PointId(3),
                        construction: false,
                    },
                ],
                ..Default::default()
            },
            originals: vec![original],
            primitive_origins: vec![handle, handle],
        };
        dependency(&mut document, 1001, handle);
        dependency(&mut document, 1002, handle);
        let mut graph = group();
        graph.work_plane[0] = Vector3::new(100.0, 0.0, 0.0);
        graph.nodes = vec![
            node(
                1,
                "AcConstrainedBoundedLine",
                AssocConstraintNodeData::BoundedLine {
                    geometry_dependency: Handle::new(1001),
                    geometry_node_id: 1,
                    point: Vector3::new(-100.0, 0.0, 0.0),
                    direction: Vector3::UNIT_X,
                    is_ray: false,
                    start_point: Vector3::new(-100.0, 0.0, 0.0),
                    end_point: Vector3::new(-90.0, 0.0, 0.0),
                },
            ),
            node(
                2,
                "AcConstrainedBoundedLine",
                AssocConstraintNodeData::BoundedLine {
                    geometry_dependency: Handle::new(1002),
                    geometry_node_id: 2,
                    point: Vector3::new(-90.0, 0.0, 0.0),
                    direction: Vector3::UNIT_Y,
                    is_ray: false,
                    start_point: Vector3::new(-90.0, 0.0, 0.0),
                    end_point: Vector3::new(-90.0, 10.0, 0.0),
                },
            ),
            implicit(3, 2, 1),
            implicit(4, 1, 0),
            geometrical(5, "AcPointCoincidenceConstraint", vec![3, 4], true),
            dimensional(
                6,
                "AcVerticalConstraint",
                vec![2],
                AssocConstraintNodeData::Parallel {
                    owner_id: 0,
                    is_implied: false,
                    is_active: true,
                    datum_line_index: None,
                },
            ),
        ];
        associative(&mut document, 900, AssociativeData::ConstraintGroup(graph));

        let read = read_native_constraints(&document, &selection).unwrap();
        assert_eq!(read.mapped, 2);
        assert_eq!(read.skipped, 0);
        assert_eq!(
            read.constraints,
            vec![
                Constraint::Coincident(PointId(3), PointId(1)),
                Constraint::Vertical(LineId(2)),
            ]
        );
    }

    #[test]
    fn skips_foreign_invalid_and_unsupported_constraints_without_panicking() {
        let selected_line = Line::from_coords(0.0, 0.0, 0.0, 10.0, 0.0, 0.0);
        let selected_arc = Arc::from_coords(30.0, 0.0, 0.0, 10.0, 0.0, 1.0);
        let (mut document, selection) = standalone_selection(
            vec![
                EntityType::Line(selected_line.clone()),
                EntityType::Arc(selected_arc.clone()),
            ],
            2.0,
        );
        let foreign = document
            .add_entity(EntityType::Line(Line::from_coords(
                50.0, 0.0, 0.0, 60.0, 0.0, 0.0,
            )))
            .unwrap();
        dependency(&mut document, 1001, selection.primitive_origins[0]);
        dependency(&mut document, 1002, selection.primitive_origins[1]);
        dependency(&mut document, 1003, foreign);
        associative(
            &mut document,
            2001,
            AssociativeData::ValueDependency(Default::default()),
        );
        let mut graph = group();
        graph.nodes = vec![
            line_node(1, Handle::new(1001), &selected_line),
            arc_node(2, Handle::new(1002), &selected_arc),
            line_node(
                3,
                Handle::new(1003),
                &Line::from_coords(50.0, 0.0, 0.0, 60.0, 0.0, 0.0),
            ),
            implicit(4, 1, 0),
            implicit(5, 3, 0),
            implicit(6, 2, 0),
            geometrical(10, "AcPointCoincidenceConstraint", vec![4, 5], true),
            distance(11, vec![4, 4], 2001),
            geometrical(12, "AcPointCoincidenceConstraint", vec![4, 6], true),
            geometrical(13, "AcG2SmoothConstraint", vec![1], true),
            geometrical(14, "AcPointCoincidenceConstraint", vec![5, 5], true),
        ];
        associative(&mut document, 900, AssociativeData::ConstraintGroup(graph));

        let read = read_native_constraints(&document, &selection).unwrap();
        assert_eq!(read.mapped, 0);
        assert_eq!(read.skipped, 4);
        assert!(read.constraints.is_empty());

        let mut bad_selection = NativeSelection {
            sketch: selection.sketch.clone(),
            originals: selection.originals.clone(),
            primitive_origins: Vec::new(),
        };
        assert!(read_native_constraints(&document, &bad_selection).is_err());
        bad_selection.primitive_origins = selection.primitive_origins.clone();
        assert!(read_native_constraints(&document, &bad_selection).is_ok());
    }

    #[test]
    fn maps_only_external_circle_tangency() {
        let first = Circle::from_coords(0.0, 0.0, 0.0, 5.0);
        let second = Circle::from_coords(10.0, 0.0, 0.0, 5.0);
        let (mut document, selection) = standalone_selection(
            vec![
                EntityType::Circle(first.clone()),
                EntityType::Circle(second.clone()),
            ],
            2.0,
        );
        dependency(&mut document, 1001, selection.primitive_origins[0]);
        dependency(&mut document, 1002, selection.primitive_origins[1]);
        let mut graph = group();
        graph.nodes = vec![
            circle_node(1, Handle::new(1001), &first),
            circle_node(2, Handle::new(1002), &second),
            geometrical(3, "AcTangentConstraint", vec![1, 2], true),
        ];
        associative(&mut document, 900, AssociativeData::ConstraintGroup(graph));
        let read = read_native_constraints(&document, &selection).unwrap();
        assert_eq!(
            read.constraints,
            vec![Constraint::TangentCircleCircle(CircleId(1), CircleId(2))]
        );

        let internal = Circle::from_coords(2.0, 0.0, 0.0, 3.0);
        let (mut document, selection) = standalone_selection(
            vec![
                EntityType::Circle(first.clone()),
                EntityType::Circle(internal.clone()),
            ],
            2.0,
        );
        dependency(&mut document, 1001, selection.primitive_origins[0]);
        dependency(&mut document, 1002, selection.primitive_origins[1]);
        let mut graph = group();
        graph.nodes = vec![
            circle_node(1, Handle::new(1001), &first),
            circle_node(2, Handle::new(1002), &internal),
            geometrical(3, "AcTangentConstraint", vec![1, 2], true),
        ];
        associative(&mut document, 900, AssociativeData::ConstraintGroup(graph));
        let read = read_native_constraints(&document, &selection).unwrap();
        assert_eq!(read.mapped, 0);
        assert_eq!(read.skipped, 1);
    }
}
