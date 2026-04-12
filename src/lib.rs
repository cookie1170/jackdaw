pub mod alignment_guides;
pub mod asset_browser;
pub mod asset_catalog;
pub mod brush;
pub mod colors;
pub mod commands;
pub mod custom_properties;
pub mod draw_brush;
pub mod entity_ops;
pub mod entity_templates;
pub mod face_grid;
pub mod gizmos;
pub mod hierarchy;
pub mod inspector;
pub mod keybind_settings;
pub mod keybinds;
pub use inspector::{EditorMeta, ReflectEditorMeta};
pub mod layout;
pub mod material_browser;
pub mod material_preview;
pub mod modal_transform;
pub mod navmesh;
pub mod physics_brush_bridge;
pub mod physics_tool;
pub mod prefab_picker;
pub mod project;
pub mod project_files;
pub mod project_select;
pub mod remote;
pub mod scene_io;
pub mod selection;
pub mod snapping;
pub mod status_bar;
pub mod terrain;
pub mod texture_browser;
pub mod view_modes;
pub mod viewport;
pub mod viewport_overlays;
pub mod viewport_select;
pub mod viewport_util;

use bevy::{
    ecs::system::SystemState,
    feathers::{FeathersPlugins, dark_theme::create_dark_theme, theme::UiTheme},
    input::mouse::{MouseScrollUnit, MouseWheel},
    input_focus::InputDispatchPlugin,
    picking::hover::HoverMap,
    prelude::*,
};
use jackdaw_feathers::EditorFeathersPlugin;
use jackdaw_feathers::dialog::EditorDialog;
use jackdaw_widgets::menu_bar::MenuAction;
use selection::Selection;

/// System set for all editor interaction systems (input handling, viewport clicks,
/// gizmo drags, etc.). Automatically disabled when any dialog is open.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct EditorInteraction;

/// Run condition: returns `true` when no `EditorDialog` entity exists.
pub fn no_dialog_open(dialogs: Query<(), With<EditorDialog>>) -> bool {
    dialogs.is_empty()
}

#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AppState {
    #[default]
    ProjectSelect,
    Editor,
}

#[derive(Component, Default)]
pub struct EditorEntity;

/// Marker component for UI overlays that should block viewport camera input
/// (scroll, pan, orbit) while they exist. Add this to any overlay entity
/// (e.g. prefab picker, context menus) to automatically disable camera controls.
#[derive(Component, Default)]
pub struct BlocksCameraInput;

/// Tag component that hides an entity from the hierarchy panel.
/// Auto-applied to unnamed child entities (likely Bevy internals like shadow cascades).
/// Users can remove it to make hidden entities visible, or add it to hide their own.
#[derive(Component, Default)]
pub struct EditorHidden;

/// Marker component for entities that should not be included in scene serialization.
/// Add this to runtime-generated child entities (brush face meshes, terrain chunks, etc.)
/// that are rebuilt automatically from their parent's component data.
#[derive(Component, Default)]
pub struct NonSerializable;

pub struct EditorPlugin;

impl Plugin for EditorPlugin {
    fn build(&self, app: &mut App) {
        // Disable InputDispatchPlugin from FeathersPlugins because bevy_ui_text_input's
        // TextInputPlugin also adds it unconditionally and panics on duplicates.
        app.init_state::<AppState>()
            .add_plugins((
                FeathersPlugins.build().disable::<InputDispatchPlugin>(),
                EditorFeathersPlugin,
                jackdaw_jsn::JsnPlugin {
                    runtime_mesh_rebuild: false,
                },
                project_select::ProjectSelectPlugin,
                inspector::InspectorPlugin,
                hierarchy::HierarchyPlugin,
                viewport::ViewportPlugin,
                gizmos::TransformGizmosPlugin,
                commands::CommandHistoryPlugin,
                selection::SelectionPlugin,
                entity_ops::EntityOpsPlugin,
                scene_io::SceneIoPlugin,
                asset_browser::AssetBrowserPlugin,
                viewport_select::ViewportSelectPlugin,
                snapping::SnappingPlugin,
            ))
            .add_plugins(keybinds::KeybindsPlugin)
            .add_plugins(keybind_settings::KeybindSettingsPlugin)
            .add_plugins((
                viewport_overlays::ViewportOverlaysPlugin,
                view_modes::ViewModesPlugin,
                status_bar::StatusBarPlugin,
                project_files::ProjectFilesPlugin,
                modal_transform::ModalTransformPlugin,
                custom_properties::CustomPropertiesPlugin,
                entity_templates::EntityTemplatesPlugin,
                brush::BrushPlugin,
            ))
            .add_plugins((
                material_browser::MaterialBrowserPlugin,
                draw_brush::DrawBrushPlugin,
                face_grid::FaceGridPlugin,
                alignment_guides::AlignmentGuidesPlugin,
                navmesh::NavmeshPlugin,
                terrain::TerrainPlugin,
                prefab_picker::PrefabPickerPlugin,
                remote::RemoteConnectionPlugin,
            ))
            .add_plugins(jackdaw_avian_integration::PhysicsOverlaysPlugin::<
                selection::Selected,
            >::new())
            .add_plugins(jackdaw_avian_integration::simulation::PhysicsSimulationPlugin)
            .add_plugins(physics_brush_bridge::PhysicsBrushBridgePlugin)
            .add_plugins(physics_tool::PhysicsToolPlugin)
            .add_plugins(jackdaw_node_graph::NodeGraphPlugin)
            .add_plugins(jackdaw_animation::AnimationPlugin)
            .configure_sets(
                Update,
                EditorInteraction
                    .run_if(in_state(AppState::Editor))
                    .run_if(no_dialog_open),
            )
            .insert_resource(UiTheme(create_dark_theme()))
            .init_resource::<layout::ActiveDocument>()
            .init_resource::<layout::SceneViewPreset>()
            .init_resource::<layout::ActiveDockWindow>()
            .init_resource::<layout::KeybindHelpPopover>()
            .init_resource::<asset_catalog::AssetCatalog>()
            .init_resource::<jackdaw_jsn::SceneJsnAst>()
            .add_systems(
                OnEnter(AppState::Editor),
                (spawn_layout, populate_menu).chain(),
            )
            .add_systems(OnExit(AppState::Editor), cleanup_editor)
            .add_systems(
                Update,
                (
                    send_scroll_events,
                    layout::update_toolbar_highlights,
                    layout::update_toolbar_tooltips,
                    layout::update_space_toggle_label,
                    layout::update_edit_tool_highlights,
                    layout::update_active_document_display,
                    layout::update_tab_strip_highlights,
                    layout::update_dock_body_visibility,
                    layout::update_dock_sidebar_highlights,
                    auto_hide_internal_entities,
                    decorate_timeline_tooltips,
                    discover_gltf_clips,
                    register_animation_entities_in_ast,
                    follow_scene_selection_to_clip,
                    sync_selected_keyframes_from_selection,
                    handle_keyframe_delete_intercept.before(entity_ops::handle_entity_keys),
                    handle_timeline_shortcuts.before(entity_ops::handle_entity_keys),
                )
                    .run_if(in_state(AppState::Editor)),
            )
            .add_observer(on_scroll)
            .add_observer(handle_menu_action)
            .add_observer(on_create_clip_for_selection)
            .add_observer(on_create_blend_graph_for_selection)
            .add_observer(on_header_new_clip)
            .add_observer(on_header_new_blend_graph)
            .add_observer(on_clip_selector_change)
            .add_observer(on_clip_name_commit)
            .add_observer(on_duration_input_commit)
            .add_observer(on_timeline_keyframe_click)
            .add_observer(layout::on_dock_sidebar_icon_click);
    }
}

/// Auto-hide unnamed child entities (likely Bevy internals like shadow cascades).
/// Skips GLTF descendants so they appear in the hierarchy panel.
fn auto_hide_internal_entities(
    mut commands: Commands,
    new_entities: Query<
        (Entity, Option<&Name>, Option<&ChildOf>),
        (
            Added<Transform>,
            Without<EditorEntity>,
            Without<EditorHidden>,
            Without<brush::BrushFaceEntity>,
        ),
    >,
    parent_query: Query<&ChildOf>,
    gltf_sources: Query<(), With<entity_ops::GltfSource>>,
) {
    for (entity, name, parent) in &new_entities {
        if name.is_none() && parent.is_some() {
            // Skip GLTF descendants, they'll be shown in the hierarchy.
            let mut current = entity;
            let mut is_gltf_descendant = false;
            while let Ok(&ChildOf(p)) = parent_query.get(current) {
                if gltf_sources.contains(p) {
                    is_gltf_descendant = true;
                    break;
                }
                current = p;
            }
            if is_gltf_descendant {
                continue;
            }

            if let Ok(mut ec) = commands.get_entity(entity) {
                ec.insert(EditorHidden);
            }
        }
    }
}

fn spawn_layout(mut commands: Commands, icon_font: Res<jackdaw_feathers::icons::IconFont>) {
    commands.spawn((Camera2d, EditorEntity));
    commands.spawn(layout::editor_layout(&icon_font));
}

/// Observer: the header "+" button spawns a new keyframe clip on
/// the same entity as the currently-selected clip. Reuses the same
/// creation logic as `on_create_clip_for_selection` but sources the
/// parent from the active clip's `ChildOf`, not from `Selection`.
fn on_header_new_clip(
    event: On<jackdaw_feathers::button::ButtonClickEvent>,
    buttons: Query<(), With<jackdaw_animation::TimelineHeaderNewClipButton>>,
    selected_clip: Res<jackdaw_animation::SelectedClip>,
    parents: Query<&ChildOf>,
    names: Query<&Name>,
    mut commands: Commands,
) {
    if !buttons.contains(event.entity) {
        return;
    }
    let Some(clip_entity) = selected_clip.0 else {
        return;
    };
    let Ok(clip_parent) = parents.get(clip_entity) else {
        return;
    };
    let target = clip_parent.parent();
    let Ok(name) = names.get(target) else {
        return;
    };
    let target_name = name.as_str().to_string();

    commands.queue(move |world: &mut World| {
        let clip = world
            .spawn((
                jackdaw_animation::Clip::default(),
                Name::new(format!("{target_name} Clip")),
                ChildOf(target),
            ))
            .id();
        world.spawn((
            jackdaw_animation::AnimationTrack::new(
                "bevy_transform::components::transform::Transform",
                "translation",
            ),
            Name::new(format!("{target_name} / translation")),
            ChildOf(clip),
        ));
        if let Some(mut selected) = world.get_resource_mut::<jackdaw_animation::SelectedClip>() {
            selected.0 = Some(clip);
        }
        if let Some(mut dirty) = world.get_resource_mut::<jackdaw_animation::TimelineDirty>() {
            dirty.0 = true;
        }
    });
}

/// Observer: the header blend-graph button spawns a new blend graph
/// clip on the same entity as the currently-selected clip.
fn on_header_new_blend_graph(
    event: On<jackdaw_feathers::button::ButtonClickEvent>,
    buttons: Query<(), With<jackdaw_animation::TimelineHeaderNewBlendGraphButton>>,
    selected_clip: Res<jackdaw_animation::SelectedClip>,
    parents: Query<&ChildOf>,
    names: Query<&Name>,
    mut commands: Commands,
) {
    if !buttons.contains(event.entity) {
        return;
    }
    let Some(clip_entity) = selected_clip.0 else {
        return;
    };
    let Ok(clip_parent) = parents.get(clip_entity) else {
        return;
    };
    let target = clip_parent.parent();
    let Ok(name) = names.get(target) else {
        return;
    };
    let target_name = name.as_str().to_string();

    commands.queue(move |world: &mut World| {
        let clip = world
            .spawn((
                jackdaw_animation::Clip::default(),
                jackdaw_animation::AnimationBlendGraph,
                jackdaw_node_graph::NodeGraph {
                    title: format!("{target_name} Blend Graph"),
                },
                jackdaw_node_graph::GraphCanvasView::default(),
                Name::new(format!("{target_name} Blend Graph")),
                ChildOf(target),
            ))
            .id();
        world.spawn((
            jackdaw_node_graph::GraphNode {
                node_type: "anim.output".into(),
                position: Vec2::new(400.0, 160.0),
            },
            jackdaw_animation::OutputNode,
            Name::new("Output"),
            ChildOf(clip),
        ));
        if let Some(mut selected) = world.get_resource_mut::<jackdaw_animation::SelectedClip>() {
            selected.0 = Some(clip);
        }
        if let Some(mut dirty) = world.get_resource_mut::<jackdaw_animation::TimelineDirty>() {
            dirty.0 = true;
        }
    });
}

/// Clip selector combobox changed. Maps the selected index to a
/// clip entity and switches `SelectedClip`.
fn on_clip_selector_change(
    event: On<jackdaw_feathers::combobox::ComboBoxChangeEvent>,
    selectors: Query<&jackdaw_animation::TimelineClipSelector>,
    child_of_query: Query<&ChildOf>,
    mut commands: Commands,
) {
    let mut current = event.entity;
    let mut selector = None;
    for _ in 0..6 {
        if let Ok(s) = selectors.get(current) {
            selector = Some(s);
            break;
        }
        let Ok(parent) = child_of_query.get(current) else {
            break;
        };
        current = parent.parent();
    }
    let Some(selector) = selector else {
        return;
    };
    let idx = event.selected;
    let Some(&clip_entity) = selector.sibling_clips.get(idx) else {
        return;
    };
    commands.queue(move |world: &mut World| {
        if let Some(mut selected) = world.get_resource_mut::<jackdaw_animation::SelectedClip>() {
            selected.0 = Some(clip_entity);
        }
        if let Some(mut dirty) = world.get_resource_mut::<jackdaw_animation::TimelineDirty>() {
            dirty.0 = true;
        }
    });
}

/// Observer: when the inline clip-name text_edit commits, route the
/// rename through `SetJsnField` on the `Name` component so it
/// participates in undo and round-trips through JSN.
fn on_clip_name_commit(
    event: On<jackdaw_feathers::text_edit::TextEditCommitEvent>,
    name_inputs: Query<&jackdaw_animation::TimelineClipNameInput>,
    child_of_query: Query<&ChildOf>,
    names: Query<&Name>,
    mut commands: Commands,
) {
    let mut current = event.entity;
    let mut clip_entity = None;
    for _ in 0..6 {
        if let Ok(input) = name_inputs.get(current) {
            clip_entity = Some(input.clip);
            break;
        }
        let Ok(parent) = child_of_query.get(current) else {
            break;
        };
        current = parent.parent();
    }
    let Some(clip_entity) = clip_entity else {
        return;
    };
    let new_name = event.text.clone();
    if new_name.is_empty() {
        return;
    }
    let Ok(old_name) = names.get(clip_entity) else {
        return;
    };
    if old_name.as_str() == new_name {
        return;
    }
    commands.queue(move |world: &mut World| {
        if let Some(mut name) = world.get_mut::<Name>(clip_entity) {
            *name = Name::new(new_name);
        }
        if let Some(mut dirty) = world.get_resource_mut::<jackdaw_animation::TimelineDirty>() {
            dirty.0 = true;
        }
    });
}

/// One-shot decorator: when timeline header buttons appear, stamp
/// them with `ToolbarTooltip` so the existing tooltip system picks
/// them up on hover. Runs every frame but short-circuits via
/// `Added<T>` filters — only fires once per button spawn.
#[allow(clippy::type_complexity)]
fn decorate_timeline_tooltips(
    play: Query<Entity, Added<jackdaw_animation::TimelinePlayButton>>,
    pause: Query<Entity, Added<jackdaw_animation::TimelinePauseButton>>,
    stop: Query<Entity, Added<jackdaw_animation::TimelineStopButton>>,
    new_clip: Query<Entity, Added<jackdaw_animation::TimelineHeaderNewClipButton>>,
    new_blend: Query<Entity, Added<jackdaw_animation::TimelineHeaderNewBlendGraphButton>>,
    mut commands: Commands,
) {
    for e in &play {
        commands
            .entity(e)
            .insert(layout::ToolbarTooltip("Play".into()));
    }
    for e in &pause {
        commands
            .entity(e)
            .insert(layout::ToolbarTooltip("Pause".into()));
    }
    for e in &stop {
        commands
            .entity(e)
            .insert(layout::ToolbarTooltip("Stop".into()));
    }
    for e in &new_clip {
        commands
            .entity(e)
            .insert(layout::ToolbarTooltip("New Clip".into()));
    }
    for e in &new_blend {
        commands
            .entity(e)
            .insert(layout::ToolbarTooltip("New Blend Graph".into()));
    }
}

/// Observer: when the placeholder "Create Blend Graph" button is
/// clicked, spawn a `Clip + AnimationBlendGraph + NodeGraph +
/// GraphCanvasView + Name` entity parented to the primary selection,
/// plus a default `OutputNode` inside it so the canvas has
/// something to connect to. Mirror of
/// [`on_create_clip_for_selection`] for the node-canvas path.
fn on_create_blend_graph_for_selection(
    event: On<jackdaw_feathers::button::ButtonClickEvent>,
    buttons: Query<(), With<jackdaw_animation::TimelineCreateBlendGraphButton>>,
    selection: Res<selection::Selection>,
    names: Query<&Name>,
    mut commands: Commands,
) {
    if !buttons.contains(event.entity) {
        return;
    }
    let Some(&primary) = selection.entities.last() else {
        warn!("Create Blend Graph: no entity selected");
        return;
    };
    let Ok(name) = names.get(primary) else {
        warn!(
            "Create Blend Graph: selected entity has no Name — give it one in the inspector first"
        );
        return;
    };
    let target_name = name.as_str().to_string();

    commands.queue(move |world: &mut World| {
        // The blend graph clip is BOTH a `Clip` and a `NodeGraph` —
        // the canvas widget consumes the NodeGraph side of that
        // entity, and the timeline dock consumes the Clip side. That
        // means children are GraphNodes + Connections rather than
        // AnimationTracks, but `compile_clips` already skips entities
        // marked with `AnimationBlendGraph`, and `rebuild_timeline`
        // branches on the same marker to spawn a canvas instead of
        // the keyframe strip.
        let clip_entity = world
            .spawn((
                jackdaw_animation::Clip::default(),
                jackdaw_animation::AnimationBlendGraph,
                jackdaw_node_graph::NodeGraph {
                    title: format!("{target_name} Blend Graph"),
                },
                jackdaw_node_graph::GraphCanvasView::default(),
                Name::new(format!("{target_name} Blend Graph")),
                ChildOf(primary),
            ))
            .id();

        // Default Output node so the canvas isn't empty on creation
        // and the user has a clear target to wire their Clip
        // Reference into. Positioned near the top-right so there's
        // room for source nodes to the left.
        world.spawn((
            jackdaw_node_graph::GraphNode {
                node_type: "anim.output".into(),
                position: Vec2::new(400.0, 160.0),
            },
            jackdaw_animation::OutputNode,
            Name::new("Output"),
            ChildOf(clip_entity),
        ));

        if let Some(mut selected) = world.get_resource_mut::<jackdaw_animation::SelectedClip>() {
            selected.0 = Some(clip_entity);
        }
        if let Some(mut dirty) = world.get_resource_mut::<jackdaw_animation::TimelineDirty>() {
            dirty.0 = true;
        }
    });
}

/// Observer: when the placeholder "Create Clip for Selection" button
/// is clicked, spawn a new `Clip` + `Name` + default `AnimationTrack` for
/// the primary selected entity, directly via `SpawnEntity`. The
/// animation crate deliberately exports no custom commands — this is
/// the minimum-wrapping form of "create a clip."
fn on_create_clip_for_selection(
    event: On<jackdaw_feathers::button::ButtonClickEvent>,
    buttons: Query<(), With<jackdaw_animation::TimelineCreateClipButton>>,
    selection: Res<selection::Selection>,
    names: Query<&Name>,
    mut commands: Commands,
) {
    if !buttons.contains(event.entity) {
        return;
    }
    let Some(&primary) = selection.entities.last() else {
        warn!("Create Clip: no entity selected");
        return;
    };
    let Ok(name) = names.get(primary) else {
        warn!("Create Clip: selected entity has no Name — give it one in the inspector first");
        return;
    };
    let target_name = name.as_str().to_string();

    commands.queue(move |world: &mut World| {
        // Spawn clip entity *as a child of the target*. The clip's
        // position in the hierarchy is what encodes "this animates
        // that" — compile/bind/snapshot all walk up from the clip to
        // the parent to find the target. Deletion cascades naturally
        // and renaming the target can't silently break the clip
        // because the target is a live Entity reference, not a
        // String.
        let clip_entity = world
            .spawn((
                jackdaw_animation::Clip::default(),
                Name::new(format!("{target_name} Clip")),
                ChildOf(primary),
            ))
            .id();

        // Default translation track as a child of the clip.
        world.spawn((
            jackdaw_animation::AnimationTrack::new(
                "bevy_transform::components::transform::Transform",
                "translation",
            ),
            Name::new(format!("{target_name} / translation")),
            ChildOf(clip_entity),
        ));

        if let Some(mut selected) = world.get_resource_mut::<jackdaw_animation::SelectedClip>() {
            selected.0 = Some(clip_entity);
        }
        if let Some(mut dirty) = world.get_resource_mut::<jackdaw_animation::TimelineDirty>() {
            dirty.0 = true;
        }
    });
}

/// Keep [`jackdaw_animation::SelectedClip`] in lockstep with the main
/// editor's [`selection::Selection`] resource so the timeline widget
/// shows the clip relevant to whatever the user is currently working
/// with.
///
/// Two cases are actively updated:
/// - **A.** Primary selection is already an animation entity (clip,
///   track, or keyframe) — walk up `ChildOf` until we hit the owning
///   `Clip` marker and select that.
/// - **B.** Primary selection is a regular scene entity — find the
///   first `Clip` among its `Children` and select it. Since clips
///   now live parented to their target, this is a structural lookup
///   rather than a name-based scan.
///
/// **Empty selection is deliberately a no-op.** After deleting a
/// keyframe the main `delete_selected` path clears `Selection`; if
/// we also cleared `SelectedClip` here the timeline would bounce to
/// its placeholder after every keyframe delete. The stale case —
/// deleting a brush cascades through `ChildOf` and takes its clip
/// with it — is already handled by `rebuild_timeline`, which falls
/// through to the placeholder when `clips.get(selected.0)` fails.
///
/// Lives here rather than in `jackdaw_animation` because the animation
/// crate must not import the main editor's `Selection` type.
fn follow_scene_selection_to_clip(
    selection: Res<selection::Selection>,
    mut selected_clip: ResMut<jackdaw_animation::SelectedClip>,
    parents: Query<&ChildOf>,
    entity_children: Query<&Children>,
    clip_marker: Query<(), With<jackdaw_animation::Clip>>,
) {
    if !selection.is_changed() {
        return;
    }
    // Empty selection: keep the current clip active so keyframe
    // deletes (which clear `Selection`) don't also reset the
    // timeline's context.
    let Some(&primary) = selection.entities.last() else {
        return;
    };

    // Case A: primary is a clip/track/keyframe; walk up to the clip.
    let mut cursor = primary;
    for _ in 0..8 {
        if clip_marker.contains(cursor) {
            if selected_clip.0 != Some(cursor) {
                selected_clip.0 = Some(cursor);
            }
            return;
        }
        let Ok(parent) = parents.get(cursor) else {
            break;
        };
        cursor = parent.parent();
    }

    // Case B: primary is a regular scene entity; pick the first Clip
    // child under it.
    if let Ok(children) = entity_children.get(primary) {
        for child in children.iter() {
            if clip_marker.contains(child) {
                if selected_clip.0 != Some(child) {
                    selected_clip.0 = Some(child);
                }
                return;
            }
        }
    }

    // Case C: the selected entity is not an animation entity and has
    // no clip children. Clear the active clip so the timeline shows
    // the placeholder with "Create Clip" / "Create Blend Graph".
    // This is distinct from the empty-selection guard at the top —
    // empty selection preserves the clip (so keyframe deletes don't
    // bounce the timeline), but selecting a clipless entity is an
    // explicit context switch.
    selected_clip.0 = None;
}

/// Typed, undo-aware delete command for animation keyframes.
///
/// We don't reuse [`commands::DespawnEntity`] for keyframes because
/// that path round-trips through Bevy's `DynamicScene::write_to_world`,
/// which doesn't play well with entity ID reuse: after despawn,
/// Bevy may reissue the keyframe's slot to a later-spawned entity,
/// and an undo that restores the snapshot at the original ID can
/// end up clobbering whatever is living at that slot now (the user
/// saw this as "Ctrl+Z deletes my brush").
///
/// This command captures the keyframe's fields directly — `time`,
/// `value`, and parent `track` — and on undo spawns a **fresh**
/// entity with those fields parented to the original track. No
/// ID reuse, no `DynamicScene`, no surprises.
enum DespawnKeyframeCmd {
    Vec3 {
        /// Current entity id. Updated after each undo so redo knows
        /// which live entity to despawn.
        keyframe: Entity,
        track: Entity,
        time: f32,
        value: Vec3,
    },
    Quat {
        keyframe: Entity,
        track: Entity,
        time: f32,
        value: Quat,
    },
    F32 {
        keyframe: Entity,
        track: Entity,
        time: f32,
        value: f32,
    },
}

impl jackdaw_commands::EditorCommand for DespawnKeyframeCmd {
    fn execute(&mut self, world: &mut World) {
        let entity = match self {
            Self::Vec3 { keyframe, .. }
            | Self::Quat { keyframe, .. }
            | Self::F32 { keyframe, .. } => *keyframe,
        };
        if let Ok(ent) = world.get_entity_mut(entity) {
            ent.despawn();
        }
    }

    fn undo(&mut self, world: &mut World) {
        let new_id = match self {
            Self::Vec3 {
                track, time, value, ..
            } => world
                .spawn((
                    jackdaw_animation::Vec3Keyframe {
                        time: *time,
                        value: *value,
                    },
                    ChildOf(*track),
                ))
                .id(),
            Self::Quat {
                track, time, value, ..
            } => world
                .spawn((
                    jackdaw_animation::QuatKeyframe {
                        time: *time,
                        value: *value,
                    },
                    ChildOf(*track),
                ))
                .id(),
            Self::F32 {
                track, time, value, ..
            } => world
                .spawn((
                    jackdaw_animation::F32Keyframe {
                        time: *time,
                        value: *value,
                    },
                    ChildOf(*track),
                ))
                .id(),
        };
        match self {
            Self::Vec3 { keyframe, .. }
            | Self::Quat { keyframe, .. }
            | Self::F32 { keyframe, .. } => *keyframe = new_id,
        }
    }

    fn description(&self) -> &str {
        "Delete keyframe"
    }
}

impl DespawnKeyframeCmd {
    /// Try to build a despawn command for `entity`. Returns `None`
    /// if the entity doesn't have any of the known keyframe
    /// component types, so the caller can fall through to a
    /// generic despawn.
    fn try_from_entity(world: &World, entity: Entity) -> Option<Self> {
        let track = world.get::<ChildOf>(entity).map(|c| c.parent())?;
        if let Some(kf) = world.get::<jackdaw_animation::Vec3Keyframe>(entity) {
            return Some(Self::Vec3 {
                keyframe: entity,
                track,
                time: kf.time,
                value: kf.value,
            });
        }
        if let Some(kf) = world.get::<jackdaw_animation::QuatKeyframe>(entity) {
            return Some(Self::Quat {
                keyframe: entity,
                track,
                time: kf.time,
                value: kf.value,
            });
        }
        if let Some(kf) = world.get::<jackdaw_animation::F32Keyframe>(entity) {
            return Some(Self::F32 {
                keyframe: entity,
                track,
                time: kf.time,
                value: kf.value,
            });
        }
        None
    }
}

/// Interceptor that runs before [`entity_ops::handle_entity_keys`]
/// and steals the Delete key for any selected keyframe entities.
/// Each keyframe gets wrapped in a [`DespawnKeyframeCmd`], the
/// commands are grouped and pushed onto the history, and the
/// keyframes are removed from [`selection::Selection`] so the
/// downstream generic delete handler ignores them.
///
/// Mixed selections (keyframes + a scene entity) work: this system
/// handles the keyframes, then `handle_entity_keys` handles the
/// remaining non-keyframe entities normally. Both halves land on
/// the history as independent commands, which is fine — undo
/// reverses them in push order.
fn handle_keyframe_delete_intercept(world: &mut World) {
    let keyboard = world.resource::<ButtonInput<KeyCode>>();
    let keybinds = world.resource::<crate::keybinds::KeybindRegistry>();
    if !keybinds.just_pressed(crate::keybinds::EditorAction::Delete, keyboard) {
        return;
    }

    // Don't process delete while a text input is focused — matches
    // the guard in `handle_entity_keys`.
    if world
        .resource::<bevy::input_focus::InputFocus>()
        .0
        .is_some()
    {
        return;
    }

    let selected: Vec<Entity> = world.resource::<selection::Selection>().entities.clone();
    if selected.is_empty() {
        return;
    }

    // Split the selection into keyframe entities and everything else.
    let mut kf_cmds: Vec<Box<dyn jackdaw_commands::EditorCommand>> = Vec::new();
    let mut keyframe_ids: Vec<Entity> = Vec::new();
    for &entity in &selected {
        if let Some(cmd) = DespawnKeyframeCmd::try_from_entity(world, entity) {
            keyframe_ids.push(entity);
            kf_cmds.push(Box::new(cmd));
        }
    }

    if kf_cmds.is_empty() {
        return;
    }

    // Strip the keyframes out of Selection so the downstream
    // generic delete path doesn't see them.
    {
        let mut selection = world.resource_mut::<selection::Selection>();
        selection.entities.retain(|e| !keyframe_ids.contains(e));
    }
    for entity in &keyframe_ids {
        if let Ok(mut ent) = world.get_entity_mut(*entity) {
            ent.remove::<selection::Selected>();
        }
    }

    // Execute each keyframe despawn and wrap them in a single
    // group so Ctrl+Z undoes the whole delete at once.
    for cmd in &mut kf_cmds {
        cmd.execute(world);
    }
    let group = commands::CommandGroup {
        commands: kf_cmds,
        label: "Delete keyframes".to_string(),
    };
    let mut history = world.resource_mut::<jackdaw_commands::CommandHistory>();
    history.undo_stack.push(Box::new(group));
    history.redo_stack.clear();
}

/// Typed, undo-aware spawn command for animation keyframes. Mirror of
/// [`DespawnKeyframeCmd`] — execute spawns a fresh entity with the
/// stored fields parented to the track, undo despawns it. Same ID-
/// reuse avoidance rationale: direct `world.spawn` rather than
/// `DynamicScene`.
///
/// Used by the keyframe paste path (`handle_keyframe_copy_paste`) so
/// pasting is undoable as a single `CommandGroup`.
enum SpawnKeyframeCmd {
    Vec3 {
        /// Filled in by `execute`; `None` before the first execute.
        keyframe: Option<Entity>,
        track: Entity,
        time: f32,
        value: Vec3,
    },
    Quat {
        keyframe: Option<Entity>,
        track: Entity,
        time: f32,
        value: Quat,
    },
    F32 {
        keyframe: Option<Entity>,
        track: Entity,
        time: f32,
        value: f32,
    },
}

impl jackdaw_commands::EditorCommand for SpawnKeyframeCmd {
    fn execute(&mut self, world: &mut World) {
        let new_id = match self {
            Self::Vec3 {
                track, time, value, ..
            } => world
                .spawn((
                    jackdaw_animation::Vec3Keyframe {
                        time: *time,
                        value: *value,
                    },
                    ChildOf(*track),
                ))
                .id(),
            Self::Quat {
                track, time, value, ..
            } => world
                .spawn((
                    jackdaw_animation::QuatKeyframe {
                        time: *time,
                        value: *value,
                    },
                    ChildOf(*track),
                ))
                .id(),
            Self::F32 {
                track, time, value, ..
            } => world
                .spawn((
                    jackdaw_animation::F32Keyframe {
                        time: *time,
                        value: *value,
                    },
                    ChildOf(*track),
                ))
                .id(),
        };
        match self {
            Self::Vec3 { keyframe, .. }
            | Self::Quat { keyframe, .. }
            | Self::F32 { keyframe, .. } => *keyframe = Some(new_id),
        }
    }

    fn undo(&mut self, world: &mut World) {
        let entity = match self {
            Self::Vec3 { keyframe, .. }
            | Self::Quat { keyframe, .. }
            | Self::F32 { keyframe, .. } => *keyframe,
        };
        if let Some(entity) = entity
            && let Ok(ent) = world.get_entity_mut(entity)
        {
            ent.despawn();
        }
    }

    fn description(&self) -> &str {
        "Paste keyframe"
    }
}

/// Combined handler for timeline keyboard shortcuts that need to
/// intercept before [`entity_ops::handle_entity_keys`]:
///
/// - **Arrow keys** (Left/Right/Home/End) step the playhead when the
///   timeline dock window is active. Consumes the key input via
///   [`ButtonInput::clear_just_pressed`] so the entity nudge handler
///   doesn't also slide a selected brush.
/// - **Ctrl+C** copies the currently-selected keyframes (if any) into
///   [`jackdaw_animation::KeyframeClipboard`], then consumes the key
///   so the generic component-copy path doesn't also fire.
/// - **Ctrl+V** pastes clipboard keyframes onto the
///   [`jackdaw_animation::SelectedClip`] at the current cursor time,
///   wrapped in a [`commands::CommandGroup`] of [`SpawnKeyframeCmd`]s
///   for atomic undo.
///
/// All three gate on `InputFocus` being empty so typing in a text
/// field doesn't trigger the timeline shortcuts.
fn handle_timeline_shortcuts(world: &mut World) {
    if world
        .resource::<bevy::input_focus::InputFocus>()
        .0
        .is_some()
    {
        return;
    }

    let (ctrl, shift) = {
        let keyboard = world.resource::<ButtonInput<KeyCode>>();
        (
            keyboard.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]),
            keyboard.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]),
        )
    };

    let timeline_active =
        world.resource::<layout::ActiveDockWindow>().0 == layout::DockWindowKind::Timeline;
    if timeline_active && !ctrl {
        handle_timeline_scrub_keys(world, shift);
    }

    if ctrl {
        handle_keyframe_copy(world);
        handle_keyframe_paste(world);
    }
}

/// Step the timeline cursor with arrow keys, Home, and End. Called
/// from [`handle_timeline_shortcuts`] when the timeline dock window
/// is active and no modifier (other than Shift) is held.
///
/// - Left / Right: step by one ruler tick, using the same
///   [`jackdaw_animation::pick_tick_step`] the timeline widget uses.
/// - Shift+Left / Shift+Right: jump to the previous / next keyframe
///   time across all tracks in the selected clip. Falls back to the
///   clip boundary (0 or `duration`) when there is no earlier /
///   later keyframe.
/// - Home / End: jump to the start / end of the clip.
fn handle_timeline_scrub_keys(world: &mut World, shift: bool) {
    let (left, right, home, end) = {
        let keyboard = world.resource::<ButtonInput<KeyCode>>();
        (
            keyboard.just_pressed(KeyCode::ArrowLeft),
            keyboard.just_pressed(KeyCode::ArrowRight),
            keyboard.just_pressed(KeyCode::Home),
            keyboard.just_pressed(KeyCode::End),
        )
    };
    if !left && !right && !home && !end {
        return;
    }
    let Some(clip_entity) = world.resource::<jackdaw_animation::SelectedClip>().0 else {
        return;
    };
    let Some(clip) = world.get::<jackdaw_animation::Clip>(clip_entity).copied() else {
        return;
    };
    let duration = clip.duration.max(0.01);
    let current_time = world
        .resource::<jackdaw_animation::TimelineCursor>()
        .seek_time;

    let new_time = if home {
        0.0
    } else if end {
        duration
    } else if shift {
        let times = collect_clip_keyframe_times(world, clip_entity);
        if left {
            times
                .iter()
                .copied()
                .filter(|t| *t < current_time - 1e-4)
                .fold(0.0_f32, f32::max)
        } else {
            times
                .iter()
                .copied()
                .filter(|t| *t > current_time + 1e-4)
                .fold(duration, f32::min)
        }
    } else {
        let step = jackdaw_animation::pick_tick_step(duration);
        let dir = if left { -1.0 } else { 1.0 };
        (current_time + dir * step).clamp(0.0, duration)
    };

    world.write_message(jackdaw_animation::AnimationSeek(new_time));

    // Consume the arrow/home/end presses so the entity nudge handler
    // downstream doesn't also move a brush this frame.
    let mut keyboard = world.resource_mut::<ButtonInput<KeyCode>>();
    keyboard.clear_just_pressed(KeyCode::ArrowLeft);
    keyboard.clear_just_pressed(KeyCode::ArrowRight);
    keyboard.clear_just_pressed(KeyCode::Home);
    keyboard.clear_just_pressed(KeyCode::End);
}

/// Gather every keyframe time on the clip, across all tracks and
/// all typed keyframe components. Used by the shift+arrow "step to
/// adjacent keyframe" path.
fn collect_clip_keyframe_times(world: &World, clip_entity: Entity) -> Vec<f32> {
    let mut times = Vec::new();
    let Some(clip_children) = world.get::<Children>(clip_entity) else {
        return times;
    };
    let track_entities: Vec<Entity> = clip_children.iter().collect();
    for track in track_entities {
        let Some(track_children) = world.get::<Children>(track) else {
            continue;
        };
        for kf in track_children.iter() {
            if let Some(k) = world.get::<jackdaw_animation::Vec3Keyframe>(kf) {
                times.push(k.time);
            } else if let Some(k) = world.get::<jackdaw_animation::QuatKeyframe>(kf) {
                times.push(k.time);
            } else if let Some(k) = world.get::<jackdaw_animation::F32Keyframe>(kf) {
                times.push(k.time);
            }
        }
    }
    times
}

/// Handle Ctrl+C when any keyframe is in the current selection: copy
/// a snapshot of each keyframe into [`KeyframeClipboard`] and consume
/// the key so the generic component-copy path doesn't also serialize
/// them. Times are stored relative to the earliest copied keyframe
/// so a later paste reconstructs the spacing anchored at the cursor.
///
/// [`KeyframeClipboard`]: jackdaw_animation::KeyframeClipboard
fn handle_keyframe_copy(world: &mut World) {
    if !world
        .resource::<ButtonInput<KeyCode>>()
        .just_pressed(KeyCode::KeyC)
    {
        return;
    }
    let selected: Vec<Entity> = world.resource::<selection::Selection>().entities.clone();
    if selected.is_empty() {
        return;
    }

    let mut entries: Vec<(f32, jackdaw_animation::KeyframeClipboardEntry)> = Vec::new();
    for &entity in &selected {
        let Some(track_entity) = world.get::<ChildOf>(entity).map(|c| c.parent()) else {
            continue;
        };
        let Some(track) = world.get::<jackdaw_animation::AnimationTrack>(track_entity) else {
            continue;
        };
        let component_type_path = track.component_type_path.clone();
        let field_path = track.field_path.clone();

        if let Some(kf) = world.get::<jackdaw_animation::Vec3Keyframe>(entity) {
            entries.push((
                kf.time,
                jackdaw_animation::KeyframeClipboardEntry {
                    component_type_path,
                    field_path,
                    relative_time: kf.time,
                    value: jackdaw_animation::KeyframeValue::Vec3(kf.value),
                },
            ));
        } else if let Some(kf) = world.get::<jackdaw_animation::QuatKeyframe>(entity) {
            entries.push((
                kf.time,
                jackdaw_animation::KeyframeClipboardEntry {
                    component_type_path,
                    field_path,
                    relative_time: kf.time,
                    value: jackdaw_animation::KeyframeValue::Quat(kf.value),
                },
            ));
        } else if let Some(kf) = world.get::<jackdaw_animation::F32Keyframe>(entity) {
            entries.push((
                kf.time,
                jackdaw_animation::KeyframeClipboardEntry {
                    component_type_path,
                    field_path,
                    relative_time: kf.time,
                    value: jackdaw_animation::KeyframeValue::F32(kf.value),
                },
            ));
        }
    }

    if entries.is_empty() {
        return;
    }

    // Normalize times: relative_time = original_time - min(original_time).
    let base = entries
        .iter()
        .map(|(t, _)| *t)
        .fold(f32::INFINITY, f32::min);
    let mut normalized: Vec<jackdaw_animation::KeyframeClipboardEntry> = entries
        .into_iter()
        .map(|(_, mut entry)| {
            entry.relative_time -= base;
            entry
        })
        .collect();
    // Sort by relative time for deterministic paste ordering.
    normalized.sort_by(|a, b| a.relative_time.partial_cmp(&b.relative_time).unwrap());

    let count = normalized.len();
    world
        .resource_mut::<jackdaw_animation::KeyframeClipboard>()
        .entries = normalized;
    world
        .resource_mut::<ButtonInput<KeyCode>>()
        .clear_just_pressed(KeyCode::KeyC);
    info!("Copied {count} keyframe(s) to animation clipboard");
}

/// Handle Ctrl+V: if the animation clipboard is non-empty and a clip
/// is selected, re-spawn each clipboard entry as a new keyframe
/// parented to the clip's matching track at `cursor_time +
/// relative_time`. Entries whose property address doesn't resolve to
/// an existing track on the current clip are skipped with a warning.
///
/// Each spawn is wrapped in a [`SpawnKeyframeCmd`] and all commands
/// are pushed as a single [`commands::CommandGroup`] so Ctrl+Z undoes
/// the entire paste at once.
fn handle_keyframe_paste(world: &mut World) {
    if !world
        .resource::<ButtonInput<KeyCode>>()
        .just_pressed(KeyCode::KeyV)
    {
        return;
    }
    let entries = world
        .resource::<jackdaw_animation::KeyframeClipboard>()
        .entries
        .clone();
    if entries.is_empty() {
        return;
    }
    let Some(clip_entity) = world.resource::<jackdaw_animation::SelectedClip>().0 else {
        return;
    };
    let cursor_time = world
        .resource::<jackdaw_animation::TimelineCursor>()
        .seek_time;

    // Resolve each entry's target track by property address. Collect
    // the list of tracks under the clip once up front.
    let mut tracks: Vec<(Entity, String, String)> = Vec::new();
    if let Some(children) = world.get::<Children>(clip_entity) {
        for child in children.iter() {
            if let Some(track) = world.get::<jackdaw_animation::AnimationTrack>(child) {
                tracks.push((
                    child,
                    track.component_type_path.clone(),
                    track.field_path.clone(),
                ));
            }
        }
    }

    let mut cmds: Vec<Box<dyn jackdaw_commands::EditorCommand>> = Vec::new();
    let mut max_paste_time = cursor_time;
    for entry in &entries {
        let track_entity = tracks.iter().find_map(|(e, tp, fp)| {
            (tp == &entry.component_type_path && fp == &entry.field_path).then_some(*e)
        });
        let Some(track_entity) = track_entity else {
            warn!(
                "Paste keyframe: no track for {}.{} on selected clip — add one via the inspector diamond first",
                entry.component_type_path, entry.field_path,
            );
            continue;
        };
        let paste_time = cursor_time + entry.relative_time;
        max_paste_time = max_paste_time.max(paste_time);
        let cmd: Box<dyn jackdaw_commands::EditorCommand> = match entry.value {
            jackdaw_animation::KeyframeValue::Vec3(v) => Box::new(SpawnKeyframeCmd::Vec3 {
                keyframe: None,
                track: track_entity,
                time: paste_time,
                value: v,
            }),
            jackdaw_animation::KeyframeValue::Quat(q) => Box::new(SpawnKeyframeCmd::Quat {
                keyframe: None,
                track: track_entity,
                time: paste_time,
                value: q,
            }),
            jackdaw_animation::KeyframeValue::F32(f) => Box::new(SpawnKeyframeCmd::F32 {
                keyframe: None,
                track: track_entity,
                time: paste_time,
                value: f,
            }),
        };
        cmds.push(cmd);
    }

    if cmds.is_empty() {
        return;
    }

    // Auto-extend the clip duration if the paste lands beyond the
    // current authored range. Matches the behavior of
    // `handle_add_keyframe_click` in the animation crate.
    if let Some(mut clip) = world.get_mut::<jackdaw_animation::Clip>(clip_entity)
        && max_paste_time > clip.duration
    {
        clip.duration = max_paste_time;
    }

    for cmd in &mut cmds {
        cmd.execute(world);
    }
    let count = cmds.len();
    let group = commands::CommandGroup {
        commands: cmds,
        label: "Paste keyframes".to_string(),
    };
    let mut history = world.resource_mut::<jackdaw_commands::CommandHistory>();
    history.undo_stack.push(Box::new(group));
    history.redo_stack.clear();
    world
        .resource_mut::<ButtonInput<KeyCode>>()
        .clear_just_pressed(KeyCode::KeyV);

    if let Some(mut dirty) = world.get_resource_mut::<jackdaw_animation::TimelineDirty>() {
        dirty.0 = true;
    }
    info!("Pasted {count} keyframe(s) from animation clipboard");
}

/// Observer: clicking a timeline keyframe diamond routes through
/// the main editor's [`selection::Selection`] resource. Ctrl+click
/// toggles into the existing selection; plain click replaces with
/// just the keyframe. Delete is then handled by the main editor's
/// existing `delete_selected` path, which wraps despawns in
/// `DespawnEntity` commands for undo safety — the animation crate
/// deliberately does NOT own a delete key handler, so there's no
/// risk of double-delete when the user has both a scene entity and
/// a keyframe "selected."
///
/// Propagation is stopped so the click doesn't also hit the
/// scrubber and seek the playhead.
fn on_timeline_keyframe_click(
    mut event: On<Pointer<Click>>,
    handles: Query<&jackdaw_animation::TimelineKeyframeHandle>,
    keys: Res<ButtonInput<KeyCode>>,
    mut selection: ResMut<selection::Selection>,
    mut commands: Commands,
) {
    let Ok(handle) = handles.get(event.event_target()) else {
        return;
    };
    let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    if ctrl {
        selection.toggle(&mut commands, handle.keyframe);
    } else {
        selection.select_single(&mut commands, handle.keyframe);
    }
    event.propagate(false);
}

/// Mirror the main [`selection::Selection`] → the animation crate's
/// [`jackdaw_animation::SelectedKeyframes`] so the timeline
/// highlight system can tell which diamonds to light up without
/// the animation crate needing to import `Selection` itself.
///
/// Runs only when `Selection` changes. Also filters out entities
/// whose keyframe component type isn't one we know about; non-
/// keyframe selections simply don't land in `SelectedKeyframes`.
fn sync_selected_keyframes_from_selection(
    selection: Res<selection::Selection>,
    mut selected_keyframes: ResMut<jackdaw_animation::SelectedKeyframes>,
    vec3_keyframes: Query<(), With<jackdaw_animation::Vec3Keyframe>>,
    quat_keyframes: Query<(), With<jackdaw_animation::QuatKeyframe>>,
    f32_keyframes: Query<(), With<jackdaw_animation::F32Keyframe>>,
) {
    if !selection.is_changed() {
        return;
    }
    selected_keyframes.entities.clear();
    for &entity in &selection.entities {
        if vec3_keyframes.contains(entity)
            || quat_keyframes.contains(entity)
            || f32_keyframes.contains(entity)
        {
            selected_keyframes.entities.insert(entity);
        }
    }
}

/// Observer: when the timeline header's duration field commits,
/// route the edit through `SetJsnField` so it flows through the AST
/// and participates in undo/redo + save/load. This is the hand-off
/// point between the animation crate (which can't import
/// `SetJsnField`) and the editor binary.
fn on_duration_input_commit(
    event: On<jackdaw_feathers::text_edit::TextEditCommitEvent>,
    duration_inputs: Query<&jackdaw_animation::TimelineDurationInput>,
    child_of_query: Query<&ChildOf>,
    clips: Query<&jackdaw_animation::Clip>,
    mut commands: Commands,
) {
    // The commit event fires on the inner text_input entity; the
    // `TimelineDurationInput` marker sits on the wrapper, so walk
    // up one step to find it. Matches the pattern used by
    // `on_material_param_commit` in material_browser.rs.
    let mut current = event.entity;
    let mut marker_clip: Option<Entity> = None;
    for _ in 0..4 {
        if let Ok(marker) = duration_inputs.get(current) {
            marker_clip = Some(marker.clip);
            break;
        }
        let Ok(child_of) = child_of_query.get(current) else {
            break;
        };
        current = child_of.parent();
    }

    let Some(clip_entity) = marker_clip else {
        return;
    };
    let Ok(new_value) = event.text.trim().parse::<f32>() else {
        return;
    };
    let Ok(clip) = clips.get(clip_entity) else {
        return;
    };
    if (new_value - clip.duration).abs() < f32::EPSILON {
        return;
    }
    let old_json = serde_json::json!(clip.duration);
    let new_json = serde_json::json!(new_value);
    commands.queue(move |world: &mut World| {
        let mut history = world
            .remove_resource::<jackdaw_commands::CommandHistory>()
            .unwrap_or_default();
        history.execute(
            Box::new(commands::SetJsnField {
                entity: clip_entity,
                type_path: "jackdaw_animation::clip::Clip".to_string(),
                field_path: "duration".to_string(),
                old_value: old_json,
                new_value: new_json,
            }),
            world,
        );
        world.insert_resource(history);
    });
}

/// After the animation crate spawns new clip/track/keyframe entities,
/// register them in the JSN AST so they participate in save/load and
/// undo/redo snapshotting. Runs every frame; cheap because
/// `register_entity_in_ast` is a no-op for already-registered entities.
fn register_animation_entities_in_ast(
    world: &mut World,
    params: &mut QueryState<
        Entity,
        Or<(
            Added<jackdaw_animation::Clip>,
            Added<jackdaw_animation::AnimationTrack>,
            Added<jackdaw_animation::Vec3Keyframe>,
            Added<jackdaw_animation::QuatKeyframe>,
            Added<jackdaw_animation::F32Keyframe>,
            Added<jackdaw_animation::GltfClipRef>,
            Added<jackdaw_animation::AnimationBlendGraph>,
            Added<jackdaw_node_graph::GraphNode>,
            Added<jackdaw_node_graph::Connection>,
        )>,
    >,
) {
    let entities: Vec<Entity> = params.iter(world).collect();
    for entity in entities {
        scene_io::register_entity_in_ast(world, entity);
    }
}

/// For every [`GltfSource`] entity whose underlying glTF asset is
/// loaded but has not yet had its clips imported, spawn one
/// [`jackdaw_animation::Clip`] + [`jackdaw_animation::GltfClipRef`]
/// child per entry in `Gltf::named_animations`. Those child entities
/// persist through JSN save/load (just two strings each), so this
/// discovery step only needs to run once per glTF in a given session.
///
/// The guard — "skip if any child already has a `GltfClipRef`" — keeps
/// us from resurrecting clips the user deleted within the session.
/// Adding new clips to the glTF file externally requires a scene
/// reload to rediscover them, which matches Blender's "reload glTF"
/// semantics.
///
/// Lives in the main crate rather than `jackdaw_animation` because it
/// needs to read `jackdaw_jsn::GltfSource`, and we'd rather not wire a
/// jackdaw_jsn dep into the animation crate.
///
/// [`GltfSource`]: jackdaw_jsn::GltfSource
fn discover_gltf_clips(
    sources: Query<(Entity, &jackdaw_jsn::GltfSource, Option<&Children>)>,
    existing_refs: Query<(), With<jackdaw_animation::GltfClipRef>>,
    asset_server: Res<AssetServer>,
    gltfs: Res<Assets<bevy::gltf::Gltf>>,
    mut commands: Commands,
) {
    for (entity, source, children) in &sources {
        // Skip if this GltfSource already has any imported clip
        // children — discovery has run at least once.
        let any_existing = children
            .into_iter()
            .flatten()
            .any(|&c| existing_refs.contains(c));
        if any_existing {
            continue;
        }

        let handle: Handle<bevy::gltf::Gltf> = asset_server.load(&source.path);
        let Some(gltf) = gltfs.get(&handle) else {
            continue;
        };

        for (clip_name, _clip_handle) in &gltf.named_animations {
            let name_str = clip_name.to_string();
            commands.spawn((
                jackdaw_animation::Clip::default(),
                jackdaw_animation::GltfClipRef {
                    gltf_path: source.path.clone(),
                    clip_name: name_str.clone(),
                },
                Name::new(name_str),
                ChildOf(entity),
            ));
        }
    }
}

fn populate_menu(world: &mut World) {
    let menu_bar_entity = world
        .query_filtered::<Entity, With<jackdaw_feathers::menu_bar::MenuBarRoot>>()
        .iter(world)
        .next();
    let Some(menu_bar_entity) = menu_bar_entity else {
        return;
    };
    jackdaw_feathers::menu_bar::populate_menu_bar(
        world,
        menu_bar_entity,
        vec![
            (
                "File",
                vec![
                    ("file.new", "New"),
                    ("file.open", "Open"),
                    ("---", ""),
                    ("file.save", "Save"),
                    ("file.save_as", "Save As..."),
                    ("---", ""),
                    ("file.save_template", "Save Selection as Template"),
                    ("---", ""),
                    ("file.keybinds", "Keybinds..."),
                    ("---", ""),
                    ("file.open_recent", "Open Recent..."),
                    ("file.home", "Home"),
                ],
            ),
            (
                "Edit",
                vec![
                    ("edit.undo", "Undo"),
                    ("edit.redo", "Redo"),
                    ("---", ""),
                    ("edit.delete", "Delete"),
                    ("edit.duplicate", "Duplicate"),
                    ("---", ""),
                    ("edit.join", "Join (Convex Merge)"),
                    ("edit.csg_subtract", "CSG Subtract"),
                    ("edit.csg_intersect", "CSG Intersect"),
                    ("edit.extend_to_brush", "Extend to Brush"),
                ],
            ),
            (
                "View",
                vec![
                    ("view.wireframe", "Toggle Wireframe"),
                    ("view.bounding_boxes", "Toggle Bounding Boxes"),
                    ("view.bounding_box_mode", "Cycle Bounding Box Mode"),
                    ("view.face_grid", "Toggle Face Grid"),
                    ("view.brush_wireframe", "Toggle Brush Wireframe"),
                    ("view.alignment_guides", "Toggle Alignment Guides"),
                    ("view.collider_gizmos", "Toggle Collider Gizmos"),
                    ("view.hierarchy_arrows", "Toggle Hierarchy Arrows"),
                ],
            ),
            (
                "Add",
                vec![
                    ("add.cube", "Cube"),
                    ("add.sphere", "Sphere"),
                    ("---", ""),
                    ("add.point_light", "Point Light"),
                    ("add.directional_light", "Directional Light"),
                    ("add.spot_light", "Spot Light"),
                    ("---", ""),
                    ("add.camera", "Camera"),
                    ("add.empty", "Empty"),
                    ("---", ""),
                    ("add.navmesh", "Navmesh Region"),
                    ("add.terrain", "Terrain"),
                    ("---", ""),
                    ("add.prefab", "Prefab..."),
                ],
            ),
        ],
    );
}

fn handle_menu_action(event: On<MenuAction>, mut commands: Commands) {
    match event.action.as_str() {
        "file.new" => {
            commands.queue(|world: &mut World| {
                scene_io::new_scene(world);
            });
        }
        "file.save" => {
            commands.queue(|world: &mut World| {
                scene_io::save_scene(world);
            });
        }
        "file.save_as" => {
            commands.queue(|world: &mut World| {
                scene_io::save_scene_as(world);
            });
        }
        "file.open" => {
            commands.queue(|world: &mut World| {
                scene_io::load_scene(world);
            });
        }
        "file.save_template" => {
            // Use a default name based on the selected entity
            commands.queue(|world: &mut World| {
                let selection = world.resource::<Selection>();
                let name = selection
                    .primary()
                    .and_then(|e| world.get::<Name>(e).map(|n| n.as_str().to_string()))
                    .unwrap_or_else(|| "template".to_string());
                entity_templates::save_entity_template(world, &name);
            });
        }
        "edit.undo" => {
            commands.queue(|world: &mut World| {
                world.resource_scope(|world, mut history: Mut<commands::CommandHistory>| {
                    history.undo(world);
                });
            });
        }
        "edit.redo" => {
            commands.queue(|world: &mut World| {
                world.resource_scope(|world, mut history: Mut<commands::CommandHistory>| {
                    history.redo(world);
                });
            });
        }
        "edit.delete" => {
            commands.queue(|world: &mut World| {
                entity_ops::delete_selected(world);
            });
        }
        "edit.duplicate" => {
            commands.queue(|world: &mut World| {
                entity_ops::duplicate_selected(world);
            });
        }
        "edit.join" => {
            commands.queue(draw_brush::join_selected_brushes_impl);
        }
        "edit.csg_subtract" => {
            commands.queue(draw_brush::csg_subtract_selected_impl);
        }
        "edit.csg_intersect" => {
            commands.queue(draw_brush::csg_intersect_selected_impl);
        }
        "edit.extend_to_brush" => {
            commands.queue(|world: &mut World| {
                let edit_mode = *world.resource::<crate::brush::EditMode>();
                let selection = world.resource::<Selection>();
                let entities = selection.entities.clone();

                let brush_selection = world.resource::<crate::brush::BrushSelection>();

                // Resolve primary + face_index: prefer active face-mode selection,
                // fall back to remembered face.
                let (primary, face_index) = if edit_mode
                    == crate::brush::EditMode::BrushEdit(crate::brush::BrushEditMode::Face)
                {
                    let primary = brush_selection.entity;
                    let face = brush_selection.faces.last().copied();
                    match (primary, face) {
                        (Some(p), Some(f)) => (p, f),
                        _ => return,
                    }
                } else {
                    let primary = match selection.primary() {
                        Some(e) => e,
                        None => return,
                    };
                    let face_index = if brush_selection.last_face_entity == Some(primary) {
                        brush_selection.last_face_index
                    } else {
                        None
                    };
                    match face_index {
                        Some(f) => (primary, f),
                        None => return,
                    }
                };

                let mut brush_query = world.query_filtered::<Entity, With<jackdaw_jsn::Brush>>();
                let targets: Vec<Entity> = entities
                    .iter()
                    .copied()
                    .filter(|&e| e != primary && brush_query.get(world, e).is_ok())
                    .collect();
                if targets.is_empty() {
                    return;
                }

                draw_brush::extend_face_to_brush_impl(world, primary, &targets, face_index);

                // Exit face mode if we were in it (geometry changed, indices invalid)
                if edit_mode == crate::brush::EditMode::BrushEdit(crate::brush::BrushEditMode::Face)
                {
                    *world.resource_mut::<crate::brush::EditMode>() =
                        crate::brush::EditMode::Object;
                    let mut bs = world.resource_mut::<crate::brush::BrushSelection>();
                    bs.entity = None;
                    bs.faces.clear();
                    bs.vertices.clear();
                    bs.edges.clear();
                }
            });
        }
        "file.keybinds" => {
            commands.trigger(keybind_settings::OpenKeybindSettingsEvent);
        }
        "file.home" => {
            commands.queue(|world: &mut World| {
                world
                    .resource_mut::<NextState<AppState>>()
                    .set(AppState::ProjectSelect);
            });
        }
        "file.open_recent" => {
            commands.queue(open_recent_dialog);
        }
        "view.wireframe" => {
            commands.queue(|world: &mut World| {
                let mut settings = world.resource_mut::<view_modes::ViewModeSettings>();
                settings.wireframe = !settings.wireframe;
            });
        }
        "view.bounding_boxes" => {
            commands.queue(|world: &mut World| {
                let mut settings = world.resource_mut::<viewport_overlays::OverlaySettings>();
                settings.show_bounding_boxes = !settings.show_bounding_boxes;
            });
        }
        "view.bounding_box_mode" => {
            commands.queue(|world: &mut World| {
                let mut settings = world.resource_mut::<viewport_overlays::OverlaySettings>();
                settings.bounding_box_mode = match settings.bounding_box_mode {
                    viewport_overlays::BoundingBoxMode::Aabb => {
                        viewport_overlays::BoundingBoxMode::ConvexHull
                    }
                    viewport_overlays::BoundingBoxMode::ConvexHull => {
                        viewport_overlays::BoundingBoxMode::Aabb
                    }
                };
            });
        }
        "view.face_grid" => {
            commands.queue(|world: &mut World| {
                let mut settings = world.resource_mut::<viewport_overlays::OverlaySettings>();
                settings.show_face_grid = !settings.show_face_grid;
            });
        }
        "view.brush_wireframe" => {
            commands.queue(|world: &mut World| {
                let mut settings = world.resource_mut::<viewport_overlays::OverlaySettings>();
                settings.show_brush_wireframe = !settings.show_brush_wireframe;
            });
        }
        "view.alignment_guides" => {
            commands.queue(|world: &mut World| {
                let mut settings = world.resource_mut::<viewport_overlays::OverlaySettings>();
                settings.show_alignment_guides = !settings.show_alignment_guides;
            });
        }
        "view.collider_gizmos" => {
            commands.queue(|world: &mut World| {
                let mut config =
                    world.resource_mut::<jackdaw_avian_integration::PhysicsOverlayConfig>();
                config.show_colliders = !config.show_colliders;
            });
        }
        "view.hierarchy_arrows" => {
            commands.queue(|world: &mut World| {
                let mut config =
                    world.resource_mut::<jackdaw_avian_integration::PhysicsOverlayConfig>();
                config.show_hierarchy_arrows = !config.show_hierarchy_arrows;
            });
        }
        "add.cube" => {
            commands.queue(|world: &mut World| {
                entity_ops::create_entity_in_world(world, entity_ops::EntityTemplate::Cube);
            });
        }
        "add.sphere" => {
            commands.queue(|world: &mut World| {
                entity_ops::create_entity_in_world(world, entity_ops::EntityTemplate::Sphere);
            });
        }
        "add.point_light" => {
            commands.queue(|world: &mut World| {
                entity_ops::create_entity_in_world(world, entity_ops::EntityTemplate::PointLight);
            });
        }
        "add.directional_light" => {
            commands.queue(|world: &mut World| {
                entity_ops::create_entity_in_world(
                    world,
                    entity_ops::EntityTemplate::DirectionalLight,
                );
            });
        }
        "add.spot_light" => {
            commands.queue(|world: &mut World| {
                entity_ops::create_entity_in_world(world, entity_ops::EntityTemplate::SpotLight);
            });
        }
        "add.camera" => {
            commands.queue(|world: &mut World| {
                entity_ops::create_entity_in_world(world, entity_ops::EntityTemplate::Camera3d);
            });
        }
        "add.empty" => {
            commands.queue(|world: &mut World| {
                entity_ops::create_entity_in_world(world, entity_ops::EntityTemplate::Empty);
            });
        }
        "add.navmesh" => {
            commands.queue(|world: &mut World| {
                let mut system_state: SystemState<(Commands, ResMut<Selection>)> =
                    SystemState::new(world);
                let (mut commands, mut selection) = system_state.get_mut(world);
                let entity = navmesh::spawn_navmesh_entity(&mut commands);
                selection.select_single(&mut commands, entity);
                system_state.apply(world);
            });
        }
        "add.terrain" => {
            commands.queue(|world: &mut World| {
                let mut system_state: SystemState<(Commands, ResMut<Selection>)> =
                    SystemState::new(world);
                let (mut commands, mut selection) = system_state.get_mut(world);
                let entity = terrain::spawn_terrain_entity(&mut commands);
                selection.select_single(&mut commands, entity);
                system_state.apply(world);
                scene_io::register_entity_in_ast(world, entity);
            });
        }
        "add.prefab" => {
            commands.queue(|world: &mut World| {
                crate::prefab_picker::open_prefab_picker(world);
            });
        }
        _ => {}
    }
}

fn cleanup_editor(world: &mut World) {
    // 1. Clear scene entities
    scene_io::clear_scene_entities(world);

    // 2. Despawn all EditorEntity entities
    let editor_entities: Vec<Entity> = world
        .query_filtered::<Entity, With<EditorEntity>>()
        .iter(world)
        .collect();
    for entity in editor_entities {
        if let Ok(ec) = world.get_entity_mut(entity) {
            ec.despawn();
        }
    }

    // 3. Despawn Camera2d entities (editor UI camera)
    let cameras: Vec<Entity> = world
        .query_filtered::<Entity, With<Camera2d>>()
        .iter(world)
        .collect();
    for entity in cameras {
        if let Ok(ec) = world.get_entity_mut(entity) {
            ec.despawn();
        }
    }

    // 4. Despawn any open dialogs
    let dialogs: Vec<Entity> = world
        .query_filtered::<Entity, With<jackdaw_feathers::dialog::EditorDialog>>()
        .iter(world)
        .collect();
    for entity in dialogs {
        if let Ok(ec) = world.get_entity_mut(entity) {
            ec.despawn();
        }
    }

    // 5. Reset resources
    world.insert_resource(scene_io::SceneFilePath::default());
    world.insert_resource(scene_io::SceneDirtyState::default());
    world.insert_resource(Selection::default());
    world.insert_resource(commands::CommandHistory::default());

    // 6. Remove project root
    world.remove_resource::<project::ProjectRoot>();

    // 7. Reset menu bar state
    let dropdown_to_despawn = {
        let mut menu_state = world.resource_mut::<jackdaw_widgets::menu_bar::MenuBarState>();
        menu_state.open_menu = None;
        menu_state.dropdown_entity.take()
    };
    if let Some(dropdown) = dropdown_to_despawn {
        if let Ok(ec) = world.get_entity_mut(dropdown) {
            ec.despawn();
        }
    }
}

fn open_recent_dialog(world: &mut World) {
    let recent = project::read_recent_projects();
    if recent.projects.is_empty() {
        return;
    }

    let mut dialog_event = jackdaw_feathers::dialog::OpenDialogEvent::new("Open Recent", "")
        .without_cancel()
        .with_close_button(true)
        .without_content_padding();
    dialog_event.action = None;
    world.commands().trigger(dialog_event);
    world.flush();

    // Find the DialogChildrenSlot and spawn rows inside it
    let slot_entity = world
        .query_filtered::<Entity, With<jackdaw_feathers::dialog::DialogChildrenSlot>>()
        .iter(world)
        .next();

    let Some(slot_entity) = slot_entity else {
        return;
    };

    let editor_font = world
        .resource::<jackdaw_feathers::icons::EditorFont>()
        .0
        .clone();

    for entry in &recent.projects {
        let path = entry.path.clone();
        let name = entry.name.clone();
        let path_display = entry.path.to_string_lossy().to_string();
        let font = editor_font.clone();

        let row = world
            .commands()
            .spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    width: Val::Percent(100.0),
                    padding: UiRect::all(Val::Px(10.0)),
                    row_gap: Val::Px(2.0),
                    ..Default::default()
                },
                BackgroundColor(jackdaw_feathers::tokens::TOOLBAR_BG),
                children![
                    (
                        Text::new(name),
                        TextFont {
                            font: font.clone(),
                            font_size: jackdaw_feathers::tokens::FONT_LG,
                            ..Default::default()
                        },
                        TextColor(jackdaw_feathers::tokens::TEXT_PRIMARY),
                        Pickable::IGNORE,
                    ),
                    (
                        Text::new(path_display),
                        TextFont {
                            font,
                            font_size: jackdaw_feathers::tokens::FONT_SM,
                            ..Default::default()
                        },
                        TextColor(jackdaw_feathers::tokens::TEXT_SECONDARY),
                        Pickable::IGNORE,
                    ),
                ],
            ))
            .id();

        // Hover effects
        world.commands().entity(row).observe(
            |hover: On<Pointer<Over>>, mut bg: Query<&mut BackgroundColor>| {
                if let Ok(mut bg) = bg.get_mut(hover.event_target()) {
                    bg.0 = jackdaw_feathers::tokens::HOVER_BG;
                }
            },
        );
        world.commands().entity(row).observe(
            |out: On<Pointer<Out>>, mut bg: Query<&mut BackgroundColor>| {
                if let Ok(mut bg) = bg.get_mut(out.event_target()) {
                    bg.0 = jackdaw_feathers::tokens::TOOLBAR_BG;
                }
            },
        );

        // Click: open the project
        world.commands().entity(row).observe(
            move |_: On<Pointer<Click>>, mut commands: Commands| {
                let path = path.clone();
                commands.insert_resource(project_select::PendingAutoOpen { path: path.clone() });
                commands.trigger(jackdaw_feathers::dialog::CloseDialogEvent);
                commands.queue(move |world: &mut World| {
                    world
                        .resource_mut::<NextState<AppState>>()
                        .set(AppState::ProjectSelect);
                });
            },
        );

        world.commands().entity(slot_entity).add_child(row);
    }

    world.flush();
}

const SCROLL_LINE_HEIGHT: f32 = 21.0;

#[derive(EntityEvent, Debug)]
#[entity_event(propagate, auto_propagate)]
struct Scroll {
    entity: Entity,
    delta: Vec2,
}

fn send_scroll_events(
    mut mouse_wheel: MessageReader<MouseWheel>,
    hover_map: Res<HoverMap>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
) {
    for event in mouse_wheel.read() {
        let mut delta = -Vec2::new(event.x, event.y);
        if event.unit == MouseScrollUnit::Line {
            delta *= SCROLL_LINE_HEIGHT;
        }
        if keyboard.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]) {
            std::mem::swap(&mut delta.x, &mut delta.y);
        }
        for pointer_map in hover_map.values() {
            for entity in pointer_map.keys().copied() {
                commands.trigger(Scroll { entity, delta });
            }
        }
    }
}

fn on_scroll(
    mut scroll: On<Scroll>,
    mut query: Query<(&mut ScrollPosition, &Node, &ComputedNode)>,
) {
    let Ok((mut scroll_position, node, computed)) = query.get_mut(scroll.entity) else {
        return;
    };
    let max_offset = (computed.content_size() - computed.size()) * computed.inverse_scale_factor();
    let delta = &mut scroll.delta;

    if node.overflow.x == OverflowAxis::Scroll && delta.x != 0. {
        let at_limit = if delta.x > 0. {
            scroll_position.x >= max_offset.x
        } else {
            scroll_position.x <= 0.
        };
        if !at_limit {
            scroll_position.x += delta.x;
            delta.x = 0.;
        }
    }

    if node.overflow.y == OverflowAxis::Scroll && delta.y != 0. {
        let at_limit = if delta.y > 0. {
            scroll_position.y >= max_offset.y
        } else {
            scroll_position.y <= 0.
        };
        if !at_limit {
            scroll_position.y += delta.y;
            delta.y = 0.;
        }
    }

    if *delta == Vec2::ZERO {
        scroll.propagate(false);
    }
}
