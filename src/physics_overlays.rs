use std::f32::consts::FRAC_PI_2;

use avian3d::prelude::*;
use bevy::prelude::*;
use parry3d::shape::TypedShape;

use crate::colors;
use crate::selection::Selected;

#[derive(Resource)]
pub struct PhysicsOverlayConfig {
    pub show_colliders: bool,
    pub show_hierarchy_arrows: bool,
}

impl Default for PhysicsOverlayConfig {
    fn default() -> Self {
        Self {
            show_colliders: true,
            show_hierarchy_arrows: false,
        }
    }
}

#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct ColliderGizmoGroup;

pub struct PhysicsOverlaysPlugin;

impl Plugin for PhysicsOverlaysPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PhysicsOverlayConfig>()
            .init_gizmo_group::<ColliderGizmoGroup>()
            .add_systems(
                PostUpdate,
                (
                    draw_collider_gizmos,
                    draw_hierarchy_arrows,
                )
                    .after(bevy::transform::TransformSystems::Propagate)
                    .run_if(in_state(crate::AppState::Editor)),
            );

        let mut store = app.world_mut().resource_mut::<GizmoConfigStore>();
        let (config, _) = store.config_mut::<ColliderGizmoGroup>();
        config.depth_bias = -0.5;
        config.line.width = 1.5;
    }
}

/// Convert a parry3d Vector (glam 0.30) to bevy Vec3 (glam 0.32).
fn parry_vec(v: parry3d::math::Vector) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

fn parry_point(p: &parry3d::math::Vector) -> Vec3 {
    Vec3::new(p.x, p.y, p.z)
}

fn draw_collider_gizmos(
    mut gizmos: Gizmos<ColliderGizmoGroup>,
    config: Res<PhysicsOverlayConfig>,
    colliders: Query<(
        Entity,
        &ColliderConstructor,
        &GlobalTransform,
        &InheritedVisibility,
        Option<&Sensor>,
        Option<&Mesh3d>,
    )>,
    selected_bodies: Query<Entity, (With<RigidBody>, With<Selected>)>,
    children_query: Query<&Children>,
    collider_check: Query<(), With<ColliderConstructor>>,
    mesh3d_query: Query<&Mesh3d>,
    meshes: Res<Assets<Mesh>>,
) {
    if !config.show_colliders {
        return;
    }

    // Collect highlighted colliders (belonging to a selected rigid body)
    let mut highlighted_colliders = bevy::ecs::entity::EntityHashSet::default();
    for body_entity in &selected_bodies {
        collect_descendant_colliders(
            body_entity,
            &children_query,
            &collider_check,
            &mut highlighted_colliders,
        );
        if collider_check.contains(body_entity) {
            highlighted_colliders.insert(body_entity);
        }
    }

    for (entity, constructor, tf, vis, sensor, mesh3d) in &colliders {
        if !vis.get() {
            continue;
        }

        let is_highlighted = highlighted_colliders.contains(&entity);
        let color = match (sensor.is_some(), is_highlighted) {
            (false, false) => colors::COLLIDER_WIREFRAME,
            (false, true) => colors::COLLIDER_SELECTED,
            (true, false) => colors::SENSOR_WIREFRAME,
            (true, true) => colors::SENSOR_SELECTED,
        };

        let transform = tf.compute_transform();
        let pos = transform.translation;
        let rot = transform.rotation;

        // Try to compute the actual Collider shape from the constructor.
        // For mesh-based variants, look for Mesh3d on self or children.
        let mesh: Option<&Mesh> = mesh3d
            .and_then(|m| meshes.get(&m.0))
            .or_else(|| {
                // Brush entities have Mesh3d on child face entities
                children_query.get(entity).ok().and_then(|children| {
                    children.iter().find_map(|child| {
                        mesh3d_query.get(child).ok().and_then(|m| meshes.get(&m.0))
                    })
                })
            });

        let collider = Collider::try_from_constructor(constructor.clone(), mesh);

        match &collider {
            Some(c) => {
                draw_parry_shape(&mut gizmos, c.shape(), pos, rot, color);
            }
            None => {
                // Log once to help debug — mesh might not be loaded
                if constructor.requires_mesh() {
                    // Mesh-based variant but no mesh found
                } else {
                    warn_once!("Collider::try_from_constructor returned None for {:?}", constructor);
                }
            }
        }
    }
}

/// Draw a wireframe for any parry shape using TypedShape pattern matching.
fn draw_parry_shape(
    gizmos: &mut Gizmos<ColliderGizmoGroup>,
    shape: &parry3d::shape::SharedShape,
    pos: Vec3,
    rot: Quat,
    color: Color,
) {
    match shape.as_typed_shape() {
        TypedShape::Ball(ball) => {
            let r = if ball.radius > 0.0 { ball.radius } else { 0.5 };
            gizmos.circle(Isometry3d::new(pos, rot * Quat::from_rotation_x(FRAC_PI_2)), r, color);
            gizmos.circle(Isometry3d::new(pos, rot), r, color);
            gizmos.circle(Isometry3d::new(pos, rot * Quat::from_rotation_y(FRAC_PI_2)), r, color);
        }
        TypedShape::Cuboid(cuboid) => {
            let he = cuboid.half_extents;
            let half = Vec3::new(he.x, he.y, he.z);
            let half = if half.length_squared() < 0.0001 { Vec3::splat(0.5) } else { half };
            draw_box_wireframe(gizmos, pos, rot, half, color);
        }
        TypedShape::RoundCuboid(rc) => {
            let he = rc.inner_shape.half_extents;
            let half = Vec3::new(he.x, he.y, he.z);
            let half = if half.length_squared() < 0.0001 { Vec3::splat(0.5) } else { half };
            draw_box_wireframe(gizmos, pos, rot, half, color);
        }
        TypedShape::Cylinder(cyl) => {
            let r = cyl.radius;
            let half_h = cyl.half_height;
            let up = rot * Vec3::Y;
            gizmos.circle(Isometry3d::new(pos + up * half_h, rot), r, color);
            gizmos.circle(Isometry3d::new(pos - up * half_h, rot), r, color);
            let right = rot * Vec3::X;
            let fwd = rot * Vec3::Z;
            for dir in [right, -right, fwd, -fwd] {
                gizmos.line(pos + dir * r + up * half_h, pos + dir * r - up * half_h, color);
            }
        }
        TypedShape::Cone(cone) => {
            let r = cone.radius;
            let half_h = cone.half_height;
            let up = rot * Vec3::Y;
            let apex = pos + up * half_h;
            gizmos.circle(Isometry3d::new(pos - up * half_h, rot), r, color);
            let right = rot * Vec3::X;
            let fwd = rot * Vec3::Z;
            for dir in [right, -right, fwd, -fwd] {
                gizmos.line(pos + dir * r - up * half_h, apex, color);
            }
        }
        TypedShape::Capsule(cap) => {
            let r = cap.radius;
            let a = parry_point(&cap.segment.a);
            let b = parry_point(&cap.segment.b);
            let half_h = (b - a).length() * 0.5;
            let up = rot * Vec3::Y;
            let right = rot * Vec3::X;
            let fwd = rot * Vec3::Z;
            gizmos.circle(Isometry3d::new(pos + up * half_h, rot), r, color);
            gizmos.circle(Isometry3d::new(pos - up * half_h, rot), r, color);
            // Hemisphere arcs
            gizmos.arc_3d(std::f32::consts::PI, r, Isometry3d::new(pos + up * half_h, rot * Quat::from_rotation_z(FRAC_PI_2)), color);
            gizmos.arc_3d(std::f32::consts::PI, r, Isometry3d::new(pos - up * half_h, rot * Quat::from_rotation_z(-FRAC_PI_2)), color);
            gizmos.arc_3d(std::f32::consts::PI, r, Isometry3d::new(pos + up * half_h, rot * Quat::from_rotation_z(FRAC_PI_2) * Quat::from_rotation_y(FRAC_PI_2)), color);
            gizmos.arc_3d(std::f32::consts::PI, r, Isometry3d::new(pos - up * half_h, rot * Quat::from_rotation_z(-FRAC_PI_2) * Quat::from_rotation_y(FRAC_PI_2)), color);
            for dir in [right, -right, fwd, -fwd] {
                gizmos.line(pos + dir * r + up * half_h, pos + dir * r - up * half_h, color);
            }
        }
        TypedShape::TriMesh(trimesh) => {
            let vertices = trimesh.vertices();
            let indices = trimesh.indices();
            for tri in indices {
                let a = pos + rot * parry_point(&vertices[tri[0] as usize]);
                let b = pos + rot * parry_point(&vertices[tri[1] as usize]);
                let c = pos + rot * parry_point(&vertices[tri[2] as usize]);
                gizmos.line(a, b, color);
                gizmos.line(b, c, color);
                gizmos.line(c, a, color);
            }
        }
        TypedShape::ConvexPolyhedron(poly) => {
            let points = poly.points();
            for edge in poly.edges() {
                let a = pos + rot * parry_vec(points[edge.vertices[0] as usize]);
                let b = pos + rot * parry_vec(points[edge.vertices[1] as usize]);
                gizmos.line(a, b, color);
            }
        }
        TypedShape::Compound(compound) => {
            for (iso, sub_shape) in compound.shapes() {
                let sub_pos = pos + rot * Vec3::new(
                    iso.translation.x,
                    iso.translation.y,
                    iso.translation.z,
                );
                // Approximate sub-rotation
                let sub_rot = rot; // TODO: compose with iso rotation
                draw_parry_shape(gizmos, sub_shape, sub_pos, sub_rot, color);
            }
        }
        TypedShape::HalfSpace(_) => {
            // Draw a large plane indicator
            let right = rot * Vec3::X * 5.0;
            let fwd = rot * Vec3::Z * 5.0;
            gizmos.line(pos - right - fwd, pos + right - fwd, color);
            gizmos.line(pos + right - fwd, pos + right + fwd, color);
            gizmos.line(pos + right + fwd, pos - right + fwd, color);
            gizmos.line(pos - right + fwd, pos - right - fwd, color);
            // Normal arrow
            gizmos.arrow(pos, pos + rot * Vec3::Y * 2.0, color);
        }
        TypedShape::Segment(seg) => {
            let a = pos + rot * parry_point(&seg.a);
            let b = pos + rot * parry_point(&seg.b);
            gizmos.line(a, b, color);
        }
        TypedShape::Triangle(tri) => {
            let a = pos + rot * parry_point(&tri.a);
            let b = pos + rot * parry_point(&tri.b);
            let c = pos + rot * parry_point(&tri.c);
            gizmos.line(a, b, color);
            gizmos.line(b, c, color);
            gizmos.line(c, a, color);
        }
        _ => {
            // Unknown shape type — draw a small marker
            gizmos.sphere(Isometry3d::new(pos, rot), 0.1, color);
        }
    }
}

fn draw_box_wireframe(
    gizmos: &mut Gizmos<ColliderGizmoGroup>,
    pos: Vec3,
    rot: Quat,
    half: Vec3,
    color: Color,
) {
    let corners = [
        Vec3::new(-half.x, -half.y, -half.z),
        Vec3::new(half.x, -half.y, -half.z),
        Vec3::new(half.x, half.y, -half.z),
        Vec3::new(-half.x, half.y, -half.z),
        Vec3::new(-half.x, -half.y, half.z),
        Vec3::new(half.x, -half.y, half.z),
        Vec3::new(half.x, half.y, half.z),
        Vec3::new(-half.x, half.y, half.z),
    ];
    let edges = [
        (0, 1), (1, 2), (2, 3), (3, 0),
        (4, 5), (5, 6), (6, 7), (7, 4),
        (0, 4), (1, 5), (2, 6), (3, 7),
    ];
    for (a, b) in edges {
        gizmos.line(pos + rot * corners[a], pos + rot * corners[b], color);
    }
}

fn draw_hierarchy_arrows(
    mut gizmos: Gizmos<ColliderGizmoGroup>,
    config: Res<PhysicsOverlayConfig>,
    selected_bodies: Query<(Entity, &GlobalTransform), (With<RigidBody>, With<Selected>)>,
    children_query: Query<&Children>,
    collider_transforms: Query<&GlobalTransform, With<ColliderConstructor>>,
    collider_check: Query<(), With<ColliderConstructor>>,
) {
    if !config.show_hierarchy_arrows {
        return;
    }

    for (body_entity, body_tf) in &selected_bodies {
        let body_pos = body_tf.translation();
        let mut descendant_colliders = bevy::ecs::entity::EntityHashSet::default();
        collect_descendant_colliders(
            body_entity,
            &children_query,
            &collider_check,
            &mut descendant_colliders,
        );

        for collider_entity in &descendant_colliders {
            if *collider_entity == body_entity {
                continue;
            }
            if let Ok(collider_tf) = collider_transforms.get(*collider_entity) {
                gizmos.arrow(body_pos, collider_tf.translation(), colors::COLLIDER_HIERARCHY_ARROW);
            }
        }
    }
}

fn collect_descendant_colliders(
    entity: Entity,
    children_query: &Query<&Children>,
    collider_check: &Query<(), With<ColliderConstructor>>,
    out: &mut bevy::ecs::entity::EntityHashSet,
) {
    if let Ok(children) = children_query.get(entity) {
        for child in children.iter() {
            if collider_check.contains(child) {
                out.insert(child);
            }
            collect_descendant_colliders(child, children_query, collider_check, out);
        }
    }
}
