mod csg;
mod geometry;
mod gizmo_overlay;
mod hull;
mod interaction;
pub(crate) mod mesh;

use bevy::prelude::*;

use crate::EditorMeta;
use crate::commands::EditorCommand;

pub use self::csg::{
    brush_planes_to_world, brushes_intersect, clean_degenerate_faces, subtract_brush,
};
pub use self::geometry::{compute_brush_geometry, compute_face_tangent_axes};
pub use self::hull::HullFace;
pub(crate) use self::hull::merge_hull_triangles;
pub(crate) use self::interaction::{
    BrushDragState, ClipState, EdgeDragState, VertexDragConstraint, VertexDragState,
};
pub use jackdaw_jsn::{Brush, BrushFaceData, BrushPlane};

/// Cached computed geometry (NOT serialized, rebuilt from Brush).
#[derive(Component)]
pub struct BrushMeshCache {
    pub vertices: Vec<Vec3>,
    /// Per-face: ordered vertex indices into `vertices`.
    pub face_polygons: Vec<Vec<usize>>,
    pub face_entities: Vec<Entity>,
}

/// Marker on child entities that render individual brush faces.
#[derive(Component)]
pub struct BrushFaceEntity {
    pub brush_entity: Entity,
    pub face_index: usize,
}

/// Marker: brush is being actively modified and should render with transparent preview materials.
#[derive(Component)]
pub struct BrushPreview;

/// Edit mode: Object (default) or brush editing.
#[derive(Resource, Default, PartialEq, Eq, Clone, Copy, Debug, Reflect)]
pub enum EditMode {
    #[default]
    Object,
    BrushEdit(BrushEditMode),
}

#[derive(PartialEq, Eq, Clone, Copy, Debug, Reflect)]
pub enum BrushEditMode {
    Face,
    Vertex,
    Edge,
    Clip,
}

/// Tracks selected sub-elements within brush edit mode.
#[derive(Resource, Default)]
pub struct BrushSelection {
    pub entity: Option<Entity>,
    pub faces: Vec<usize>,
    pub vertices: Vec<usize>,
    /// Selected edges as normalized (min, max) vertex index pairs.
    pub edges: Vec<(usize, usize)>,
    /// Remembered face from the last time face mode was exited (for extend-to-brush fallback).
    pub last_face_entity: Option<Entity>,
    pub last_face_index: Option<usize>,
}

/// Intent for face hover highlight color.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum HoverIntent {
    #[default]
    PushPull,
    Extend,
}

/// Tracks which brush face the cursor is hovering over.
#[derive(Resource, Default)]
pub struct BrushFaceHover {
    pub entity: Option<Entity>,
    pub face_index: Option<usize>,
    pub intent: HoverIntent,
}

/// Material palette for brush faces.
#[derive(Resource, Default)]
pub struct BrushMaterialPalette {
    pub materials: Vec<Handle<StandardMaterial>>,
    pub preview_materials: Vec<Handle<StandardMaterial>>,
    /// Grid-textured default material at 50% alpha (unselected).
    pub default_material: Handle<StandardMaterial>,
    /// Grid-textured default material at 90% alpha (selected).
    pub default_selected_material: Handle<StandardMaterial>,
    /// Grid-textured default material at 75% alpha (drag preview).
    pub default_preview_material: Handle<StandardMaterial>,
}

/// Remembers the last material applied via the texture/material browser, so new brushes inherit it.
#[derive(Resource, Default)]
pub struct LastUsedMaterial {
    pub material: Option<Handle<StandardMaterial>>,
}

pub struct SetBrush {
    pub entity: Entity,
    pub old: Brush,
    pub new: Brush,
    pub label: String,
}

impl EditorCommand for SetBrush {
    fn execute(&mut self, world: &mut World) {
        if let Some(mut brush) = world.get_mut::<Brush>(self.entity) {
            *brush = self.new.clone();
        }
        sync_brush_to_ast(world, self.entity, &self.new);
    }

    fn undo(&mut self, world: &mut World) {
        if let Some(mut brush) = world.get_mut::<Brush>(self.entity) {
            *brush = self.old.clone();
        }
        sync_brush_to_ast(world, self.entity, &self.old);
    }

    fn description(&self) -> &str {
        &self.label
    }
}

/// Serialize a Brush component to JSON and store it in the AST.
pub fn sync_brush_to_ast(world: &mut World, entity: Entity, brush: &Brush) {
    crate::commands::sync_component_to_ast(
        world,
        entity,
        "jackdaw_jsn::types::brush::Brush",
        brush,
    );
}

impl EditorMeta for Brush {
    fn category() -> &'static str {
        "Brush"
    }
}

pub struct BrushPlugin;

impl Plugin for BrushPlugin {
    fn build(&self, app: &mut App) {
        // Note: Brush, BrushFaceData, BrushPlane type registration is handled by JsnPlugin
        app.register_type_data::<Brush, crate::ReflectEditorMeta>()
            .register_type::<EditMode>()
            .register_type::<BrushEditMode>()
            .init_resource::<EditMode>()
            .init_resource::<BrushSelection>()
            .init_resource::<BrushMaterialPalette>()
            .init_resource::<BrushFaceHover>()
            .init_resource::<BrushDragState>()
            .init_resource::<VertexDragState>()
            .init_resource::<EdgeDragState>()
            .init_resource::<ClipState>()
            .init_resource::<LastUsedMaterial>()
            .add_systems(
                OnEnter(crate::AppState::Editor),
                mesh::setup_default_materials,
            )
            .add_systems(
                Update,
                (
                    interaction::handle_edit_mode_keys,
                    interaction::brush_face_hover,
                    interaction::brush_face_interact,
                    interaction::brush_vertex_interact,
                    interaction::brush_edge_interact,
                    interaction::handle_brush_delete,
                    interaction::handle_clip_mode,
                )
                    .chain()
                    .in_set(crate::EditorInteraction),
            )
            .add_systems(
                Update,
                (
                    mesh::sync_brush_preview,
                    ApplyDeferred,
                    mesh::regenerate_brush_meshes,
                    ApplyDeferred,
                    mesh::ensure_brush_face_materials,
                    gizmo_overlay::draw_brush_edit_gizmos,
                )
                    .chain()
                    .after(crate::EditorInteraction)
                    .run_if(in_state(crate::AppState::Editor)),
            );
    }
}
