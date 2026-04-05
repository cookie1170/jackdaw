use bevy::{
    feathers::{
        theme::{ThemeBackgroundColor, ThemedText},
        tokens as bevy_tokens,
    },
    picking::hover::Hovered,
    prelude::*,
    ui_widgets::observe,
};
use jackdaw_feathers::{
    icons::{Icon, IconFont},
    menu_bar, panel_header, popover, separator, split_panel, status_bar,
    text_edit::{self, TextEditProps},
    tokens,
    tree_view::tree_container_drop_observers,
};

use crate::{
    EditorEntity,
    asset_browser::{self, ActiveTooltip},
    brush::{BrushEditMode, BrushSelection, EditMode},
    draw_brush::DrawBrushState,
    gizmos::{GizmoMode, GizmoSpace},
    hierarchy::{HierarchyPanel, HierarchyShowAllButton, HierarchyTreeContainer},
    inspector::Inspector,
    material_browser,
    remote::ConnectionManager,
    selection::Selection,
    viewport::SceneViewport,
};

/// Which workspace tab is active.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq)]
pub enum ActiveWorkspace {
    #[default]
    SceneEditor,
    RemoteDebug,
}

/// Marker for the workspace tab bar row.
#[derive(Component)]
pub struct WorkspaceTabBar;

/// Marker for a workspace tab, storing which workspace it activates.
#[derive(Component)]
pub struct WorkspaceTab(pub ActiveWorkspace);

/// Marker for the scene editor workspace container.
#[derive(Component)]
pub struct SceneEditorWorkspace;

/// Marker for the remote debug workspace container.
#[derive(Component)]
pub struct RemoteDebugWorkspace;

/// Marker on the hierarchy filter text input
#[derive(Component)]
pub struct HierarchyFilter;

/// Marker for the toolbar
#[derive(Component)]
pub struct Toolbar;

/// Marker for gizmo mode buttons
#[derive(Component)]
pub struct GizmoModeButton(pub GizmoMode);

/// Marker for gizmo space toggle
#[derive(Component)]
pub struct GizmoSpaceButton;

/// Marker for edit mode/tool buttons in the toolbar
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum EditToolButton {
    Object,
    Draw,
    Vertex,
    Edge,
    Face,
    Clip,
}

/// Stores tooltip text for toolbar buttons (used with `Hovered` component).
#[derive(Component)]
pub struct ToolbarTooltip(pub String);

/// Marker for keybind helper button
#[derive(Component)]
pub struct KeybindHelpButton;

/// Resource tracking the keybind help popover entity
#[derive(Resource, Default)]
pub struct KeybindHelpPopover {
    pub entity: Option<Entity>,
}

pub fn editor_layout(icon_font: &IconFont) -> impl Bundle {
    let font = icon_font.0.clone();
    (
        EditorEntity,
        ThemeBackgroundColor(bevy_tokens::WINDOW_BG),
        Node {
            width: percent(100),
            height: percent(100),
            flex_direction: FlexDirection::Column,
            ..Default::default()
        },
        children![
            // Menu bar (fixed height, populated in spawn_layout)
            menu_bar::menu_bar_shell(),
            // Workspace tab bar
            workspace_tab_bar(),
            // Content container (flex grow) — holds both workspaces
            (
                EditorEntity,
                Node {
                    width: percent(100),
                    flex_grow: 1.0,
                    min_height: px(0.0),
                    flex_direction: FlexDirection::Column,
                    ..Default::default()
                },
                children![
                    // Scene Editor workspace (active by default)
                    (
                        SceneEditorWorkspace,
                        EditorEntity,
                        Node {
                            width: percent(100),
                            flex_grow: 1.0,
                            min_height: px(0.0),
                            flex_direction: FlexDirection::Column,
                            display: Display::Flex,
                            ..Default::default()
                        },
                        // Vertical split: main area (top) + bottom panels (bottom)
                        split_panel::panel_group(
                            0.15,
                            (
                                Spawn((split_panel::panel(4), main_area(font.clone()))),
                                Spawn(split_panel::panel_handle()),
                                Spawn((split_panel::panel(1), bottom_panels(font))),
                            ),
                        ),
                    ),
                    // Remote Debug workspace (hidden by default)
                    (
                        RemoteDebugWorkspace,
                        EditorEntity,
                        Node {
                            width: percent(100),
                            flex_grow: 1.0,
                            min_height: px(0.0),
                            flex_direction: FlexDirection::Column,
                            display: Display::None,
                            ..Default::default()
                        },
                        split_panel::panel_group(
                            0.2,
                            (
                                Spawn((
                                    split_panel::panel(1),
                                    crate::remote::entity_browser::remote_debug_workspace_content(),
                                )),
                                Spawn(split_panel::panel_handle()),
                                Spawn((
                                    split_panel::panel(1),
                                    crate::remote::remote_inspector::remote_inspector(),
                                )),
                            ),
                        ),
                    )
                ],
            ),
            // Status bar (fixed height) with connection indicator
            editor_status_bar()
        ],
    )
}

fn workspace_tab_bar() -> impl Bundle {
    (
        WorkspaceTabBar,
        EditorEntity,
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            width: percent(100),
            height: px(28.0),
            flex_shrink: 0.0,
            padding: UiRect::horizontal(px(tokens::SPACING_SM)),
            column_gap: px(tokens::SPACING_XS),
            ..Default::default()
        },
        BackgroundColor(tokens::TOOLBAR_BG),
        children![
            workspace_tab("Scene", ActiveWorkspace::SceneEditor, true),
            workspace_tab("Remote", ActiveWorkspace::RemoteDebug, false),
        ],
    )
}

fn workspace_tab(label: &str, workspace: ActiveWorkspace, active: bool) -> impl Bundle {
    let bg = if active {
        tokens::SELECTED_BG
    } else {
        tokens::TOOLBAR_BUTTON_BG
    };
    (
        WorkspaceTab(workspace),
        Interaction::default(),
        Node {
            padding: UiRect::axes(px(tokens::SPACING_LG), px(tokens::SPACING_XS)),
            border_radius: BorderRadius::all(px(tokens::BORDER_RADIUS_SM)),
            ..Default::default()
        },
        BackgroundColor(bg),
        children![(
            Text::new(label.to_string()),
            TextFont {
                font_size: tokens::FONT_SM,
                ..Default::default()
            },
            TextColor(if active {
                tokens::TEXT_PRIMARY
            } else {
                tokens::TEXT_SECONDARY
            }),
        )],
        observe(
            move |_: On<Pointer<Click>>,
                  mut workspace_res: ResMut<ActiveWorkspace>,
                  manager: Res<ConnectionManager>| {
                // Only allow switching to Remote Debug when connected
                if workspace == ActiveWorkspace::RemoteDebug && !manager.is_connected() {
                    return;
                }
                *workspace_res = workspace;
            },
        ),
    )
}

fn main_area(icon_font: Handle<Font>) -> impl Bundle {
    (
        EditorEntity,
        Node {
            width: percent(100),
            height: percent(100),
            ..Default::default()
        },
        // Horizontal split: hierarchy | viewport | inspector
        split_panel::panel_group(
            0.2,
            (
                Spawn((split_panel::panel(1), entity_heiarchy(icon_font.clone()))),
                Spawn(split_panel::panel_handle()),
                Spawn((split_panel::panel(4), viewport_with_toolbar(icon_font))),
                Spawn(split_panel::panel_handle()),
                Spawn((split_panel::panel(1), entity_inspector())),
            ),
        ),
    )
}

fn viewport_with_toolbar(icon_font: Handle<Font>) -> impl Bundle {
    (
        EditorEntity,
        Node {
            height: percent(100),
            flex_direction: FlexDirection::Column,
            overflow: Overflow::clip(),
            ..Default::default()
        },
        children![
            toolbar(icon_font),
            crate::navmesh::toolbar::navmesh_toolbar(),
            crate::terrain::toolbar::terrain_toolbar(),
            scene_view(),
        ],
    )
}

fn toolbar(icon_font: Handle<Font>) -> impl Bundle {
    let f = icon_font.clone();
    (
        Toolbar,
        EditorEntity,
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            padding: UiRect::axes(px(tokens::SPACING_MD), px(tokens::SPACING_SM)),
            column_gap: px(tokens::SPACING_SM),
            width: percent(100),
            height: px(32.0),
            flex_shrink: 0.0,
            ..Default::default()
        },
        BackgroundColor(tokens::TOOLBAR_BG),
        children![
            // Gizmo mode buttons
            toolbar_button(
                Icon::Move,
                "",
                GizmoMode::Translate,
                icon_font.clone(),
                "Move (Esc)"
            ),
            toolbar_button(
                Icon::RotateCw,
                "R",
                GizmoMode::Rotate,
                icon_font.clone(),
                "Rotate (R)"
            ),
            toolbar_button(
                Icon::Scaling,
                "T",
                GizmoMode::Scale,
                icon_font.clone(),
                "Scale (T)"
            ),
            // Separator
            separator::separator(separator::SeparatorProps::vertical()),
            // Space toggle
            toolbar_space_button(f.clone()),
            // Separator
            separator::separator(separator::SeparatorProps::vertical()),
            // Edit mode buttons
            toolbar_edit_button(
                Icon::MousePointer,
                EditToolButton::Object,
                f.clone(),
                "Object Mode"
            ),
            toolbar_edit_button(Icon::Box, EditToolButton::Draw, f.clone(), "Draw Brush (B)"),
            toolbar_edit_button(
                Icon::CircleDot,
                EditToolButton::Vertex,
                f.clone(),
                "Vertex Mode (1)"
            ),
            toolbar_edit_button(
                Icon::GitCommitHorizontal,
                EditToolButton::Edge,
                f.clone(),
                "Edge Mode (2)"
            ),
            toolbar_edit_button(
                Icon::Hexagon,
                EditToolButton::Face,
                f.clone(),
                "Face Mode (3)"
            ),
            toolbar_edit_button(
                Icon::ScissorsLineDashed,
                EditToolButton::Clip,
                f.clone(),
                "Clip Mode (4)"
            ),
            // Spacer pushes help button to the right
            (Node {
                flex_grow: 1.0,
                ..Default::default()
            },),
            // Keybind help button
            toolbar_help_button(f),
        ],
    )
}

fn toolbar_button(
    icon: Icon,
    label: &str,
    mode: GizmoMode,
    font: Handle<Font>,
    tooltip: &str,
) -> impl Bundle {
    let label = label.to_string();
    (
        GizmoModeButton(mode),
        Hovered::default(),
        ToolbarTooltip(tooltip.into()),
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(tokens::SPACING_XS),
            padding: UiRect::axes(px(tokens::SPACING_MD), px(tokens::SPACING_XS)),
            border_radius: BorderRadius::all(px(tokens::BORDER_RADIUS_SM)),
            ..Default::default()
        },
        BackgroundColor(tokens::TOOLBAR_BUTTON_BG),
        children![
            (
                Text::new(String::from(icon.unicode())),
                TextFont {
                    font,
                    font_size: tokens::FONT_MD,
                    ..Default::default()
                },
                TextColor(tokens::TEXT_SECONDARY),
            ),
            (
                Text::new(label),
                TextFont {
                    font_size: tokens::FONT_SM,
                    ..Default::default()
                },
                ThemedText,
            )
        ],
        observe(
            move |_: On<Pointer<Click>>, mut gizmo_mode: ResMut<GizmoMode>| {
                *gizmo_mode = mode;
            },
        ),
    )
}

fn toolbar_space_button(icon_font: Handle<Font>) -> impl Bundle {
    (
        GizmoSpaceButton,
        Hovered::default(),
        ToolbarTooltip("Toggle World/Local (X)".into()),
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(tokens::SPACING_XS),
            padding: UiRect::axes(px(tokens::SPACING_MD), px(tokens::SPACING_XS)),
            border_radius: BorderRadius::all(px(tokens::BORDER_RADIUS_SM)),
            ..Default::default()
        },
        BackgroundColor(tokens::TOOLBAR_BUTTON_BG),
        children![
            (
                Text::new(String::from(Icon::Globe.unicode())),
                TextFont {
                    font: icon_font,
                    font_size: tokens::FONT_MD,
                    ..Default::default()
                },
                TextColor(tokens::TEXT_SECONDARY),
            ),
            (
                Text::new("World"),
                TextFont {
                    font_size: tokens::FONT_SM,
                    ..Default::default()
                },
                ThemedText,
            )
        ],
        observe(|_: On<Pointer<Click>>, mut space: ResMut<GizmoSpace>| {
            *space = match *space {
                GizmoSpace::World => GizmoSpace::Local,
                GizmoSpace::Local => GizmoSpace::World,
            };
        }),
    )
}

fn toolbar_edit_button(
    icon: Icon,
    tool: EditToolButton,
    font: Handle<Font>,
    tooltip: &str,
) -> impl Bundle {
    (
        tool,
        Hovered::default(),
        ToolbarTooltip(tooltip.into()),
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            padding: UiRect::axes(px(tokens::SPACING_MD), px(tokens::SPACING_XS)),
            border_radius: BorderRadius::all(px(tokens::BORDER_RADIUS_SM)),
            ..Default::default()
        },
        BackgroundColor(tokens::TOOLBAR_BUTTON_BG),
        children![(
            Text::new(String::from(icon.unicode())),
            TextFont {
                font,
                font_size: tokens::FONT_MD,
                ..Default::default()
            },
            TextColor(tokens::TEXT_SECONDARY),
        )],
        observe(
            move |_: On<Pointer<Click>>,
                  mut edit_mode: ResMut<EditMode>,
                  mut brush_selection: ResMut<BrushSelection>,
                  mut draw_state: ResMut<DrawBrushState>,
                  selection: Res<Selection>,
                  brushes: Query<(), With<jackdaw_jsn::Brush>>| {
                match tool {
                    EditToolButton::Object => {
                        *edit_mode = EditMode::Object;
                        brush_selection.entity = None;
                        brush_selection.faces.clear();
                        brush_selection.vertices.clear();
                        brush_selection.edges.clear();
                        draw_state.active = None;
                    }
                    EditToolButton::Draw => {
                        // Toggle draw mode
                        if draw_state.active.is_some() {
                            draw_state.active = None;
                        } else {
                            // Exit brush edit mode if active
                            if *edit_mode != EditMode::Object {
                                *edit_mode = EditMode::Object;
                                brush_selection.entity = None;
                                brush_selection.faces.clear();
                                brush_selection.vertices.clear();
                                brush_selection.edges.clear();
                            }
                            // Check if a brush is selected for append mode
                            let append_target =
                                selection.primary().filter(|&e| brushes.contains(e));
                            draw_state.active = Some(crate::draw_brush::ActiveDraw {
                                corner1: Vec3::ZERO,
                                corner2: Vec3::ZERO,
                                depth: 0.0,
                                phase: crate::draw_brush::DrawPhase::PlacingFirstCorner,
                                mode: crate::draw_brush::DrawMode::Add,
                                plane: crate::draw_brush::DrawPlane {
                                    origin: Vec3::ZERO,
                                    normal: Vec3::Y,
                                    axis_u: Vec3::X,
                                    axis_v: Vec3::Z,
                                },
                                extrude_start_cursor: Vec2::ZERO,
                                plane_locked: false,
                                cursor_on_plane: None,
                                append_target,
                                drag_footprint: false,
                                press_screen_pos: None,
                                polygon_vertices: Vec::new(),
                                polygon_cursor: None,
                                diagonal_snap: false,
                                cached_face_hit: None,
                            });
                        }
                    }
                    EditToolButton::Vertex
                    | EditToolButton::Edge
                    | EditToolButton::Face
                    | EditToolButton::Clip => {
                        // Cancel draw mode if active
                        draw_state.active = None;

                        let target_mode = match tool {
                            EditToolButton::Vertex => BrushEditMode::Vertex,
                            EditToolButton::Edge => BrushEditMode::Edge,
                            EditToolButton::Face => BrushEditMode::Face,
                            EditToolButton::Clip => BrushEditMode::Clip,
                            _ => unreachable!(),
                        };

                        if let EditMode::BrushEdit(current) = *edit_mode {
                            if current == target_mode {
                                // Same mode: toggle off
                                *edit_mode = EditMode::Object;
                                brush_selection.entity = None;
                                brush_selection.faces.clear();
                                brush_selection.vertices.clear();
                                brush_selection.edges.clear();
                            } else {
                                // Switch sub-mode
                                *edit_mode = EditMode::BrushEdit(target_mode);
                                brush_selection.faces.clear();
                                brush_selection.vertices.clear();
                                brush_selection.edges.clear();
                            }
                        } else {
                            // Enter edit on primary if it's a brush
                            if let Some(entity) =
                                selection.primary().filter(|&e| brushes.contains(e))
                            {
                                *edit_mode = EditMode::BrushEdit(target_mode);
                                brush_selection.entity = Some(entity);
                                brush_selection.faces.clear();
                                brush_selection.vertices.clear();
                                brush_selection.edges.clear();
                            }
                        }
                    }
                }
            },
        ),
    )
}

fn toolbar_help_button(icon_font: Handle<Font>) -> impl Bundle {
    (
        KeybindHelpButton,
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            padding: UiRect::axes(px(tokens::SPACING_MD), px(tokens::SPACING_XS)),
            border_radius: BorderRadius::all(px(tokens::BORDER_RADIUS_SM)),
            ..Default::default()
        },
        BackgroundColor(tokens::TOOLBAR_BUTTON_BG),
        children![(
            Text::new(String::from(Icon::Keyboard.unicode())),
            TextFont {
                font: icon_font,
                font_size: tokens::FONT_MD,
                ..Default::default()
            },
            TextColor(tokens::TEXT_SECONDARY),
        )],
        observe(
            |trigger: On<Pointer<Click>>,
             mut commands: Commands,
             mut popover_state: ResMut<KeybindHelpPopover>,
             registry: Res<crate::keybinds::KeybindRegistry>| {
                // Toggle: if popover exists, despawn it
                if let Some(entity) = popover_state.entity.take() {
                    if let Ok(mut ec) = commands.get_entity(entity) {
                        ec.despawn();
                    }
                    return;
                }

                let anchor = trigger.event_target();

                let popover_entity = commands
                    .spawn(popover::popover(
                        popover::PopoverProps::new(anchor)
                            .with_placement(popover::PopoverPlacement::BottomEnd)
                            .with_padding(12.0)
                            .with_z_index(200),
                    ))
                    .with_children(|parent| {
                        parent
                            .spawn(Node {
                                flex_direction: FlexDirection::Column,
                                max_height: px(500.0),
                                overflow: Overflow::scroll_y(),
                                ..Default::default()
                            })
                            .with_children(|scroll_parent| {
                                spawn_keybind_help_content(scroll_parent, &registry);
                            });
                    })
                    .id();

                popover_state.entity = Some(popover_entity);
            },
        ),
    )
}

fn spawn_keybind_help_content(
    parent: &mut ChildSpawnerCommands,
    registry: &crate::keybinds::KeybindRegistry,
) {
    use jackdaw_commands::keybinds::EditorAction;

    // Mouse-only entries that can't be expressed as keybinds, grouped by category
    let mouse_entries: &[(&str, &[(&str, &str)])] = &[
        (
            "Navigation",
            &[
                ("RMB + Drag", "Look around"),
                ("Shift", "Double speed"),
                ("Scroll", "Dolly forward/back"),
                ("RMB + Scroll", "Adjust move speed"),
            ],
        ),
        (
            "Selection",
            &[
                ("LMB", "Select entity"),
                ("Ctrl+Click", "Toggle multi-select"),
                ("Shift+LMB Drag", "Box select"),
            ],
        ),
        (
            "Transform",
            &[
                ("MMB", "Toggle snap"),
                ("Ctrl", "Toggle snap (during drag)"),
            ],
        ),
        (
            "Brush Edit",
            &[
                ("Shift+Click", "Multi-select"),
                ("Click+Drag", "Move selected"),
            ],
        ),
        (
            "Draw Brush",
            &[
                ("Click", "Place vertex / advance"),
                ("Right-click", "Cancel"),
            ],
        ),
        ("View", &[("Ctrl+Alt+Scroll", "Grid size")]),
    ];

    // Build sections dynamically from registry
    let category_order = [
        "File",
        "Entity",
        "Transform",
        "Brush Edit",
        "Draw Brush",
        "CSG",
        "Gizmo",
        "Navigation",
        "View",
    ];

    // Also include Selection between Navigation and View
    let display_order = [
        "Navigation",
        "Selection",
        "Transform",
        "Entity",
        "Brush Edit",
        "CSG",
        "Draw Brush",
        "Gizmo",
        "View",
        "File",
    ];

    for &section in &display_order {
        let mut entries: Vec<(String, String)> = Vec::new();

        // Add mouse entries for this section
        for (cat, mouse_binds) in mouse_entries {
            if *cat == section {
                for (key, desc) in *mouse_binds {
                    entries.push((key.to_string(), desc.to_string()));
                }
            }
        }

        // Add keybind entries for this section (if it's a real category)
        if category_order.contains(&section) {
            for &action in EditorAction::all() {
                if action.category() != section {
                    continue;
                }
                let bindings = registry.bindings.get(&action).cloned().unwrap_or_default();
                if bindings.is_empty() {
                    continue;
                }
                let key_str = bindings
                    .iter()
                    .map(|b| b.to_string())
                    .collect::<Vec<_>>()
                    .join(" / ");
                entries.push((key_str, action.to_string()));
            }
        }

        if entries.is_empty() {
            continue;
        }

        // Section header
        parent.spawn((
            Text::new(section),
            TextFont {
                font_size: tokens::FONT_SM,
                ..Default::default()
            },
            TextColor(tokens::TEXT_PRIMARY),
            Node {
                margin: UiRect::top(px(tokens::SPACING_SM)),
                ..Default::default()
            },
        ));

        for (key, desc) in &entries {
            parent.spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::SpaceBetween,
                    column_gap: px(tokens::SPACING_LG),
                    width: px(260.0),
                    ..Default::default()
                },
                children![
                    (
                        Text::new(key.clone()),
                        TextFont {
                            font_size: tokens::FONT_SM,
                            ..Default::default()
                        },
                        TextColor(tokens::TEXT_PRIMARY),
                    ),
                    (
                        Text::new(desc.clone()),
                        TextFont {
                            font_size: tokens::FONT_SM,
                            ..Default::default()
                        },
                        TextColor(tokens::TEXT_SECONDARY),
                    )
                ],
            ));
        }
    }
}

fn entity_heiarchy(icon_font: Handle<Font>) -> impl Bundle {
    (
        HierarchyPanel,
        Node {
            height: percent(100),
            flex_direction: FlexDirection::Column,
            overflow: Overflow::clip(),
            ..Default::default()
        },
        BackgroundColor(tokens::PANEL_BG),
        children![
            panel_header::panel_header("Hierarchy"),
            (
                Node {
                    flex_direction: FlexDirection::Column,
                    flex_grow: 1.0,
                    min_height: px(0.0),
                    padding: UiRect::all(px(tokens::SPACING_SM)),
                    ..Default::default()
                },
                children![
                    // Filter row: text input + show-all toggle button
                    (
                        Node {
                            flex_direction: FlexDirection::Row,
                            align_items: AlignItems::Center,
                            column_gap: px(tokens::SPACING_XS),
                            width: percent(100),
                            ..Default::default()
                        },
                        children![
                            (
                                Node {
                                    flex_grow: 1.0,
                                    ..Default::default()
                                },
                                children![(
                                    HierarchyFilter,
                                    text_edit::text_edit(
                                        TextEditProps::default()
                                            .with_placeholder("Filter entities")
                                            .allow_empty()
                                    ),
                                )],
                            ),
                            // Show all / named only toggle
                            (
                                HierarchyShowAllButton,
                                Interaction::default(),
                                Node {
                                    width: px(24.0),
                                    height: px(24.0),
                                    justify_content: JustifyContent::Center,
                                    align_items: AlignItems::Center,
                                    border_radius: BorderRadius::all(px(tokens::BORDER_RADIUS_SM),),
                                    ..Default::default()
                                },
                                children![(
                                    Text::new(String::from(Icon::Eye.unicode())),
                                    TextFont {
                                        font: icon_font,
                                        font_size: 14.0,
                                        ..Default::default()
                                    },
                                    TextColor(tokens::TEXT_SECONDARY),
                                )],
                            ),
                        ],
                    ),
                    (
                        HierarchyTreeContainer,
                        Node {
                            flex_direction: FlexDirection::Column,
                            width: percent(100),
                            flex_grow: 1.0,
                            min_height: px(0.0),
                            overflow: Overflow::scroll_y(),
                            margin: UiRect::top(px(tokens::SPACING_SM)),
                            ..Default::default()
                        },
                        BackgroundColor(Color::NONE),
                        tree_container_drop_observers(),
                    )
                ],
            )
        ],
    )
}

fn scene_view() -> impl Bundle {
    (
        EditorEntity,
        SceneViewport,
        Node {
            width: percent(100),
            flex_grow: 1.0,
            ..Default::default()
        },
    )
}

/// Updates toolbar button backgrounds to highlight the active gizmo mode.
pub fn update_toolbar_highlights(
    mode: Res<GizmoMode>,
    mut buttons: Query<(&GizmoModeButton, &mut BackgroundColor)>,
) {
    if !mode.is_changed() {
        return;
    }
    for (button, mut bg) in &mut buttons {
        bg.0 = if button.0 == *mode {
            tokens::SELECTED_BG
        } else {
            tokens::TOOLBAR_BUTTON_BG
        };
    }
}

/// Shows/hides toolbar tooltips based on `Hovered` state (flicker-free).
pub fn update_toolbar_tooltips(
    buttons: Query<(Entity, &ToolbarTooltip, &Hovered), Changed<Hovered>>,
    mut commands: Commands,
    mut active: ResMut<ActiveTooltip>,
) {
    for (entity, tooltip, hovered) in &buttons {
        if hovered.get() {
            if let Some(old) = active.0.take() {
                commands.entity(old).try_despawn();
            }
            let tip = commands
                .spawn(popover::popover(
                    popover::PopoverProps::new(entity)
                        .with_placement(popover::PopoverPlacement::Bottom)
                        .with_padding(4.0)
                        .with_z_index(300),
                ))
                .id();
            commands.spawn((
                Text::new(tooltip.0.clone()),
                TextFont {
                    font_size: tokens::FONT_SM,
                    ..Default::default()
                },
                TextColor(tokens::TEXT_PRIMARY),
                ChildOf(tip),
            ));
            active.0 = Some(tip);
        } else if let Some(old) = active.0.take() {
            commands.entity(old).try_despawn();
        }
    }
}

/// Updates the gizmo space toggle button label.
pub fn update_space_toggle_label(
    space: Res<GizmoSpace>,
    buttons: Query<&Children, With<GizmoSpaceButton>>,
    mut texts: Query<&mut Text, With<ThemedText>>,
) {
    if !space.is_changed() {
        return;
    }
    let label = match *space {
        GizmoSpace::World => "World",
        GizmoSpace::Local => "Local",
    };
    for children in &buttons {
        for child in children.iter() {
            if let Ok(mut text) = texts.get_mut(child) {
                text.0 = label.to_string();
                return;
            }
        }
    }
}

/// Updates edit tool button backgrounds to highlight the active edit mode/draw state.
pub fn update_edit_tool_highlights(
    edit_mode: Res<EditMode>,
    draw_state: Res<DrawBrushState>,
    mut buttons: Query<(&EditToolButton, &mut BackgroundColor)>,
) {
    if !edit_mode.is_changed() && !draw_state.is_changed() {
        return;
    }
    let draw_active = draw_state.active.is_some();
    for (button, mut bg) in &mut buttons {
        let active = match button {
            EditToolButton::Object => !draw_active && *edit_mode == EditMode::Object,
            EditToolButton::Draw => draw_active,
            EditToolButton::Vertex => {
                !draw_active && *edit_mode == EditMode::BrushEdit(BrushEditMode::Vertex)
            }
            EditToolButton::Edge => {
                !draw_active && *edit_mode == EditMode::BrushEdit(BrushEditMode::Edge)
            }
            EditToolButton::Face => {
                !draw_active && *edit_mode == EditMode::BrushEdit(BrushEditMode::Face)
            }
            EditToolButton::Clip => {
                !draw_active && *edit_mode == EditMode::BrushEdit(BrushEditMode::Clip)
            }
        };
        bg.0 = if active {
            tokens::SELECTED_BG
        } else {
            tokens::TOOLBAR_BUTTON_BG
        };
    }
}

/// Toggle workspace container visibility when ActiveWorkspace changes.
pub fn update_workspace_visibility(
    workspace: Res<ActiveWorkspace>,
    mut scene_editors: Query<
        &mut Node,
        (With<SceneEditorWorkspace>, Without<RemoteDebugWorkspace>),
    >,
    mut remote_debugs: Query<
        &mut Node,
        (With<RemoteDebugWorkspace>, Without<SceneEditorWorkspace>),
    >,
) {
    if !workspace.is_changed() {
        return;
    }
    for mut node in &mut scene_editors {
        node.display = if *workspace == ActiveWorkspace::SceneEditor {
            Display::Flex
        } else {
            Display::None
        };
    }
    for mut node in &mut remote_debugs {
        node.display = if *workspace == ActiveWorkspace::RemoteDebug {
            Display::Flex
        } else {
            Display::None
        };
    }
}

/// Update tab bg colors and dim remote tab when disconnected.
pub fn update_tab_highlights(
    workspace: Res<ActiveWorkspace>,
    manager: Res<ConnectionManager>,
    mut tabs: Query<(&WorkspaceTab, &mut BackgroundColor, &Children)>,
    mut texts: Query<&mut TextColor>,
) {
    if !workspace.is_changed() && !manager.is_changed() {
        return;
    }
    let connected = manager.is_connected();
    for (tab, mut bg, children) in &mut tabs {
        let is_active = tab.0 == *workspace;
        let is_disabled = tab.0 == ActiveWorkspace::RemoteDebug && !connected;

        bg.0 = if is_active {
            tokens::SELECTED_BG
        } else {
            tokens::TOOLBAR_BUTTON_BG
        };

        let text_color = if is_disabled {
            Color::srgba(0.4, 0.4, 0.4, 0.5)
        } else if is_active {
            tokens::TEXT_PRIMARY
        } else {
            tokens::TEXT_SECONDARY
        };

        for child in children.iter() {
            if let Ok(mut tc) = texts.get_mut(child) {
                tc.0 = text_color;
            }
        }
    }
}

fn bottom_panels(icon_font: Handle<Font>) -> impl Bundle {
    (
        EditorEntity,
        Node {
            width: percent(100),
            height: percent(100),
            ..Default::default()
        },
        // Horizontal split: asset browser | material browser
        split_panel::panel_group(
            0.15,
            (
                Spawn((
                    split_panel::panel(2),
                    asset_browser::asset_browser_panel(icon_font.clone()),
                )),
                Spawn(split_panel::panel_handle()),
                Spawn((
                    split_panel::panel(1),
                    material_browser::material_browser_panel(icon_font),
                )),
            ),
        ),
    )
}

/// Custom status bar that wraps the feathers status bar sections and adds
/// a connection indicator on the far right.
fn editor_status_bar() -> impl Bundle {
    (
        status_bar::StatusBar,
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::SpaceBetween,
            width: Val::Percent(100.0),
            height: Val::Px(tokens::STATUS_BAR_HEIGHT),
            padding: UiRect::horizontal(Val::Px(tokens::SPACING_MD)),
            flex_shrink: 0.0,
            ..Default::default()
        },
        BackgroundColor(tokens::STATUS_BAR_BG),
        children![
            (
                status_bar::StatusBarLeft,
                Text::new("Ready"),
                TextFont {
                    font_size: tokens::FONT_SM,
                    ..Default::default()
                },
                bevy::feathers::theme::ThemedText,
            ),
            (
                status_bar::StatusBarCenter,
                Text::new(""),
                TextFont {
                    font_size: tokens::FONT_SM,
                    ..Default::default()
                },
                TextColor(tokens::TEXT_SECONDARY),
            ),
            // Right side: gizmo info + connection indicator
            (
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(tokens::SPACING_LG),
                    ..Default::default()
                },
                children![
                    (
                        status_bar::StatusBarRight,
                        Text::new(""),
                        TextFont {
                            font_size: tokens::FONT_SM,
                            ..Default::default()
                        },
                        TextColor(tokens::TEXT_SECONDARY),
                    ),
                    // Connection indicator
                    crate::remote::panel::connection_indicator()
                ],
            )
        ],
    )
}

fn entity_inspector() -> impl Bundle {
    (
        Node {
            height: percent(100),
            flex_direction: FlexDirection::Column,
            overflow: Overflow::clip(),
            ..Default::default()
        },
        BackgroundColor(tokens::PANEL_BG),
        children![
            panel_header::panel_header("Inspector"),
            (
                Inspector,
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: px(tokens::SPACING_SM),
                    overflow: Overflow::scroll_y(),
                    flex_grow: 1.0,
                    min_height: px(0.0),
                    padding: UiRect::all(px(tokens::SPACING_SM)),
                    ..Default::default()
                }
            ),
        ],
    )
}
