use crate::EditorEntity;
use crate::brush::{Brush, BrushEditMode, BrushFaceData, BrushSelection, EditMode, SetBrush};
use crate::commands::CommandHistory;
use crate::selection::Selection;

use bevy::prelude::*;
use jackdaw_feathers::{
    text_edit::{self, TextEditCommitEvent, TextEditProps},
    tokens,
};

use super::{BrushFaceField, BrushFaceFieldBinding, BrushFacePropsContainer};

fn resolve_material_label(
    mat_handle: &Handle<StandardMaterial>,
    materials: &Assets<StandardMaterial>,
) -> String {
    if let Some(path) = mat_handle.path() {
        return path.to_string();
    }
    if let Some(mat) = materials.get(mat_handle) {
        if let Some(ref tex) = mat.base_color_texture {
            if let Some(path) = tex.path() {
                if let Some(filename) = path.path().file_name() {
                    return filename.to_string_lossy().to_string();
                }
            }
        }
    }
    format!("Material {:?}", mat_handle.id())
}

/// Apply the first selected face's material + UV settings to all faces of the brush.
#[derive(Event, Debug, Clone)]
pub(crate) struct ApplyTextureToAllFaces;

/// Apply a UV scale preset to all selected faces.
#[derive(Event, Debug, Clone)]
pub(crate) struct ApplyUvScalePreset(pub f32);

pub(super) fn spawn_brush_display(
    commands: &mut Commands,
    parent: Entity,
    brush: &crate::brush::Brush,
    materials: &Assets<StandardMaterial>,
) {
    let (vertices, face_polygons) = crate::brush::compute_brush_geometry(&brush.faces);
    let face_count = brush.faces.len();
    let vertex_count = vertices.len();
    let edge_count = {
        let mut edges = std::collections::HashSet::new();
        for polygon in &face_polygons {
            for i in 0..polygon.len() {
                let a = polygon[i];
                let b = polygon[(i + 1) % polygon.len()];
                let edge = if a < b { (a, b) } else { (b, a) };
                edges.insert(edge);
            }
        }
        edges.len()
    };

    let info = format!("{face_count} faces, {vertex_count} vertices, {edge_count} edges");
    commands.spawn((
        Text::new(info),
        TextFont {
            font_size: tokens::FONT_SM,
            ..Default::default()
        },
        TextColor(tokens::TEXT_SECONDARY),
        ChildOf(parent),
    ));

    // Material summary: shows unique materials used by this brush.
    spawn_material_summary(commands, parent, brush, materials);

    // Face properties container -- populated dynamically by update_brush_face_properties
    commands.spawn((
        BrushFacePropsContainer,
        EditorEntity,
        Node {
            flex_direction: FlexDirection::Column,
            width: Val::Percent(100.0),
            row_gap: px(tokens::SPACING_XS),
            ..Default::default()
        },
        ChildOf(parent),
    ));
}

/// Clear material from all faces of selected brushes (Object mode).
#[derive(Event, Debug, Clone)]
pub(crate) struct ClearMaterialFromBrush;

fn spawn_material_summary(
    commands: &mut Commands,
    parent: Entity,
    brush: &Brush,
    materials: &Assets<StandardMaterial>,
) {
    // Collect unique materials with face counts
    let mut material_counts: Vec<(Handle<StandardMaterial>, usize)> = Vec::new();
    for face in &brush.faces {
        if let Some(entry) = material_counts
            .iter_mut()
            .find(|(h, _)| *h == face.material)
        {
            entry.1 += 1;
        } else {
            material_counts.push((face.material.clone(), 1));
        }
    }

    let total_faces = brush.faces.len();
    let any_has_material = material_counts.iter().any(|(h, _)| *h != Handle::default());

    // Section header
    commands.spawn((
        Text::new("Materials & Textures"),
        TextFont {
            font_size: tokens::FONT_SM,
            ..Default::default()
        },
        TextColor(tokens::TEXT_SECONDARY),
        Node {
            margin: UiRect::top(Val::Px(tokens::SPACING_SM)),
            ..Default::default()
        },
        ChildOf(parent),
    ));

    for (mat_handle, count) in &material_counts {
        let is_default = *mat_handle == Handle::default();

        let row = commands
            .spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(tokens::SPACING_XS),
                    width: Val::Percent(100.0),
                    ..Default::default()
                },
                ChildOf(parent),
            ))
            .id();

        // Thumbnail
        if !is_default {
            if let Some(mat) = materials.get(mat_handle) {
                if let Some(ref tex) = mat.base_color_texture {
                    commands.spawn((
                        ImageNode::new(tex.clone()),
                        Node {
                            width: Val::Px(32.0),
                            height: Val::Px(32.0),
                            flex_shrink: 0.0,
                            ..Default::default()
                        },
                        ChildOf(row),
                    ));
                }
            }
        }

        // Material name
        let mat_label = if is_default {
            "No Material".to_string()
        } else {
            resolve_material_label(mat_handle, materials)
        };
        commands.spawn((
            Text::new(mat_label),
            TextFont {
                font_size: tokens::FONT_SM,
                ..Default::default()
            },
            TextColor(if is_default {
                tokens::TEXT_SECONDARY
            } else {
                tokens::TEXT_PRIMARY
            }),
            Node {
                flex_grow: 1.0,
                ..Default::default()
            },
            ChildOf(row),
        ));

        // Face count
        let count_text = if *count == total_faces {
            "(all faces)".to_string()
        } else {
            format!("({count} faces)")
        };
        commands.spawn((
            Text::new(count_text),
            TextFont {
                font_size: tokens::FONT_SM,
                ..Default::default()
            },
            TextColor(tokens::TEXT_SECONDARY),
            ChildOf(row),
        ));
    }

    // Clear All button, only if at least one face has a material.
    if any_has_material {
        let clear_all_btn = commands
            .spawn((
                Node {
                    padding: UiRect::axes(Val::Px(tokens::SPACING_SM), Val::Px(2.0)),
                    border_radius: BorderRadius::all(Val::Px(3.0)),
                    margin: UiRect::top(Val::Px(tokens::SPACING_XS)),
                    ..Default::default()
                },
                BackgroundColor(tokens::INPUT_BG),
                ChildOf(parent),
            ))
            .id();
        commands.spawn((
            Text::new("Clear All"),
            TextFont {
                font_size: tokens::FONT_SM,
                ..Default::default()
            },
            TextColor(tokens::TEXT_PRIMARY),
            ChildOf(clear_all_btn),
        ));
        commands
            .entity(clear_all_btn)
            .observe(|_: On<Pointer<Click>>, mut commands: Commands| {
                commands.trigger(ClearMaterialFromBrush);
            });
        commands.entity(clear_all_btn).observe(
            |hover: On<Pointer<Over>>, mut bg: Query<&mut BackgroundColor>| {
                if let Ok(mut bg) = bg.get_mut(hover.event_target()) {
                    bg.0 = tokens::HOVER_BG;
                }
            },
        );
        commands.entity(clear_all_btn).observe(
            |out: On<Pointer<Out>>, mut bg: Query<&mut BackgroundColor>| {
                if let Ok(mut bg) = bg.get_mut(out.event_target()) {
                    bg.0 = tokens::INPUT_BG;
                }
            },
        );
    }
}

/// Tracks the last state we rendered so we only rebuild on change.
#[derive(Default)]
pub(super) struct BrushFacePropsState {
    entity: Option<Entity>,
    faces: Vec<usize>,
    /// Hash of face data to detect UV edits
    data_hash: u64,
}

/// Clear material from currently selected brush faces.
#[derive(Event, Debug, Clone)]
pub(crate) struct ClearMaterialFromFaces;

fn hash_face_data(face: &BrushFaceData) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    // Hash the material handle id
    face.material.id().hash(&mut hasher);
    face.uv_offset.x.to_bits().hash(&mut hasher);
    face.uv_offset.y.to_bits().hash(&mut hasher);
    face.uv_scale.x.to_bits().hash(&mut hasher);
    face.uv_scale.y.to_bits().hash(&mut hasher);
    face.uv_rotation.to_bits().hash(&mut hasher);
    hasher.finish()
}

pub(crate) fn update_brush_face_properties(
    mut commands: Commands,
    edit_mode: Res<EditMode>,
    brush_selection: Res<BrushSelection>,
    brushes: Query<&Brush>,
    container_query: Query<(Entity, Option<&Children>), With<BrushFacePropsContainer>>,
    mut local_state: Local<BrushFacePropsState>,
    materials: Res<Assets<StandardMaterial>>,
) {
    let Ok((container_entity, container_children)) = container_query.single() else {
        return;
    };

    let show = *edit_mode == EditMode::BrushEdit(BrushEditMode::Face)
        && !brush_selection.faces.is_empty()
        && brush_selection.entity.is_some();

    if !show {
        // Clear if we had content
        if local_state.entity.is_some() {
            if let Some(children) = container_children {
                for child in children.iter() {
                    commands.entity(child).despawn();
                }
            }
            *local_state = BrushFacePropsState::default();
        }
        return;
    }

    let brush_entity = brush_selection.entity.unwrap();
    let Ok(brush) = brushes.get(brush_entity) else {
        return;
    };

    // Compute hash of selected face data
    let mut combined_hash = 0u64;
    for &fi in &brush_selection.faces {
        if fi < brush.faces.len() {
            combined_hash = combined_hash.wrapping_add(hash_face_data(&brush.faces[fi]));
        }
    }

    // Check if anything changed
    if local_state.entity == Some(brush_entity)
        && local_state.faces == brush_selection.faces
        && local_state.data_hash == combined_hash
    {
        return;
    }

    // Rebuild UI
    if let Some(children) = container_children {
        for child in children.iter() {
            commands.entity(child).despawn();
        }
    }

    local_state.entity = Some(brush_entity);
    local_state.faces = brush_selection.faces.clone();
    local_state.data_hash = combined_hash;

    // Use first selected face for display values
    let first_face_idx = brush_selection.faces[0];
    let face = &brush.faces[first_face_idx];
    let multi = brush_selection.faces.len() > 1;

    // Header
    let header_text = if multi {
        format!("{} faces selected", brush_selection.faces.len())
    } else {
        format!("Face {}", first_face_idx)
    };
    commands.spawn((
        Text::new(header_text),
        TextFont {
            font_size: tokens::FONT_SM,
            ..Default::default()
        },
        TextColor(tokens::TEXT_PRIMARY),
        Node {
            margin: UiRect::vertical(Val::Px(tokens::SPACING_XS)),
            ..Default::default()
        },
        ChildOf(container_entity),
    ));

    // Material info
    let has_material = face.material != Handle::default();
    if has_material {
        let mat_label = resolve_material_label(&face.material, &materials);

        let mat_row = commands
            .spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(tokens::SPACING_XS),
                    width: Val::Percent(100.0),
                    ..Default::default()
                },
                ChildOf(container_entity),
            ))
            .id();

        // Show base_color thumbnail if available
        if let Some(mat) = materials.get(&face.material) {
            if let Some(ref tex) = mat.base_color_texture {
                commands.spawn((
                    ImageNode::new(tex.clone()),
                    Node {
                        width: Val::Px(32.0),
                        height: Val::Px(32.0),
                        flex_shrink: 0.0,
                        ..Default::default()
                    },
                    ChildOf(mat_row),
                ));
            }
        }

        commands.spawn((
            Text::new(mat_label),
            TextFont {
                font_size: tokens::FONT_SM,
                ..Default::default()
            },
            TextColor(tokens::TEXT_SECONDARY),
            Node {
                flex_grow: 1.0,
                ..Default::default()
            },
            ChildOf(mat_row),
        ));

        // Clear material button
        let clear_mat_btn = commands
            .spawn((
                Node {
                    padding: UiRect::axes(Val::Px(tokens::SPACING_SM), Val::Px(2.0)),
                    border_radius: BorderRadius::all(Val::Px(3.0)),
                    ..Default::default()
                },
                BackgroundColor(tokens::INPUT_BG),
                ChildOf(mat_row),
            ))
            .id();
        commands.spawn((
            Text::new("Clear"),
            TextFont {
                font_size: tokens::FONT_SM,
                ..Default::default()
            },
            TextColor(tokens::TEXT_PRIMARY),
            ChildOf(clear_mat_btn),
        ));
        commands
            .entity(clear_mat_btn)
            .observe(|_: On<Pointer<Click>>, mut commands: Commands| {
                commands.trigger(ClearMaterialFromFaces);
            });
        commands.entity(clear_mat_btn).observe(
            |hover: On<Pointer<Over>>, mut bg: Query<&mut BackgroundColor>| {
                if let Ok(mut bg) = bg.get_mut(hover.event_target()) {
                    bg.0 = tokens::HOVER_BG;
                }
            },
        );
        commands.entity(clear_mat_btn).observe(
            |out: On<Pointer<Out>>, mut bg: Query<&mut BackgroundColor>| {
                if let Ok(mut bg) = bg.get_mut(out.event_target()) {
                    bg.0 = tokens::INPUT_BG;
                }
            },
        );

        // "Apply to All Faces" button
        let apply_all_btn = commands
            .spawn((
                Node {
                    padding: UiRect::axes(Val::Px(tokens::SPACING_SM), Val::Px(2.0)),
                    border_radius: BorderRadius::all(Val::Px(3.0)),
                    ..Default::default()
                },
                BackgroundColor(tokens::INPUT_BG),
                ChildOf(container_entity),
            ))
            .id();
        commands.spawn((
            Text::new("Apply to All Faces"),
            TextFont {
                font_size: tokens::FONT_SM,
                ..Default::default()
            },
            TextColor(tokens::TEXT_PRIMARY),
            ChildOf(apply_all_btn),
        ));
        commands
            .entity(apply_all_btn)
            .observe(|_: On<Pointer<Click>>, mut commands: Commands| {
                commands.trigger(ApplyTextureToAllFaces);
            });
        commands.entity(apply_all_btn).observe(
            |hover: On<Pointer<Over>>, mut bg: Query<&mut BackgroundColor>| {
                if let Ok(mut bg) = bg.get_mut(hover.event_target()) {
                    bg.0 = tokens::HOVER_BG;
                }
            },
        );
        commands.entity(apply_all_btn).observe(
            |out: On<Pointer<Out>>, mut bg: Query<&mut BackgroundColor>| {
                if let Ok(mut bg) = bg.get_mut(out.event_target()) {
                    bg.0 = tokens::INPUT_BG;
                }
            },
        );
    } else {
        commands.spawn((
            Text::new("No Material"),
            TextFont {
                font_size: tokens::FONT_SM,
                ..Default::default()
            },
            TextColor(tokens::TEXT_SECONDARY),
            ChildOf(container_entity),
        ));
    }

    // UV Offset
    spawn_brush_face_field_row(
        &mut commands,
        container_entity,
        "UV Offset",
        face.uv_offset.x as f64,
        face.uv_offset.y as f64,
        BrushFaceField::UvOffsetX,
        BrushFaceField::UvOffsetY,
        brush_entity,
    );

    // UV Scale
    spawn_brush_face_field_row(
        &mut commands,
        container_entity,
        "UV Scale",
        face.uv_scale.x as f64,
        face.uv_scale.y as f64,
        BrushFaceField::UvScaleX,
        BrushFaceField::UvScaleY,
        brush_entity,
    );

    // UV Scale preset buttons
    let preset_row = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                column_gap: px(tokens::SPACING_XS),
                width: Val::Percent(100.0),
                ..Default::default()
            },
            ChildOf(container_entity),
        ))
        .id();
    for preset in [0.25_f32, 0.5, 1.0, 2.0] {
        let label = if preset == 1.0 {
            "1x".to_string()
        } else {
            format!("{preset}x")
        };
        let btn = commands
            .spawn((
                Node {
                    padding: UiRect::axes(Val::Px(tokens::SPACING_SM), Val::Px(2.0)),
                    border_radius: BorderRadius::all(Val::Px(3.0)),
                    flex_grow: 1.0,
                    justify_content: JustifyContent::Center,
                    ..Default::default()
                },
                BackgroundColor(tokens::INPUT_BG),
                ChildOf(preset_row),
            ))
            .id();
        commands.spawn((
            Text::new(label),
            TextFont {
                font_size: tokens::FONT_SM,
                ..Default::default()
            },
            TextColor(tokens::TEXT_PRIMARY),
            ChildOf(btn),
        ));
        commands
            .entity(btn)
            .observe(move |_: On<Pointer<Click>>, mut commands: Commands| {
                commands.trigger(ApplyUvScalePreset(preset));
            });
        commands.entity(btn).observe(
            |hover: On<Pointer<Over>>, mut bg: Query<&mut BackgroundColor>| {
                if let Ok(mut bg) = bg.get_mut(hover.event_target()) {
                    bg.0 = tokens::HOVER_BG;
                }
            },
        );
        commands.entity(btn).observe(
            |out: On<Pointer<Out>>, mut bg: Query<&mut BackgroundColor>| {
                if let Ok(mut bg) = bg.get_mut(out.event_target()) {
                    bg.0 = tokens::INPUT_BG;
                }
            },
        );
    }

    // UV Rotation
    let rot_row = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(tokens::SPACING_XS),
                width: Val::Percent(100.0),
                ..Default::default()
            },
            ChildOf(container_entity),
        ))
        .id();

    commands.spawn((
        Text::new("Rotation"),
        TextFont {
            font_size: tokens::FONT_SM,
            ..Default::default()
        },
        TextColor(tokens::TEXT_SECONDARY),
        Node {
            min_width: px(60.0),
            flex_shrink: 0.0,
            ..Default::default()
        },
        ChildOf(rot_row),
    ));

    let rotation_degrees = face.uv_rotation.to_degrees() as f64;
    commands.spawn((
        text_edit::text_edit(
            TextEditProps::default()
                .numeric_f32()
                .grow()
                .with_default_value(rotation_degrees.to_string()),
        ),
        BrushFaceFieldBinding {
            field: BrushFaceField::UvRotation,
        },
        ChildOf(rot_row),
    ));
}

fn spawn_brush_face_field_row(
    commands: &mut Commands,
    parent: Entity,
    label: &str,
    x_value: f64,
    y_value: f64,
    x_field: BrushFaceField,
    y_field: BrushFaceField,
    _brush_entity: Entity,
) {
    let row = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(tokens::SPACING_XS),
                width: Val::Percent(100.0),
                ..Default::default()
            },
            ChildOf(parent),
        ))
        .id();

    commands.spawn((
        Text::new(label),
        TextFont {
            font_size: tokens::FONT_SM,
            ..Default::default()
        },
        TextColor(tokens::TEXT_SECONDARY),
        Node {
            min_width: px(60.0),
            flex_shrink: 0.0,
            ..Default::default()
        },
        ChildOf(row),
    ));

    // X input
    commands.spawn((
        text_edit::text_edit(
            TextEditProps::default()
                .numeric_f32()
                .grow()
                .with_default_value(x_value.to_string()),
        ),
        BrushFaceFieldBinding { field: x_field },
        ChildOf(row),
    ));

    // Y input
    commands.spawn((
        text_edit::text_edit(
            TextEditProps::default()
                .numeric_f32()
                .grow()
                .with_default_value(y_value.to_string()),
        ),
        BrushFaceFieldBinding { field: y_field },
        ChildOf(row),
    ));
}

/// Handle TextEditCommitEvent for brush face field bindings.
pub(crate) fn on_brush_face_text_commit(
    event: On<TextEditCommitEvent>,
    bindings: Query<&BrushFaceFieldBinding>,
    child_of_query: Query<&ChildOf>,
    brush_selection: Res<BrushSelection>,
    mut brushes: Query<&mut Brush>,
    mut history: ResMut<CommandHistory>,
) {
    // Walk up from the committed entity to find a BrushFaceFieldBinding
    let mut current = event.entity;
    for _ in 0..4 {
        let Ok(child_of) = child_of_query.get(current) else {
            break;
        };
        if let Ok(binding) = bindings.get(child_of.parent()) {
            let value: f64 = event.text.parse().unwrap_or(0.0);
            apply_brush_face_field(
                binding.field,
                value,
                &brush_selection,
                &mut brushes,
                &mut history,
            );
            return;
        }
        current = child_of.parent();
    }
}

fn apply_brush_face_field(
    field: BrushFaceField,
    value: f64,
    brush_selection: &BrushSelection,
    brushes: &mut Query<&mut Brush>,
    history: &mut CommandHistory,
) {
    let Some(brush_entity) = brush_selection.entity else {
        return;
    };
    let Ok(mut brush) = brushes.get_mut(brush_entity) else {
        return;
    };

    let old = brush.clone();
    for &face_idx in &brush_selection.faces {
        if face_idx >= brush.faces.len() {
            continue;
        }
        let face = &mut brush.faces[face_idx];
        match field {
            BrushFaceField::UvOffsetX => face.uv_offset.x = value as f32,
            BrushFaceField::UvOffsetY => face.uv_offset.y = value as f32,
            BrushFaceField::UvScaleX => face.uv_scale.x = value as f32,
            BrushFaceField::UvScaleY => face.uv_scale.y = value as f32,
            BrushFaceField::UvRotation => face.uv_rotation = (value as f32).to_radians(),
        }
    }

    let cmd = SetBrush {
        entity: brush_entity,
        old,
        new: brush.clone(),
        label: "Edit face UV".to_string(),
    };
    history.push_executed(Box::new(cmd));
}

pub(crate) fn handle_clear_material(
    _event: On<ClearMaterialFromFaces>,
    brush_selection: Res<BrushSelection>,
    edit_mode: Res<EditMode>,
    mut brushes: Query<&mut Brush>,
    mut history: ResMut<CommandHistory>,
    mut commands: Commands,
) {
    if *edit_mode != EditMode::BrushEdit(BrushEditMode::Face) {
        return;
    }
    let Some(brush_entity) = brush_selection.entity else {
        return;
    };
    if brush_selection.faces.is_empty() {
        return;
    }
    let Ok(mut brush) = brushes.get_mut(brush_entity) else {
        return;
    };

    let old = brush.clone();
    for &face_idx in &brush_selection.faces {
        if face_idx < brush.faces.len() {
            brush.faces[face_idx].material = Handle::default();
        }
    }

    let cmd = SetBrush {
        entity: brush_entity,
        old,
        new: brush.clone(),
        label: "Clear material".to_string(),
    };
    history.push_executed(Box::new(cmd));
    commands.entity(brush_entity).insert(super::InspectorDirty);
}

pub(crate) fn handle_clear_texture(
    _event: On<crate::asset_browser::ClearTextureFromFaces>,
    brush_selection: Res<BrushSelection>,
    edit_mode: Res<EditMode>,
    mut brushes: Query<&mut Brush>,
    mut history: ResMut<CommandHistory>,
    mut commands: Commands,
) {
    if *edit_mode != EditMode::BrushEdit(BrushEditMode::Face) {
        return;
    }
    let Some(brush_entity) = brush_selection.entity else {
        return;
    };
    if brush_selection.faces.is_empty() {
        return;
    }
    let Ok(mut brush) = brushes.get_mut(brush_entity) else {
        return;
    };

    let old = brush.clone();
    for &face_idx in &brush_selection.faces {
        if face_idx < brush.faces.len() {
            brush.faces[face_idx].material = Handle::default();
        }
    }

    let cmd = SetBrush {
        entity: brush_entity,
        old,
        new: brush.clone(),
        label: "Clear texture".to_string(),
    };
    history.push_executed(Box::new(cmd));
    commands.entity(brush_entity).insert(super::InspectorDirty);
}

pub(crate) fn handle_apply_texture_to_all(
    _event: On<ApplyTextureToAllFaces>,
    brush_selection: Res<BrushSelection>,
    edit_mode: Res<EditMode>,
    mut brushes: Query<&mut Brush>,
    mut history: ResMut<CommandHistory>,
    mut commands: Commands,
) {
    if *edit_mode != EditMode::BrushEdit(BrushEditMode::Face) {
        return;
    }
    let Some(brush_entity) = brush_selection.entity else {
        return;
    };
    if brush_selection.faces.is_empty() {
        return;
    }
    let Ok(mut brush) = brushes.get_mut(brush_entity) else {
        return;
    };

    let source_idx = brush_selection.faces[0];
    if source_idx >= brush.faces.len() {
        return;
    }
    let source = brush.faces[source_idx].clone();

    let old = brush.clone();
    for face in &mut brush.faces {
        face.material = source.material.clone();
        face.uv_scale = source.uv_scale;
        face.uv_offset = source.uv_offset;
        face.uv_rotation = source.uv_rotation;
    }

    let cmd = SetBrush {
        entity: brush_entity,
        old,
        new: brush.clone(),
        label: "Apply material to all faces".to_string(),
    };
    history.push_executed(Box::new(cmd));
    commands.entity(brush_entity).insert(super::InspectorDirty);
}

pub(crate) fn handle_uv_scale_preset(
    event: On<ApplyUvScalePreset>,
    brush_selection: Res<BrushSelection>,
    edit_mode: Res<EditMode>,
    mut brushes: Query<&mut Brush>,
    mut history: ResMut<CommandHistory>,
) {
    if *edit_mode != EditMode::BrushEdit(BrushEditMode::Face) {
        return;
    }
    let Some(brush_entity) = brush_selection.entity else {
        return;
    };
    if brush_selection.faces.is_empty() {
        return;
    }
    let Ok(mut brush) = brushes.get_mut(brush_entity) else {
        return;
    };

    let old = brush.clone();
    let scale = Vec2::splat(event.0);
    for &face_idx in &brush_selection.faces {
        if face_idx < brush.faces.len() {
            brush.faces[face_idx].uv_scale = scale;
        }
    }

    let cmd = SetBrush {
        entity: brush_entity,
        old,
        new: brush.clone(),
        label: "Set UV scale preset".to_string(),
    };
    history.push_executed(Box::new(cmd));
}

pub(crate) fn handle_clear_material_from_brush(
    _event: On<ClearMaterialFromBrush>,
    selection: Res<Selection>,
    mut brushes: Query<&mut Brush>,
    mut history: ResMut<CommandHistory>,
    brush_groups: Query<(), With<jackdaw_jsn::types::BrushGroup>>,
    children_query: Query<&Children>,
    mut commands: Commands,
) {
    // Expand BrushGroups into child brushes
    let targets: Vec<Entity> = selection
        .entities
        .iter()
        .flat_map(|&e| {
            if brush_groups.contains(e) {
                children_query
                    .get(e)
                    .map(|c| c.iter().collect::<Vec<_>>())
                    .unwrap_or_default()
            } else {
                vec![e]
            }
        })
        .collect();

    let mut group_commands: Vec<Box<dyn jackdaw_commands::EditorCommand>> = Vec::new();
    for entity in targets {
        if let Ok(mut brush) = brushes.get_mut(entity) {
            let has_any_material = brush.faces.iter().any(|f| f.material != Handle::default());
            if !has_any_material {
                continue;
            }
            let old = brush.clone();
            for face in brush.faces.iter_mut() {
                face.material = Handle::default();
            }
            let cmd = SetBrush {
                entity,
                old,
                new: brush.clone(),
                label: "Clear all materials".to_string(),
            };
            group_commands.push(Box::new(cmd));
            commands.entity(entity).insert(super::InspectorDirty);
        }
    }
    if !group_commands.is_empty() {
        history.push_executed(Box::new(jackdaw_commands::CommandGroup {
            commands: group_commands,
            label: "Clear all materials".to_string(),
        }));
    }
}
