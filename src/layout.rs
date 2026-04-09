use bevy::{feathers::theme::ThemedText, picking::hover::Hovered, prelude::*, ui_widgets::observe};
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
    Physics,
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
        // Outer shell: dark background with padding (Figma: 10px padding, bg #171717)
        BackgroundColor(tokens::WINDOW_BG),
        Node {
            width: percent(100),
            height: percent(100),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            padding: UiRect::all(px(tokens::PANEL_GAP)),
            ..Default::default()
        },
        children![(
            // Inner container: the editor workspace with rounded corners and border.
            EditorEntity,
            Node {
                width: percent(100),
                flex_grow: 1.0,
                min_height: px(0.0),
                flex_direction: FlexDirection::Column,
                border: UiRect::all(px(1.0)),
                border_radius: BorderRadius::all(px(8.0)),
                overflow: Overflow::clip(),
                ..Default::default()
            },
            BackgroundColor(tokens::WINDOW_BG),
            BorderColor::all(tokens::BORDER_SUBTLE),
            children![
                // Integrated window header: menu bar + scene tabs + controls
                window_header(),
                // Content container (flex grow). Holds both workspaces.
                // Figma: Editor (Rows) has padding: 0px 4px
                (
                    EditorEntity,
                    Node {
                        width: percent(100),
                        flex_grow: 1.0,
                        min_height: px(0.0),
                        flex_direction: FlexDirection::Column,
                        padding: UiRect::horizontal(px(tokens::PANEL_GAP)),
                        row_gap: px(tokens::PANEL_GAP),
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
                            display: Display::Flex,
                            ..Default::default()
                        },
                        children![(
                            // Three-column layout: left (hierarchy) | center (viewport+bottom) | right (inspector)
                            // Must have its own Node with Row direction for horizontal split
                            Node {
                                width: percent(100),
                                height: percent(100),
                                flex_direction: FlexDirection::Row,
                                ..Default::default()
                            },
                            split_panel::panel_group(
                                0.1,
                                (
                                    // Left column: hierarchy + project files (~266px default, ratio 1)
                                    Spawn((split_panel::panel(1), left_column(font.clone()))),
                                    Spawn(split_panel::panel_handle()),
                                    // Center column: viewport + bottom panels (ratio 4)
                                    Spawn((
                                        split_panel::panel(4),
                                        center_column(font.clone()),
                                    )),
                                    Spawn(split_panel::panel_handle()),
                                    // Right column: inspector (~310px default, ratio 1)
                                    Spawn((split_panel::panel(1), entity_inspector(font.clone()))),
                                ),
                            ),
                        )],
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
        )],
    )
}

/// Integrated window header: \[menu bar items\] \[scene tabs\] \[controls\]
fn window_header() -> impl Bundle {
    (
        EditorEntity,
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            width: percent(100),
            height: px(34.0),
            flex_shrink: 0.0,
            border_radius: BorderRadius::top(Val::Px(7.0)),
            ..Default::default()
        },
        BackgroundColor(tokens::WINDOW_BG),
        children![
            // Left: App name + menu bar
            (
                // This node hosts the MenuBar + MenuBarRoot so populate_menu_bar still works
                menu_bar::menu_bar_shell(),
            ),
            // Center: workspace / scene tabs
            (
                WorkspaceTabBar,
                EditorEntity,
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    flex_grow: 1.0,
                    height: percent(100),
                    column_gap: px(tokens::SPACING_SM),
                    ..Default::default()
                },
                children![
                    scene_tab("Main scene", ActiveWorkspace::SceneEditor, true),
                    scene_tab("Remote", ActiveWorkspace::RemoteDebug, false),
                ],
            ),
            // Right: placeholder for play/pause controls
            (
                EditorEntity,
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    padding: UiRect::horizontal(px(tokens::SPACING_MD)),
                    column_gap: px(tokens::SPACING_SM),
                    ..Default::default()
                },
            ),
        ],
    )
}

/// A scene tab in the header with colored accent bar.
fn scene_tab(label: &str, workspace: ActiveWorkspace, active: bool) -> impl Bundle {
    let accent_color = if active {
        tokens::ACCENT_BLUE
    } else {
        Color::NONE
    };
    let bg = if active {
        Color::srgba(1.0, 1.0, 1.0, 0.08)
    } else {
        Color::NONE
    };
    let text_color = if active {
        tokens::TEXT_PRIMARY
    } else {
        tokens::TEXT_SECONDARY
    };
    (
        WorkspaceTab(workspace),
        Interaction::default(),
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            padding: UiRect::axes(px(7.0), px(4.0)),
            column_gap: px(5.0),
            border_radius: BorderRadius::all(px(tokens::BORDER_RADIUS_MD)),
            ..Default::default()
        },
        BackgroundColor(bg),
        children![
            // Colored accent bar (2.5x12px)
            (
                Node {
                    width: px(2.5),
                    height: px(12.0),
                    border_radius: BorderRadius::all(px(5.0)),
                    ..Default::default()
                },
                BackgroundColor(accent_color),
            ),
            // Icon
            (
                Text::new(String::from(Icon::File.unicode())),
                TextFont {
                    font_size: 12.0,
                    ..Default::default()
                },
                TextColor(text_color),
            ),
            // Label
            (
                Text::new(label.to_string()),
                TextFont {
                    font_size: tokens::TEXT_SIZE_LG,
                    ..Default::default()
                },
                TextColor(text_color),
            ),
        ],
        observe(
            move |_: On<Pointer<Click>>,
                  mut workspace_res: ResMut<ActiveWorkspace>,
                  manager: Res<ConnectionManager>| {
                if workspace == ActiveWorkspace::RemoteDebug && !manager.is_connected() {
                    return;
                }
                *workspace_res = workspace;
            },
        ),
    )
}

/// Left column: Scene Tree panel (top) + Project Files panel (bottom), resizable split
fn left_column(icon_font: Handle<Font>) -> impl Bundle {
    (
        EditorEntity,
        Node {
            width: percent(100),
            height: percent(100),
            flex_direction: FlexDirection::Column,
            ..Default::default()
        },
        // Vertical split: hierarchy (top, ratio 3) | project files (bottom, ratio 1)
        split_panel::panel_group(
            0.15,
            (
                Spawn((split_panel::panel(3), entity_heiarchy(icon_font))),
                Spawn(split_panel::panel_handle()),
                Spawn((split_panel::panel(1), project_files_panel())),
            ),
        ),
    )
}

/// Project Files panel. File tree browser.
fn project_files_panel() -> impl Bundle {
    (
        EditorEntity,
        Node {
            flex_direction: FlexDirection::Column,
            width: percent(100),
            height: percent(100),
            overflow: Overflow::clip(),
            border_radius: BorderRadius::all(px(tokens::BORDER_RADIUS_LG)),
            ..Default::default()
        },
        BackgroundColor(tokens::PANEL_BG),
        children![
            panel_header::panel_tab_bar(&[panel_header::TabDef::new("Project Files", true)], true,),
            // Search input
            (
                Node {
                    flex_direction: FlexDirection::Column,
                    width: percent(100),
                    padding: UiRect::all(px(tokens::SPACING_SM)),
                    flex_shrink: 0.0,
                    ..Default::default()
                },
                children![(text_edit::text_edit(
                    TextEditProps::default()
                        .with_placeholder("Search...")
                        .allow_empty()
                ),)],
            ),
            // File tree content, populated by ProjectFilesPlugin.
            (
                crate::project_files::ProjectFilesTree,
                EditorEntity,
                Node {
                    flex_direction: FlexDirection::Column,
                    width: percent(100),
                    flex_grow: 1.0,
                    min_height: px(0.0),
                    overflow: Overflow::scroll_y(),
                    padding: UiRect::all(px(tokens::SPACING_SM)),
                    ..Default::default()
                },
            ),
        ],
    )
}

fn center_column(icon_font: Handle<Font>) -> impl Bundle {
    (
        EditorEntity,
        Node {
            width: percent(100),
            height: percent(100),
            flex_direction: FlexDirection::Column,
            ..Default::default()
        },
        // Vertical split: viewport (top) | bottom panels (bottom)
        split_panel::panel_group(
            0.15,
            (
                Spawn((
                    split_panel::panel(4),
                    viewport_with_toolbar(icon_font.clone()),
                )),
                Spawn(split_panel::panel_handle()),
                Spawn((split_panel::panel(1), bottom_panels(icon_font))),
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
            border_radius: BorderRadius::all(px(tokens::BORDER_RADIUS_LG)),
            ..Default::default()
        },
        BackgroundColor(tokens::PANEL_BG),
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
        BackgroundColor(tokens::PANEL_HEADER_BG),
        children![
            // Gizmo mode buttons
            toolbar_button(
                Icon::Move3d,
                "",
                GizmoMode::Translate,
                icon_font.clone(),
                "Move (Esc)"
            ),
            toolbar_button(
                Icon::Rotate3d,
                "R",
                GizmoMode::Rotate,
                icon_font.clone(),
                "Rotate (R)"
            ),
            toolbar_button(
                Icon::Scale3d,
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
                Icon::MousePointer2,
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
            // Separator
            separator::separator(separator::SeparatorProps::vertical()),
            toolbar_edit_button(
                Icon::Zap,
                EditToolButton::Physics,
                f.clone(),
                "Physics Tool"
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
                    font_size: 15.0,
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
                font_size: 15.0,
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
                    EditToolButton::Physics => {
                        draw_state.active = None;
                        brush_selection.entity = None;
                        brush_selection.faces.clear();
                        brush_selection.vertices.clear();
                        brush_selection.edges.clear();
                        if *edit_mode == EditMode::Physics {
                            // Toggle off
                            *edit_mode = EditMode::Object;
                        } else {
                            *edit_mode = EditMode::Physics;
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
    let add_entity_icon_font = icon_font.clone();
    (
        HierarchyPanel,
        Node {
            height: percent(100),
            flex_direction: FlexDirection::Column,
            overflow: Overflow::clip(),
            border_radius: BorderRadius::all(px(tokens::BORDER_RADIUS_LG)),
            ..Default::default()
        },
        BackgroundColor(tokens::PANEL_BG),
        children![
            panel_header::panel_tab_bar(
                &[
                    panel_header::TabDef::new("Scene Tree", true),
                    panel_header::TabDef::new("Import", false),
                ],
                true,
            ),
            // Tab 0: Scene Tree content
            (
                panel_header::PanelTabContent(0),
                Node {
                    flex_direction: FlexDirection::Column,
                    flex_grow: 1.0,
                    min_height: px(0.0),
                    padding: UiRect::all(px(tokens::SPACING_SM)),
                    display: Display::Flex,
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
                                            .with_placeholder("Filter...")
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
                    // Add Entity button (matching Figma reference)
                    (
                        Interaction::default(),
                        Hovered::default(),
                        Node {
                            flex_direction: FlexDirection::Row,
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            width: percent(100),
                            height: px(tokens::ROW_HEIGHT),
                            column_gap: px(tokens::SPACING_SM),
                            border_radius: BorderRadius::all(px(tokens::BORDER_RADIUS_MD)),
                            margin: UiRect::vertical(px(tokens::SPACING_XS)),
                            flex_shrink: 0.0,
                            ..Default::default()
                        },
                        BackgroundColor(tokens::ELEVATED_BG),
                        observe(
                            |hover: On<Pointer<Over>>, mut bg: Query<&mut BackgroundColor>| {
                                if let Ok(mut bg) = bg.get_mut(hover.event_target()) {
                                    bg.0 = tokens::TOOLBAR_ACTIVE_BG;
                                }
                            },
                        ),
                        observe(
                            |out: On<Pointer<Out>>, mut bg: Query<&mut BackgroundColor>| {
                                if let Ok(mut bg) = bg.get_mut(out.event_target()) {
                                    bg.0 = tokens::ELEVATED_BG;
                                }
                            },
                        ),
                        children![
                            (
                                Text::new(String::from(Icon::PackagePlus.unicode())),
                                TextFont {
                                    font: add_entity_icon_font,
                                    font_size: tokens::ICON_SM,
                                    ..Default::default()
                                },
                                TextColor(tokens::TEXT_PRIMARY),
                            ),
                            (
                                Text::new("Add Entity"),
                                TextFont {
                                    font_size: tokens::TEXT_SIZE,
                                    weight: FontWeight::MEDIUM,
                                    ..Default::default()
                                },
                                TextColor(tokens::TEXT_PRIMARY),
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
                    ),
                    // Scene stats footer (center-justified)
                    (
                        crate::status_bar::SceneStatsText,
                        Text::new(""),
                        TextFont {
                            font_size: tokens::FONT_SM,
                            ..Default::default()
                        },
                        TextColor(tokens::TEXT_SECONDARY),
                        TextLayout::new_with_justify(Justify::Center),
                        Node {
                            padding: UiRect::all(px(tokens::SPACING_XS)),
                            flex_shrink: 0.0,
                            width: percent(100),
                            ..Default::default()
                        },
                    )
                ],
            ),
            // Tab 1: Import placeholder
            (
                panel_header::PanelTabContent(1),
                Node {
                    flex_direction: FlexDirection::Column,
                    flex_grow: 1.0,
                    min_height: px(0.0),
                    padding: UiRect::all(px(tokens::SPACING_MD)),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    display: Display::None,
                    ..Default::default()
                },
                children![(
                    Text::new("Import"),
                    TextFont {
                        font_size: tokens::FONT_MD,
                        ..Default::default()
                    },
                    TextColor(tokens::TEXT_SECONDARY),
                )],
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
            tokens::TOOLBAR_ACTIVE_BG
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
pub(crate) fn update_edit_tool_highlights(
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
            EditToolButton::Physics => !draw_active && *edit_mode == EditMode::Physics,
        };
        bg.0 = if active {
            tokens::TOOLBAR_ACTIVE_BG
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

/// Update scene tab styling and dim remote tab when disconnected.
pub fn update_tab_highlights(
    workspace: Res<ActiveWorkspace>,
    manager: Res<ConnectionManager>,
    mut tabs: Query<(&WorkspaceTab, &mut BackgroundColor, &Children)>,
    mut texts: Query<&mut TextColor>,
    mut bgs: Query<&mut BackgroundColor, Without<WorkspaceTab>>,
) {
    if !workspace.is_changed() && !manager.is_changed() {
        return;
    }
    let connected = manager.is_connected();
    for (tab, mut tab_bg, children) in &mut tabs {
        let is_active = tab.0 == *workspace;
        let is_disabled = tab.0 == ActiveWorkspace::RemoteDebug && !connected;

        // Tab background
        tab_bg.0 = if is_active {
            Color::srgba(1.0, 1.0, 1.0, 0.08)
        } else {
            Color::NONE
        };

        let text_color = if is_disabled {
            Color::srgba(0.4, 0.4, 0.4, 0.5)
        } else if is_active {
            tokens::TEXT_PRIMARY
        } else {
            tokens::TEXT_SECONDARY
        };

        // Update text + accent bar colors on children
        let accent_color = if is_active {
            tokens::ACCENT_BLUE
        } else {
            Color::NONE
        };

        for child in children.iter() {
            if let Ok(mut tc) = texts.get_mut(child) {
                tc.0 = text_color;
            }
            // Update accent bar bg (first child is the 2.5px bar)
            if let Ok(mut child_bg) = bgs.get_mut(child) {
                child_bg.0 = accent_color;
            }
        }
    }
}

fn bottom_panels(icon_font: Handle<Font>) -> impl Bundle {
    asset_browser::asset_browser_panel(icon_font)
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
        BackgroundColor(tokens::WINDOW_BG),
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

fn entity_inspector(icon_font: Handle<Font>) -> impl Bundle {
    (
        Node {
            height: percent(100),
            flex_direction: FlexDirection::Column,
            overflow: Overflow::clip(),
            border_radius: BorderRadius::all(px(tokens::BORDER_RADIUS_LG)),
            ..Default::default()
        },
        BackgroundColor(tokens::PANEL_BG),
        children![
            panel_header::panel_tab_bar(
                &[
                    panel_header::TabDef::new("Components", true),
                    panel_header::TabDef::new("Materials", false),
                    panel_header::TabDef::new("Resources", false),
                    panel_header::TabDef::new("Systems", false),
                ],
                true,
            ),
            // Tab 0: Components
            (
                panel_header::PanelTabContent(0),
                Node {
                    flex_direction: FlexDirection::Column,
                    flex_grow: 1.0,
                    min_height: px(0.0),
                    display: Display::Flex,
                    ..Default::default()
                },
                children![
                    // Filter + Add Component (non-scrolling header area)
                    (
                        Node {
                            flex_direction: FlexDirection::Column,
                            width: percent(100),
                            padding: UiRect::all(px(tokens::SPACING_SM)),
                            row_gap: px(tokens::SPACING_XS),
                            flex_shrink: 0.0,
                            ..Default::default()
                        },
                        children![
                            // Filter input
                            (
                                crate::inspector::InspectorSearch,
                                text_edit::text_edit(
                                    TextEditProps::default()
                                        .with_placeholder("Filter...")
                                        .allow_empty()
                                ),
                            ),
                            // Add Component button, wired to component_picker observer.
                            (
                                crate::inspector::AddComponentButton,
                                Interaction::default(),
                                Node {
                                    flex_direction: FlexDirection::Row,
                                    justify_content: JustifyContent::Center,
                                    align_items: AlignItems::Center,
                                    width: percent(100),
                                    height: px(tokens::ROW_HEIGHT),
                                    column_gap: px(tokens::SPACING_SM),
                                    border_radius: BorderRadius::all(px(tokens::BORDER_RADIUS_MD)),
                                    flex_shrink: 0.0,
                                    ..Default::default()
                                },
                                BackgroundColor(tokens::ELEVATED_BG),
                                observe(
                                    |hover: On<Pointer<Over>>,
                                     mut bg: Query<&mut BackgroundColor>| {
                                        if let Ok(mut bg) = bg.get_mut(hover.event_target()) {
                                            bg.0 = tokens::TOOLBAR_ACTIVE_BG;
                                        }
                                    },
                                ),
                                observe(
                                    |out: On<Pointer<Out>>,
                                     mut bg: Query<&mut BackgroundColor>| {
                                        if let Ok(mut bg) = bg.get_mut(out.event_target()) {
                                            bg.0 = tokens::ELEVATED_BG;
                                        }
                                    },
                                ),
                                children![
                                    (
                                        Text::new(String::from(Icon::PackagePlus.unicode())),
                                        TextFont {
                                            font: icon_font.clone(),
                                            font_size: tokens::ICON_SM,
                                            ..Default::default()
                                        },
                                        TextColor(tokens::TEXT_PRIMARY),
                                    ),
                                    (
                                        Text::new("Add Component"),
                                        TextFont {
                                            font_size: tokens::TEXT_SIZE,
                                            weight: FontWeight::MEDIUM,
                                            ..Default::default()
                                        },
                                        TextColor(tokens::TEXT_PRIMARY),
                                    ),
                                ],
                                observe(
                                    |click: On<Pointer<Click>>, mut commands: Commands| {
                                        commands.trigger(
                                            jackdaw_feathers::button::ButtonClickEvent {
                                                entity: click.event_target(),
                                            },
                                        );
                                    },
                                ),
                            ),
                        ],
                    ),
                    // Scrollable component list
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
            ),
            // Tab 1: Materials browser (wrapped so PanelTabContent controls visibility)
            (
                panel_header::PanelTabContent(1),
                Node {
                    flex_direction: FlexDirection::Column,
                    flex_grow: 1.0,
                    min_height: px(0.0),
                    display: Display::None,
                    ..Default::default()
                },
                children![material_browser::material_browser_panel(icon_font)],
            ),
            // Tab 2: Resources placeholder
            (
                panel_header::PanelTabContent(2),
                Node {
                    flex_direction: FlexDirection::Column,
                    flex_grow: 1.0,
                    min_height: px(0.0),
                    padding: UiRect::all(px(tokens::SPACING_MD)),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    display: Display::None,
                    ..Default::default()
                },
                children![(
                    Text::new("Resources"),
                    TextFont {
                        font_size: tokens::FONT_MD,
                        ..Default::default()
                    },
                    TextColor(tokens::TEXT_SECONDARY),
                )],
            ),
            // Tab 3: Systems placeholder
            (
                panel_header::PanelTabContent(3),
                Node {
                    flex_direction: FlexDirection::Column,
                    flex_grow: 1.0,
                    min_height: px(0.0),
                    padding: UiRect::all(px(tokens::SPACING_MD)),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    display: Display::None,
                    ..Default::default()
                },
                children![(
                    Text::new("Systems"),
                    TextFont {
                        font_size: tokens::FONT_MD,
                        ..Default::default()
                    },
                    TextColor(tokens::TEXT_SECONDARY),
                )],
            ),
        ],
    )
}
