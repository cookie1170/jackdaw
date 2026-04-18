use bevy::{
    input::mouse::{MouseScrollUnit, MouseWheel},
    prelude::*,
    ui::UiGlobalTransform,
};

use super::{
    CHUNK_SIZE, TerrainBrushSettings, TerrainDirtyChunks, TerrainEditMode, TerrainSculptState,
};
use crate::commands::{CommandHistory, EditorCommand};
use crate::default_style;
use crate::selection::Selection;
use crate::viewport::{MainViewportCamera, SceneViewport};

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        (
            terrain_sculpt_interaction,
            handle_brush_resize_scroll,
            draw_terrain_brush_gizmo,
        )
            .run_if(in_state(crate::AppState::Editor)),
    );
}

/// Undo command for terrain height changes.
pub struct SetTerrainHeights {
    pub entity: Entity,
    pub old_heights: Vec<f32>,
    pub new_heights: Vec<f32>,
    pub label: String,
}

impl EditorCommand for SetTerrainHeights {
    fn execute(&mut self, world: &mut World) {
        if let Some(mut terrain) = world.get_mut::<jackdaw_jsn::Terrain>(self.entity) {
            terrain.heights = self.new_heights.clone();
        }
        if let Some(mut dirty) = world.get_mut::<TerrainDirtyChunks>(self.entity) {
            dirty.rebuild_all = true;
        }
        sync_terrain_heights_to_ast(world, self.entity);
    }

    fn undo(&mut self, world: &mut World) {
        if let Some(mut terrain) = world.get_mut::<jackdaw_jsn::Terrain>(self.entity) {
            terrain.heights = self.old_heights.clone();
        }
        if let Some(mut dirty) = world.get_mut::<TerrainDirtyChunks>(self.entity) {
            dirty.rebuild_all = true;
        }
        sync_terrain_heights_to_ast(world, self.entity);
    }

    fn description(&self) -> &str {
        &self.label
    }
}

fn sync_terrain_heights_to_ast(world: &mut World, entity: Entity) {
    if let Some(terrain) = world.get::<jackdaw_jsn::Terrain>(entity) {
        let terrain = terrain.clone();
        crate::commands::sync_component_to_ast(
            world,
            entity,
            "jackdaw_jsn::types::terrain::Terrain",
            &terrain,
        );
    }
}

fn terrain_sculpt_interaction(
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    camera_query: Query<(&Camera, &GlobalTransform), With<MainViewportCamera>>,
    viewport_query: Query<(&ComputedNode, &UiGlobalTransform), With<SceneViewport>>,
    mut terrain_query: Query<(
        Entity,
        &mut jackdaw_jsn::Terrain,
        &GlobalTransform,
        &mut TerrainDirtyChunks,
    )>,
    edit_mode: Res<TerrainEditMode>,
    brush_settings: Res<TerrainBrushSettings>,
    mut sculpt_state: ResMut<TerrainSculptState>,
    selection: Res<Selection>,
    mut history: ResMut<CommandHistory>,
    time: Res<Time>,
) {
    let tool = match *edit_mode {
        TerrainEditMode::Sculpt(tool) => tool,
        _ => {
            if sculpt_state.active {
                sculpt_state.active = false;
                sculpt_state.brush_position = None;
            }
            return;
        }
    };

    // Must have a terrain selected
    let Some(selected) = selection.primary() else {
        sculpt_state.brush_position = None;
        return;
    };
    let Ok((terrain_entity, mut terrain, terrain_tf, mut dirty)) = terrain_query.get_mut(selected)
    else {
        sculpt_state.brush_position = None;
        return;
    };

    let Ok(window) = windows.single() else {
        return;
    };
    let Some(cursor_pos) = window.cursor_position() else {
        return;
    };

    // Viewport cursor conversion
    let Ok((vp_computed, vp_tf)) = viewport_query.single() else {
        return;
    };
    let scale = vp_computed.inverse_scale_factor();
    let vp_pos = vp_tf.translation * scale;
    let vp_size = vp_computed.size() * scale;
    let vp_top_left = vp_pos - vp_size / 2.0;
    let local_cursor = cursor_pos - vp_top_left;
    if local_cursor.x < 0.0
        || local_cursor.y < 0.0
        || local_cursor.x > vp_size.x
        || local_cursor.y > vp_size.y
    {
        return;
    }

    let Ok((camera, cam_tf)) = camera_query.single() else {
        return;
    };
    let target_size = camera.logical_viewport_size().unwrap_or(vp_size);
    let local_cursor = local_cursor * target_size / vp_size;

    // Raycast against the terrain's XZ plane to find hit position
    let Ok(ray) = camera.viewport_to_world(cam_tf, local_cursor) else {
        return;
    };

    let terrain_origin = terrain_tf.translation();
    let plane_normal = Vec3::Y;
    let denom = ray.direction.dot(plane_normal);

    // Use plane intersection at approximate terrain center height
    let approx_y = terrain_origin.y;
    let hit_pos = if denom.abs() > 1e-6 {
        let t = (approx_y - ray.origin.y) / denom;
        if t > 0.0 {
            let point = ray.origin + ray.direction * t;
            let local = point - terrain_origin;
            let half = terrain.size / 2.0;
            if local.x.abs() <= half.x && local.z.abs() <= half.y {
                Some(point)
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    let Some(world_hit) = hit_pos else {
        sculpt_state.brush_position = None;
        return;
    };

    // Convert world hit to terrain-local, then to grid coords
    let local_hit = world_hit - terrain_origin;
    let heightmap = jackdaw_terrain::Heightmap {
        resolution: terrain.resolution,
        size: terrain.size,
        max_height: terrain.max_height,
        heights: terrain.heights.clone(),
    };
    let grid_pos = heightmap.world_to_grid(Vec2::new(local_hit.x, local_hit.z));
    sculpt_state.brush_position = Some(grid_pos);
    sculpt_state.target = Some(terrain_entity);

    // Start stroke
    if mouse.just_pressed(MouseButton::Left) {
        sculpt_state.active = true;
        sculpt_state.stroke_snapshot = terrain.heights.clone();
    }

    // Apply brush while held
    if sculpt_state.active && mouse.pressed(MouseButton::Left) {
        let mut hm = jackdaw_terrain::Heightmap {
            resolution: terrain.resolution,
            size: terrain.size,
            max_height: terrain.max_height,
            heights: terrain.heights.clone(),
        };

        jackdaw_terrain::apply_brush(
            &mut hm,
            tool,
            grid_pos,
            brush_settings.radius,
            brush_settings.strength,
            brush_settings.falloff,
            time.delta_secs(),
            None,
        );

        let affected =
            jackdaw_terrain::affected_chunks(&hm, grid_pos, brush_settings.radius, CHUNK_SIZE);

        // Write heights back
        terrain.heights = hm.heights;
        for chunk in affected {
            dirty.dirty.insert(chunk);
        }
    }

    // End stroke -- push undo command
    if mouse.just_released(MouseButton::Left) && sculpt_state.active {
        sculpt_state.active = false;

        let cmd = SetTerrainHeights {
            entity: terrain_entity,
            old_heights: std::mem::take(&mut sculpt_state.stroke_snapshot),
            new_heights: terrain.heights.clone(),
            label: format!("Terrain {:?}", tool),
        };
        history.push_executed(Box::new(cmd));
    }
}

fn handle_brush_resize_scroll(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut scroll_events: MessageReader<MouseWheel>,
    edit_mode: Res<TerrainEditMode>,
    mut brush_settings: ResMut<TerrainBrushSettings>,
) {
    if !matches!(*edit_mode, TerrainEditMode::Sculpt(_)) {
        return;
    }

    let shift = keyboard.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if !shift {
        return;
    }

    for event in scroll_events.read() {
        let delta = match event.unit {
            MouseScrollUnit::Line => event.y,
            MouseScrollUnit::Pixel => event.y * 0.01,
        };
        if delta > 0.0 {
            brush_settings.radius = (brush_settings.radius * 1.15).min(50.0);
        } else if delta < 0.0 {
            brush_settings.radius = (brush_settings.radius * 0.87).max(1.0);
        }
    }
}

fn draw_terrain_brush_gizmo(
    sculpt_state: Res<TerrainSculptState>,
    brush_settings: Res<TerrainBrushSettings>,
    edit_mode: Res<TerrainEditMode>,
    terrains: Query<(&jackdaw_jsn::Terrain, &GlobalTransform)>,
    mut gizmos: Gizmos,
) {
    if !matches!(*edit_mode, TerrainEditMode::Sculpt(_)) {
        return;
    }

    let Some(target) = sculpt_state.target else {
        return;
    };
    let Some(grid_pos) = sculpt_state.brush_position else {
        return;
    };

    let Ok((terrain, terrain_tf)) = terrains.get(target) else {
        return;
    };

    let heightmap = jackdaw_terrain::Heightmap {
        resolution: terrain.resolution,
        size: terrain.size,
        max_height: terrain.max_height,
        heights: terrain.heights.clone(),
    };

    let segments = 32;
    let radius = brush_settings.radius;
    let origin = terrain_tf.translation();
    let cell = heightmap.cell_size();

    for i in 0..segments {
        let a0 = (i as f32 / segments as f32) * std::f32::consts::TAU;
        let a1 = ((i + 1) as f32 / segments as f32) * std::f32::consts::TAU;

        let gx0 = grid_pos.x + a0.cos() * radius;
        let gz0 = grid_pos.y + a0.sin() * radius;
        let gx1 = grid_pos.x + a1.cos() * radius;
        let gz1 = grid_pos.y + a1.sin() * radius;

        let h0 = heightmap.sample_bilinear(gx0, gz0);
        let h1 = heightmap.sample_bilinear(gx1, gz1);

        let half = terrain.size / 2.0;
        let p0 = origin + Vec3::new(gx0 * cell.x - half.x, h0 + 0.1, gz0 * cell.y - half.y);
        let p1 = origin + Vec3::new(gx1 * cell.x - half.x, h1 + 0.1, gz1 * cell.y - half.y);

        gizmos.line(p0, p1, default_style::TERRAIN_SCULPT_GIZMO);
    }
}
