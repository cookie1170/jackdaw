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
pub mod prefab_picker;
pub mod project;
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
                modal_transform::ModalTransformPlugin,
                custom_properties::CustomPropertiesPlugin,
                entity_templates::EntityTemplatesPlugin,
                brush::BrushPlugin,
                material_browser::MaterialBrowserPlugin,
                draw_brush::DrawBrushPlugin,
                face_grid::FaceGridPlugin,
                alignment_guides::AlignmentGuidesPlugin,
                navmesh::NavmeshPlugin,
                terrain::TerrainPlugin,
                prefab_picker::PrefabPickerPlugin,
                remote::RemoteConnectionPlugin,
            ))
            .configure_sets(
                Update,
                EditorInteraction
                    .run_if(in_state(AppState::Editor))
                    .run_if(no_dialog_open),
            )
            .insert_resource(UiTheme(create_dark_theme()))
            .init_resource::<layout::ActiveWorkspace>()
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
                    layout::update_workspace_visibility,
                    layout::update_tab_highlights,
                    auto_hide_internal_entities,
                )
                    .run_if(in_state(AppState::Editor)),
            )
            .add_observer(on_scroll)
            .add_observer(handle_menu_action);
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
            // Skip GLTF descendants — they'll be shown in the hierarchy
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
