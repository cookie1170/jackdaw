use std::path::Path;

use bevy::{ecs::system::SystemState, gltf::GltfAssetLabel, prelude::*};

use crate::{
    EditorEntity,
    commands::{CommandHistory, DespawnEntity, EditorCommand},
    selection::{Selected, Selection},
};
use bevy::input_focus::InputFocus;

/// System clipboard for copy/paste of entities as JSN text.
/// On Linux/X11 the clipboard is ownership-based: data is only available while
/// the Clipboard instance is alive. Storing as a Bevy Resource keeps it alive.
#[derive(Resource)]
pub struct SystemClipboard {
    clipboard: arboard::Clipboard,
    /// Fallback: last copied JSN text, in case system clipboard read fails.
    last_jsn: String,
}

impl Default for SystemClipboard {
    fn default() -> Self {
        Self {
            clipboard: arboard::Clipboard::new().expect("Failed to init system clipboard"),
            last_jsn: String::new(),
        }
    }
}

// Re-export from jackdaw_jsn
pub use jackdaw_jsn::GltfSource;

pub struct EntityOpsPlugin;

impl Plugin for EntityOpsPlugin {
    fn build(&self, app: &mut App) {
        // Note: GltfSource type registration is handled by JsnPlugin
        match arboard::Clipboard::new() {
            Ok(clipboard) => {
                app.insert_resource(SystemClipboard {
                    clipboard,
                    last_jsn: String::new(),
                });
            }
            Err(e) => {
                warn!("Failed to initialize system clipboard: {e}");
            }
        }
        app.add_systems(Update, handle_entity_keys.in_set(crate::EditorInteraction));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityTemplate {
    Empty,
    Cube,
    Sphere,
    PointLight,
    DirectionalLight,
    SpotLight,
    Camera3d,
}

impl EntityTemplate {
    pub fn label(self) -> &'static str {
        match self {
            Self::Empty => "Empty Entity",
            Self::Cube => "Cube",
            Self::Sphere => "Sphere",
            Self::PointLight => "Point Light",
            Self::DirectionalLight => "Directional Light",
            Self::SpotLight => "Spot Light",
            Self::Camera3d => "Camera",
        }
    }
}

pub fn create_entity(
    commands: &mut Commands,
    template: EntityTemplate,
    selection: &mut Selection,
) -> Entity {
    let entity = match template {
        EntityTemplate::Empty => commands
            .spawn((Name::new("Empty"), Transform::default()))
            .id(),
        EntityTemplate::Cube => {
            let id = commands
                .spawn((
                    Name::new("Cube"),
                    crate::brush::Brush::cuboid(0.5, 0.5, 0.5),
                    Transform::default(),
                    Visibility::default(),
                ))
                .id();
            commands.queue(apply_last_material(id));
            id
        }
        EntityTemplate::Sphere => {
            let id = commands
                .spawn((
                    Name::new("Sphere"),
                    crate::brush::Brush::sphere(0.5),
                    Transform::default(),
                    Visibility::default(),
                ))
                .id();
            commands.queue(apply_last_material(id));
            id
        }
        EntityTemplate::PointLight => commands
            .spawn((
                Name::new("Point Light"),
                PointLight {
                    shadows_enabled: true,
                    ..default()
                },
                Transform::from_xyz(0.0, 3.0, 0.0),
            ))
            .id(),
        EntityTemplate::DirectionalLight => commands
            .spawn((
                Name::new("Directional Light"),
                DirectionalLight {
                    shadows_enabled: true,
                    ..default()
                },
                Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.8, 0.4, 0.0)),
            ))
            .id(),
        EntityTemplate::SpotLight => commands
            .spawn((
                Name::new("Spot Light"),
                SpotLight {
                    shadows_enabled: true,
                    ..default()
                },
                Transform::from_xyz(0.0, 3.0, 0.0).looking_at(Vec3::ZERO, Vec3::Y),
            ))
            .id(),
        EntityTemplate::Camera3d => commands
            .spawn((
                Name::new("Camera"),
                Camera3d::default(),
                Camera {
                    // Scene cameras are authored inactive so they don't
                    // render over the editor viewport. They become active
                    // at play time (or via a future "preview through this
                    // camera" operator).
                    is_active: false,
                    ..default()
                },
                bevy::camera::RenderTarget::None {
                    size: UVec2::splat(1),
                },
                Transform::from_xyz(0.0, 2.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
            ))
            .id(),
    };

    selection.select_single(commands, entity);
    entity
}

/// Returns a command that applies the last-used material to all faces of a brush entity.
fn apply_last_material(entity: Entity) -> impl FnOnce(&mut World) {
    move |world: &mut World| {
        let last_mat = world
            .resource::<crate::brush::LastUsedMaterial>()
            .material
            .clone();
        if let Some(mat) = last_mat {
            if let Some(mut brush) = world.get_mut::<crate::brush::Brush>(entity) {
                for face in &mut brush.faces {
                    face.material = mat.clone();
                }
            }
        }
    }
}

/// World-access version of `create_entity`. Used from menu actions and other deferred contexts.
/// Pushes a `SpawnEntity` command so the addition can be undone.
pub fn create_entity_in_world(world: &mut World, template: EntityTemplate) {
    let label = format!("Add {}", template.label());
    let spawn_fn = Box::new(move |world: &mut World| -> Entity {
        let mut system_state: SystemState<(Commands, ResMut<Selection>)> = SystemState::new(world);
        let (mut commands, mut selection) = system_state.get_mut(world);
        let entity = create_entity(&mut commands, template, &mut selection);
        system_state.apply(world);
        crate::scene_io::register_entity_in_ast(world, entity);
        entity
    });

    let mut cmd: Box<dyn EditorCommand> = Box::new(crate::commands::SpawnEntity {
        spawned: None,
        spawn_fn,
        label,
    });
    cmd.execute(world);
    world.resource_mut::<CommandHistory>().push_executed(cmd);
}

pub fn spawn_gltf(
    commands: &mut Commands,
    asset_server: &AssetServer,
    path: &str,
    position: Vec3,
    selection: &mut Selection,
) -> Entity {
    let file_name = Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "GLTF Model".to_string());
    let scene_index = 0;
    let asset_path = to_asset_path(path);
    let scene = asset_server.load(GltfAssetLabel::Scene(scene_index).from_asset(asset_path));
    let entity = commands
        .spawn((
            Name::new(file_name),
            GltfSource {
                path: path.to_string(),
                scene_index,
            },
            SceneRoot(scene),
            Transform::from_translation(position),
        ))
        .id();
    selection.select_single(commands, entity);
    entity
}

pub fn spawn_gltf_in_world(world: &mut World, path: &str, position: Vec3) {
    let mut system_state: SystemState<(Commands, Res<AssetServer>, ResMut<Selection>)> =
        SystemState::new(world);
    let (mut commands, asset_server, mut selection) = system_state.get_mut(world);
    spawn_gltf(&mut commands, &asset_server, path, position, &mut selection);
    system_state.apply(world);
}

pub fn delete_selected(world: &mut World) {
    let selection = world.resource::<Selection>();
    let entities: Vec<Entity> = selection.entities.clone();

    if entities.is_empty() {
        return;
    }

    // Build commands for each entity
    let mut cmds: Vec<Box<dyn EditorCommand>> = Vec::new();
    for &entity in &entities {
        if world.get_entity(entity).is_err() {
            continue;
        }
        if world.get::<EditorEntity>(entity).is_some() {
            continue;
        }
        cmds.push(Box::new(DespawnEntity::from_world(world, entity)));
    }

    // Deselect entities before despawning so that `On<Remove, Selected>`
    // observers can clean up tree-row UI while the entities still exist.
    for &entity in &entities {
        if let Ok(mut ec) = world.get_entity_mut(entity) {
            ec.remove::<Selected>();
        }
    }
    let mut selection = world.resource_mut::<Selection>();
    selection.entities.clear();

    // Execute all despawn commands
    for cmd in &mut cmds {
        cmd.execute(world);
    }

    // Push as a single group command
    if !cmds.is_empty() {
        let group = crate::commands::CommandGroup {
            commands: cmds,
            label: "Delete entities".to_string(),
        };
        let mut history = world.resource_mut::<CommandHistory>();
        history.push_executed(Box::new(group));
    }
}

pub fn duplicate_selected(world: &mut World) {
    let selection = world.resource::<Selection>();
    let entities: Vec<Entity> = selection.entities.clone();

    if entities.is_empty() {
        return;
    }

    // Deselect current entities first
    for &entity in &entities {
        if let Ok(mut ec) = world.get_entity_mut(entity) {
            ec.remove::<Selected>();
        }
    }

    let mut new_entities = Vec::new();

    for &entity in &entities {
        if world.get_entity(entity).is_err() {
            continue;
        }
        if world.get::<EditorEntity>(entity).is_some() {
            continue;
        }

        // Snapshot the entity (and descendants) via DynamicSceneBuilder
        let mut snapshot_entities = Vec::new();
        crate::commands::collect_entity_ids(world, entity, &mut snapshot_entities);
        let scene = DynamicSceneBuilder::from_world(world)
            .extract_entities(snapshot_entities.into_iter())
            .build();

        // Write the snapshot back to create a clone
        let mut entity_map = Default::default();
        if scene.write_to_world(world, &mut entity_map).is_err() {
            continue;
        }

        // Find the cloned root entity
        let Some(&new_root) = entity_map.get(&entity) else {
            continue;
        };

        // Rename with incremented number suffix
        if let Some(name) = world.get::<Name>(new_root) {
            // Strip trailing " (Copy)" chains and trailing " N" to find base name
            let mut base = name.as_str().to_string();
            while base.ends_with(" (Copy)") {
                base.truncate(base.len() - 7);
            }
            if let Some(pos) = base.rfind(' ') {
                if base[pos + 1..].parse::<u32>().is_ok() {
                    base.truncate(pos);
                }
            }

            // Find highest existing number for this base name
            let mut max_num = 0u32;
            let mut query = world.query::<&Name>();
            for existing in query.iter(world) {
                let s = existing.as_str();
                if s == base {
                    max_num = max_num.max(1);
                } else if let Some(rest) = s.strip_prefix(base.as_str()) {
                    if let Some(num_str) = rest.strip_prefix(' ') {
                        if let Ok(n) = num_str.parse::<u32>() {
                            max_num = max_num.max(n);
                        }
                    }
                }
            }

            let new_name = format!("{} {}", base, max_num + 1);
            world.entity_mut(new_root).insert(Name::new(new_name));
        }

        // Preserve parent relationship from original
        let parent = world.get::<ChildOf>(entity).map(|c| c.0);
        if let Some(parent) = parent {
            world.entity_mut(new_root).insert(ChildOf(parent));
        } else {
            // Original was a root entity, remove any ChildOf the scene write may have added.
            world.entity_mut(new_root).remove::<ChildOf>();
        }

        new_entities.push(new_root);
    }

    // Register duplicates in AST
    crate::scene_io::register_entities_in_ast(world, &new_entities);

    // Select the new entities
    let mut selection = world.resource_mut::<Selection>();
    selection.entities = new_entities;
    for &entity in &selection.entities.clone() {
        world.entity_mut(entity).insert(Selected);
    }
}

/// Snap a vector to the nearest cardinal world axis (±X, ±Y, ±Z).
/// Returns a signed unit vector along the axis with the largest absolute component.
fn snap_to_nearest_axis(v: Vec3) -> Vec3 {
    let abs = v.abs();
    if abs.x >= abs.y && abs.x >= abs.z {
        Vec3::new(v.x.signum(), 0.0, 0.0)
    } else if abs.y >= abs.x && abs.y >= abs.z {
        Vec3::new(0.0, v.y.signum(), 0.0)
    } else {
        Vec3::new(0.0, 0.0, v.z.signum())
    }
}

/// Derive TrenchBroom-style rotation axes from the camera transform.
///
/// - **Yaw** (left/right arrows): always world Y. Vertical rotation is always intuitive.
/// - **Roll** (up/down arrows): camera forward projected to horizontal, snapped to nearest
///   world axis, then negated. This is the axis you're "looking along".
/// - **Pitch** (PageUp/PageDown): camera right snapped to nearest world axis. If it
///   collides with the roll axis, use the cross product with Y instead.
fn camera_snapped_rotation_axes(gt: &GlobalTransform) -> (Vec3, Vec3, Vec3) {
    let yaw_axis = Vec3::Y;

    // Forward projected onto the horizontal plane, snapped to nearest axis
    let fwd = gt.forward().as_vec3();
    let fwd_horiz = Vec3::new(fwd.x, 0.0, fwd.z);
    let roll_axis = if fwd_horiz.length_squared() > 1e-6 {
        -snap_to_nearest_axis(fwd_horiz)
    } else {
        // Looking straight down/up, use camera up projected horizontally instead.
        let up = gt.up().as_vec3();
        let up_horiz = Vec3::new(up.x, 0.0, up.z);
        if up_horiz.length_squared() > 1e-6 {
            snap_to_nearest_axis(up_horiz)
        } else {
            Vec3::NEG_Z
        }
    };

    // Right snapped to nearest axis, with deduplication against roll
    let right = gt.right().as_vec3();
    let mut pitch_axis = snap_to_nearest_axis(right);
    if pitch_axis.abs() == roll_axis.abs() {
        // Collision, derive perpendicular horizontal axis.
        pitch_axis = snap_to_nearest_axis(yaw_axis.cross(roll_axis));
    }

    (yaw_axis, roll_axis, pitch_axis)
}

pub fn handle_entity_keys(world: &mut World) {
    // Don't process entity keys when a text input is focused
    let has_input_focus = world.resource::<InputFocus>().0.is_some();
    if has_input_focus {
        return;
    }

    // Don't process entity keys during modal transform operations or draw mode
    let modal_active = world
        .resource::<crate::modal_transform::ModalTransformState>()
        .active
        .is_some();
    if modal_active {
        return;
    }
    let draw_active = world
        .resource::<crate::draw_brush::DrawBrushState>()
        .active
        .is_some();
    if draw_active {
        return;
    }

    // Don't process entity ops during brush edit mode (Delete etc. handled by brush systems)
    let in_brush_edit = !matches!(
        *world.resource::<crate::brush::EditMode>(),
        crate::brush::EditMode::Object
    );
    if in_brush_edit {
        return;
    }

    use crate::keybinds::EditorAction;

    let keyboard = world.resource::<ButtonInput<KeyCode>>();
    let keybinds = world.resource::<crate::keybinds::KeybindRegistry>();

    let delete = keybinds.just_pressed(EditorAction::Delete, keyboard);
    let duplicate = keybinds.just_pressed(EditorAction::Duplicate, keyboard);
    let copy = keybinds.just_pressed(EditorAction::CopyComponents, keyboard);
    let paste = keybinds.just_pressed(EditorAction::PasteComponents, keyboard);
    let reset_pos = keybinds.just_pressed(EditorAction::ResetPosition, keyboard);
    let reset_rot = keybinds.just_pressed(EditorAction::ResetRotation, keyboard);
    let reset_scale = keybinds.just_pressed(EditorAction::ResetScale, keyboard);
    let do_hide_selected = keybinds.just_pressed(EditorAction::ToggleVisibility, keyboard);
    let unhide_all = keybinds.just_pressed(EditorAction::UnhideAll, keyboard);
    let hide_unselected = keybinds.just_pressed(EditorAction::HideAll, keyboard);

    // Rotations (Alt+Arrow/PageUp/Down)
    let rot_left = keybinds.just_pressed(EditorAction::Rotate90Left, keyboard);
    let rot_right = keybinds.just_pressed(EditorAction::Rotate90Right, keyboard);
    let rot_up = keybinds.just_pressed(EditorAction::Rotate90Up, keyboard);
    let rot_down = keybinds.just_pressed(EditorAction::Rotate90Down, keyboard);
    let roll_left = keybinds.just_pressed(EditorAction::Roll90Left, keyboard);
    let roll_right = keybinds.just_pressed(EditorAction::Roll90Right, keyboard);
    let any_rotation = rot_left || rot_right || rot_up || rot_down || roll_left || roll_right;

    // Nudge: use key_just_pressed since Ctrl+arrow is also valid (duplicate+nudge).
    let ctrl = keyboard.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    let alt = keyboard.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]);
    let nudge_left = keybinds.key_just_pressed(EditorAction::NudgeLeft, keyboard) && !alt;
    let nudge_right = keybinds.key_just_pressed(EditorAction::NudgeRight, keyboard) && !alt;
    let nudge_fwd = keybinds.key_just_pressed(EditorAction::NudgeForward, keyboard) && !alt;
    let nudge_back = keybinds.key_just_pressed(EditorAction::NudgeBack, keyboard) && !alt;
    let nudge_up = keybinds.key_just_pressed(EditorAction::NudgeUp, keyboard) && !alt;
    let nudge_down = keybinds.key_just_pressed(EditorAction::NudgeDown, keyboard) && !alt;
    let any_nudge = nudge_left || nudge_right || nudge_fwd || nudge_back || nudge_up || nudge_down;

    if delete {
        delete_selected(world);
    } else if duplicate {
        duplicate_selected(world);
    } else if copy {
        copy_components(world);
    } else if paste {
        paste_components(world);
    } else if reset_pos {
        reset_transform_selected(world, TransformReset::Position);
    } else if reset_rot {
        reset_transform_selected(world, TransformReset::Rotation);
    } else if reset_scale {
        reset_transform_selected(world, TransformReset::Scale);
    } else if unhide_all {
        unhide_all_entities(world);
    } else if hide_unselected {
        hide_all_entities(world);
    } else if do_hide_selected {
        hide_selected(world);
    } else if any_rotation {
        // TrenchBroom-style rotation: snap camera axes to the nearest world axis
        // so rotations always produce axis-aligned results while still feeling
        // intuitive from the current viewpoint.
        let (yaw_axis, roll_axis, pitch_axis) = {
            let mut cam_query = world
                .query_filtered::<&GlobalTransform, With<crate::viewport::MainViewportCamera>>();
            cam_query
                .iter(world)
                .next()
                .map(|gt| camera_snapped_rotation_axes(gt))
                .unwrap_or((Vec3::Y, Vec3::NEG_Z, Vec3::X))
        };

        let angle = std::f32::consts::FRAC_PI_2;
        let rotation = if rot_left {
            Quat::from_axis_angle(yaw_axis, -angle)
        } else if rot_right {
            Quat::from_axis_angle(yaw_axis, angle)
        } else if rot_up {
            Quat::from_axis_angle(roll_axis, -angle)
        } else if rot_down {
            Quat::from_axis_angle(roll_axis, angle)
        } else if roll_left {
            Quat::from_axis_angle(pitch_axis, angle)
        } else {
            // roll_right
            Quat::from_axis_angle(pitch_axis, -angle)
        };
        rotate_selected(world, rotation);
    } else if any_nudge {
        let grid_size = world
            .resource::<crate::snapping::SnapSettings>()
            .grid_size();
        let offset = if nudge_left {
            Vec3::new(-grid_size, 0.0, 0.0)
        } else if nudge_right {
            Vec3::new(grid_size, 0.0, 0.0)
        } else if nudge_fwd {
            Vec3::new(0.0, 0.0, -grid_size)
        } else if nudge_back {
            Vec3::new(0.0, 0.0, grid_size)
        } else if nudge_up {
            Vec3::new(0.0, grid_size, 0.0)
        } else {
            // nudge_down
            Vec3::new(0.0, -grid_size, 0.0)
        };

        if ctrl {
            // Ctrl+arrow: duplicate then nudge
            duplicate_selected(world);
        }
        nudge_selected(world, offset);
    }
}

enum TransformReset {
    Position,
    Rotation,
    Scale,
}

fn reset_transform_selected(world: &mut World, reset: TransformReset) {
    let selection = world.resource::<Selection>();
    let entities: Vec<Entity> = selection.entities.clone();

    if entities.is_empty() {
        return;
    }

    let mut cmds: Vec<Box<dyn EditorCommand>> = Vec::new();

    for &entity in &entities {
        if world.get_entity(entity).is_err() {
            continue;
        }
        let Some(&old_transform) = world.get::<Transform>(entity) else {
            continue;
        };

        let new_transform = match reset {
            TransformReset::Position => Transform {
                translation: Vec3::ZERO,
                ..old_transform
            },
            TransformReset::Rotation => Transform {
                rotation: Quat::IDENTITY,
                ..old_transform
            },
            TransformReset::Scale => Transform {
                scale: Vec3::ONE,
                ..old_transform
            },
        };

        if old_transform == new_transform {
            continue;
        }

        let mut cmd = crate::commands::SetTransform {
            entity,
            old_transform,
            new_transform,
        };
        cmd.execute(world);
        cmds.push(Box::new(cmd));
    }

    if !cmds.is_empty() {
        let label = match reset {
            TransformReset::Position => "Reset position",
            TransformReset::Rotation => "Reset rotation",
            TransformReset::Scale => "Reset scale",
        };
        let group = crate::commands::CommandGroup {
            commands: cmds,
            label: label.to_string(),
        };
        let mut history = world.resource_mut::<CommandHistory>();
        history.push_executed(Box::new(group));
    }
}

fn nudge_selected(world: &mut World, offset: Vec3) {
    let selection = world.resource::<Selection>();
    let entities: Vec<Entity> = selection.entities.clone();

    if entities.is_empty() {
        return;
    }

    let mut cmds: Vec<Box<dyn EditorCommand>> = Vec::new();

    for &entity in &entities {
        if world.get_entity(entity).is_err() {
            continue;
        }
        let Some(&old_transform) = world.get::<Transform>(entity) else {
            continue;
        };

        let new_transform = Transform {
            translation: old_transform.translation + offset,
            ..old_transform
        };

        let mut cmd = crate::commands::SetTransform {
            entity,
            old_transform,
            new_transform,
        };
        cmd.execute(world);
        cmds.push(Box::new(cmd));
    }

    if !cmds.is_empty() {
        let group = crate::commands::CommandGroup {
            commands: cmds,
            label: "Nudge".to_string(),
        };
        let mut history = world.resource_mut::<CommandHistory>();
        history.push_executed(Box::new(group));
    }
}

fn rotate_selected(world: &mut World, rotation: Quat) {
    let selection = world.resource::<Selection>();
    let entities: Vec<Entity> = selection.entities.clone();

    if entities.is_empty() {
        return;
    }

    let mut cmds: Vec<Box<dyn EditorCommand>> = Vec::new();

    for &entity in &entities {
        if world.get_entity(entity).is_err() {
            continue;
        }
        let Some(&old_transform) = world.get::<Transform>(entity) else {
            continue;
        };

        let new_transform = Transform {
            rotation: rotation * old_transform.rotation,
            ..old_transform
        };

        let mut cmd = crate::commands::SetTransform {
            entity,
            old_transform,
            new_transform,
        };
        cmd.execute(world);
        cmds.push(Box::new(cmd));
    }

    if !cmds.is_empty() {
        let group = crate::commands::CommandGroup {
            commands: cmds,
            label: "Rotate 90\u{00b0}".to_string(),
        };
        let mut history = world.resource_mut::<CommandHistory>();
        history.push_executed(Box::new(group));
    }
}

/// Copy all reflected components from the primary selected entity to the clipboard.
/// Copy selected entities to the system clipboard as JSN text.
fn copy_components(world: &mut World) {
    let selection = world.resource::<Selection>();
    if selection.entities.is_empty() {
        return;
    }

    let ast = world.resource::<jackdaw_jsn::SceneJsnAst>();
    let jsn_entities: Vec<jackdaw_jsn::format::JsnEntity> = selection
        .entities
        .iter()
        .filter_map(|&e| {
            ast.node_for_entity(e)
                .map(|node| jackdaw_jsn::format::JsnEntity {
                    parent: None,
                    components: node.components.clone(),
                })
        })
        .collect();

    if jsn_entities.is_empty() {
        warn!("Copy: no selected entities have AST nodes");
        return;
    }

    let jsn_text = match serde_json::to_string_pretty(&jsn_entities) {
        Ok(t) => t,
        Err(e) => {
            warn!("Failed to serialize entities for clipboard: {e}");
            return;
        }
    };

    let Some(mut cb) = world.get_resource_mut::<SystemClipboard>() else {
        return;
    };
    info!(
        "Display: WAYLAND_DISPLAY={:?} DISPLAY={:?}",
        std::env::var("WAYLAND_DISPLAY").ok(),
        std::env::var("DISPLAY").ok(),
    );
    cb.last_jsn = jsn_text.clone();
    match cb.clipboard.set_text(&jsn_text) {
        Ok(()) => {
            // Verify by reading back, like BSN branch does
            match cb.clipboard.get_text() {
                Ok(readback) => info!(
                    "Clipboard set+readback OK ({} bytes written, {} read back)",
                    jsn_text.len(),
                    readback.len(),
                ),
                Err(e) => warn!("Clipboard set OK but readback failed: {e}"),
            }
        }
        Err(e) => warn!("Copy: system clipboard failed ({e}), using internal fallback"),
    }
}

/// Paste entities from system clipboard JSN text.
fn paste_components(world: &mut World) {
    let jsn_text = {
        let Some(mut cb) = world.get_resource_mut::<SystemClipboard>() else {
            return;
        };
        cb.clipboard
            .get_text()
            .unwrap_or_else(|_| cb.last_jsn.clone())
    };

    if jsn_text.trim().is_empty() {
        return;
    }

    let parsed: Vec<jackdaw_jsn::format::JsnEntity> = match serde_json::from_str(&jsn_text) {
        Ok(entities) => entities,
        Err(e) => {
            warn!("Clipboard text is not valid JSN: {e}");
            return;
        }
    };

    if parsed.is_empty() {
        return;
    }

    let local_assets = std::collections::HashMap::new();
    let parent_path = std::path::Path::new(".");
    let spawned = crate::scene_io::load_scene_from_jsn(world, &parsed, parent_path, &local_assets);

    crate::scene_io::register_entities_in_ast(world, &spawned);

    // Deselect current, select pasted
    for &entity in &world.resource::<Selection>().entities.clone() {
        if let Ok(mut ec) = world.get_entity_mut(entity) {
            ec.remove::<Selected>();
        }
    }
    let mut selection = world.resource_mut::<Selection>();
    selection.entities = spawned.clone();
    for &entity in &spawned {
        world.entity_mut(entity).insert(Selected);
    }

    info!("Pasted {} entities from JSN clipboard", spawned.len());
}

fn hide_selected(world: &mut World) {
    let selection = world.resource::<Selection>();
    let entities: Vec<Entity> = selection.entities.clone();

    if entities.is_empty() {
        return;
    }

    let mut cmds: Vec<Box<dyn EditorCommand>> = Vec::new();

    for &entity in &entities {
        let current = world
            .get::<Visibility>(entity)
            .copied()
            .unwrap_or(Visibility::Inherited);

        let new_visibility = match current {
            Visibility::Hidden => Visibility::Inherited,
            _ => Visibility::Hidden,
        };

        let mut cmd = crate::commands::SetJsnField {
            entity,
            type_path: "bevy_camera::visibility::Visibility".to_string(),
            field_path: String::new(),
            old_value: serde_json::Value::String(format!("{current:?}")),
            new_value: serde_json::Value::String(format!("{new_visibility:?}")),
            was_derived: false,
        };
        cmd.execute(world);
        cmds.push(Box::new(cmd));
    }

    if !cmds.is_empty() {
        let group = crate::commands::CommandGroup {
            commands: cmds,
            label: "Toggle visibility".to_string(),
        };
        let mut history = world.resource_mut::<CommandHistory>();
        history.push_executed(Box::new(group));
    }
}

fn unhide_all_entities(world: &mut World) {
    let mut cmds: Vec<Box<dyn EditorCommand>> = Vec::new();

    // Only unhide top-level scene entities (with Name), matching hide_unselected logic.
    let hidden: Vec<Entity> = {
        let mut query = world.query_filtered::<(Entity, &Visibility), (
            With<Name>,
            Without<EditorEntity>,
            Without<Node>,
        )>();
        query
            .iter(world)
            .filter(|(_, vis)| **vis == Visibility::Hidden)
            .map(|(e, _)| e)
            .collect()
    };

    for entity in hidden {
        let mut cmd = crate::commands::SetJsnField {
            entity,
            type_path: "bevy_camera::visibility::Visibility".to_string(),
            field_path: String::new(),
            old_value: serde_json::Value::String("Hidden".to_string()),
            new_value: serde_json::Value::String("Inherited".to_string()),
            was_derived: false,
        };
        cmd.execute(world);
        cmds.push(Box::new(cmd));
    }

    if !cmds.is_empty() {
        let group = crate::commands::CommandGroup {
            commands: cmds,
            label: "Unhide all".to_string(),
        };
        let mut history = world.resource_mut::<CommandHistory>();
        history.push_executed(Box::new(group));
    }
}

fn hide_all_entities(world: &mut World) {
    let mut cmds: Vec<Box<dyn EditorCommand>> = Vec::new();

    // Hide all top-level scene entities (same filter as H, applied to everything).
    let to_hide: Vec<(Entity, Visibility)> = {
        let mut query = world.query_filtered::<(Entity, &Visibility), (
            With<Name>,
            Without<EditorEntity>,
            Without<Node>,
        )>();
        query
            .iter(world)
            .filter(|(_, vis)| **vis != Visibility::Hidden)
            .map(|(e, vis)| (e, *vis))
            .collect()
    };

    for (entity, current) in to_hide {
        let mut cmd = crate::commands::SetJsnField {
            entity,
            type_path: "bevy_camera::visibility::Visibility".to_string(),
            field_path: String::new(),
            old_value: serde_json::Value::String(format!("{current:?}")),
            new_value: serde_json::Value::String("Hidden".to_string()),
            was_derived: false,
        };
        cmd.execute(world);
        cmds.push(Box::new(cmd));
    }

    if !cmds.is_empty() {
        let group = crate::commands::CommandGroup {
            commands: cmds,
            label: "Hide all".to_string(),
        };
        let mut history = world.resource_mut::<CommandHistory>();
        history.push_executed(Box::new(group));
    }
}

/// Convert a filesystem path to a Bevy asset path (relative to the assets directory).
///
/// Bevy's default asset source reads from `<base>/assets/` where `<base>` is
/// `BEVY_ASSET_ROOT`, `CARGO_MANIFEST_DIR`, or the executable's parent directory.
fn to_asset_path(path: &str) -> String {
    let path = Path::new(path);
    if let Some(assets_dir) = get_assets_base_dir() {
        if let Ok(relative) = path.strip_prefix(&assets_dir) {
            return relative.to_string_lossy().to_string();
        }
    }
    // Fallback: if already a simple relative path, use as-is
    if !path.is_absolute() {
        return path.to_string_lossy().to_string();
    }
    warn!(
        "Cannot load '{}': file is outside the assets directory. \
         Move it into your project's assets/ folder.",
        path.display()
    );
    path.to_string_lossy().to_string()
}

/// Get the absolute path of Bevy's assets directory.
/// Uses the last-opened ProjectRoot if available, then falls back to
/// the standard FileAssetReader lookup (BEVY_ASSET_ROOT / CARGO_MANIFEST_DIR / exe dir).
fn get_assets_base_dir() -> Option<std::path::PathBuf> {
    // Try ProjectRoot via recent projects config
    if let Some(project_dir) = crate::project::read_last_project() {
        let assets = project_dir.join("assets");
        if assets.is_dir() {
            return Some(assets);
        }
    }

    let base = if let Ok(dir) = std::env::var("BEVY_ASSET_ROOT") {
        std::path::PathBuf::from(dir)
    } else if let Ok(dir) = std::env::var("CARGO_MANIFEST_DIR") {
        std::path::PathBuf::from(dir)
    } else {
        std::env::current_exe().ok()?.parent()?.to_path_buf()
    };
    Some(base.join("assets"))
}
