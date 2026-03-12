use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bevy::{
    feathers::theme::ThemedText,
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, futures_lite::future},
    ui_widgets::observe,
    window::{PrimaryWindow, RawHandleWrapper},
};
use jackdaw_feathers::{
    icons,
    text_edit::{self, TextEditProps, TextEditValue},
    tokens,
};
use rfd::AsyncFileDialog;

use crate::{
    EditorEntity,
    asset_browser::attach_tooltip,
    brush::{Brush, BrushEditMode, BrushSelection, EditMode, SetBrush},
    commands::CommandHistory,
    material_preview::MaterialPreviewState,
    selection::Selection,
};

pub struct MaterialBrowserPlugin;

impl Plugin for MaterialBrowserPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MaterialBrowserState>()
            .init_resource::<MaterialPreviewState>()
            .init_resource::<MaterialRegistry>()
            .add_systems(
                OnEnter(crate::AppState::Editor),
                (
                    scan_material_definitions,
                    crate::material_preview::setup_material_preview_scene,
                ),
            )
            .add_systems(
                Update,
                (
                    rescan_material_definitions,
                    apply_material_filter,
                    update_material_browser_ui,
                    update_preview_area,
                    poll_material_browser_folder,
                    crate::material_preview::update_preview_camera_transform,
                    crate::material_preview::update_active_preview_material,
                )
                    .run_if(in_state(crate::AppState::Editor)),
            )
            .add_observer(handle_apply_material)
            .add_observer(handle_select_material_preview);
    }
}

/// Simple registry of named materials for browsing.
#[derive(Resource, Default)]
pub struct MaterialRegistry {
    pub entries: Vec<MaterialRegistryEntry>,
}

pub struct MaterialRegistryEntry {
    pub name: String,
    pub handle: Handle<StandardMaterial>,
}

impl MaterialRegistry {
    pub fn get_by_name(&self, name: &str) -> Option<&MaterialRegistryEntry> {
        self.entries.iter().find(|e| e.name == name)
    }

    pub fn add(&mut self, name: String, handle: Handle<StandardMaterial>) {
        self.entries.push(MaterialRegistryEntry { name, handle });
    }
}

#[derive(Resource, Default)]
pub struct MaterialBrowserState {
    pub filter: String,
    pub needs_rescan: bool,
    pub scan_directory: PathBuf,
}

#[derive(Event, Clone)]
pub struct ApplyMaterialDefToFaces {
    pub material: Handle<StandardMaterial>,
}

#[derive(Event, Clone)]
struct SelectMaterialPreview {
    handle: Handle<StandardMaterial>,
}

#[derive(Component)]
pub struct MaterialBrowserPanel;

#[derive(Component)]
pub struct MaterialBrowserGrid;

#[derive(Component)]
pub struct MaterialBrowserFilter;

#[derive(Component)]
struct MaterialBrowserRootLabel;

#[derive(Resource)]
struct MaterialBrowserFolderTask(Task<Option<rfd::FileHandle>>);

/// Container for the interactive preview area (shown when a material is selected).
#[derive(Component)]
struct PreviewAreaContainer;

/// The ImageNode displaying the render-to-texture preview.
#[derive(Component)]
struct PreviewAreaImage;

/// Text label showing the selected material name in the preview area.
#[derive(Component)]
struct PreviewAreaLabel;

/// PBR filename regex pattern.
fn pbr_filename_regex() -> Option<regex::Regex> {
    let pattern = r"(?i)^(.+?)[_\-\.\s](diffuse|diff|albedo|base|col|color|basecolor|metallic|metalness|metal|mtl|roughness|rough|rgh|normal|normaldx|normalgl|nor|nrm|nrml|norm|orm|emission|emissive|emit|ao|ambient|occlusion|ambientocclusion|displacement|displace|disp|dsp|height|heightmap|alpha|opacity|specularity|specular|spec|spc|gloss|glossy|glossiness|bump|bmp|b|n)\.(png|jpg|jpeg|ktx2|bmp|tga|webp)$";
    regex::Regex::new(pattern).ok()
}

/// Returns `true` if the PNG file uses 16-bit (or higher) bit depth.
///
/// Bevy decodes such PNGs as `R16Uint` which is incompatible with
/// `StandardMaterial`'s float-filterable `depth_map` slot.
fn is_16bit_png(path: &Path) -> bool {
    if !path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("png"))
    {
        return false;
    }
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    use std::io::Read;
    // PNG layout: 8-byte signature, then IHDR chunk (4 len + 4 type + 13 data).
    // Byte 24 (offset 24) is the bit depth field inside IHDR.
    let mut header = [0u8; 25];
    if file.read_exact(&mut header).is_err() {
        return false;
    }
    header[24] >= 16
}

/// Scan a directory for PBR texture sets and create `StandardMaterial` assets.
fn detect_and_create_materials(
    dir: &Path,
    asset_server: &AssetServer,
    materials: &mut Assets<StandardMaterial>,
) -> Vec<(String, Handle<StandardMaterial>)> {
    let re = match pbr_filename_regex() {
        Some(r) => r,
        None => return Vec::new(),
    };

    let mut groups: HashMap<String, Vec<(String, String)>> = HashMap::new();
    scan_dir_recursive(dir, &re, &mut groups);

    let mut results = Vec::new();
    for (base_name, slots) in &groups {
        let mut base_color_texture = None;
        let mut normal_map_texture = None;
        let mut metallic_roughness_texture = None;
        let mut emissive_texture = None;
        let mut occlusion_texture = None;
        let mut depth_map = None;

        for (tag, asset_path) in slots {
            let tag_lower = tag.to_lowercase();
            match tag_lower.as_str() {
                "diffuse" | "diff" | "albedo" | "base" | "col" | "color" | "basecolor" | "b" => {
                    base_color_texture = Some(asset_server.load::<Image>(asset_path.clone()));
                }
                "normalgl" | "nor" | "nrm" | "nrml" | "norm" | "bump" | "bmp" | "n" | "normal" => {
                    normal_map_texture = Some(asset_server.load::<Image>(asset_path.clone()));
                }
                "orm" | "metallic" | "metalness" | "metal" | "mtl" | "roughness" | "rough" | "rgh" => {
                    if metallic_roughness_texture.is_none() {
                        metallic_roughness_texture = Some(asset_server.load::<Image>(asset_path.clone()));
                    }
                }
                "emission" | "emissive" | "emit" => {
                    emissive_texture = Some(asset_server.load::<Image>(asset_path.clone()));
                }
                "ao" | "ambient" | "occlusion" | "ambientocclusion" => {
                    occlusion_texture = Some(asset_server.load::<Image>(asset_path.clone()));
                }
                "displacement" | "displace" | "disp" | "dsp" | "height" | "heightmap" => {
                    // Skip 16-bit integer PNGs — Bevy decodes them as R16Uint which
                    // is incompatible with StandardMaterial's float-filterable depth_map slot.
                    if !is_16bit_png(Path::new(asset_path)) {
                        depth_map = Some(asset_server.load::<Image>(asset_path.clone()));
                    }
                }
                _ => {}
            }
        }

        // Only create if at least one texture slot is populated
        if base_color_texture.is_none()
            && normal_map_texture.is_none()
            && metallic_roughness_texture.is_none()
            && emissive_texture.is_none()
            && occlusion_texture.is_none()
            && depth_map.is_none()
        {
            continue;
        }

        let handle = materials.add(StandardMaterial {
            base_color_texture,
            normal_map_texture,
            metallic_roughness_texture,
            emissive_texture,
            occlusion_texture,
            depth_map,
            ..default()
        });

        results.push((base_name.clone(), handle));
    }

    results.sort_by(|a, b| a.0.cmp(&b.0));
    results
}

fn scan_dir_recursive(
    dir: &Path,
    re: &regex::Regex,
    groups: &mut HashMap<String, Vec<(String, String)>>,
) {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_dir_recursive(&path, re, groups);
        } else {
            let file_name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            // Skip non-2D KTX2 files (cubemaps, texture arrays) — they can't
            // be used as regular 2D textures in StandardMaterial.
            if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("ktx2"))
                && crate::asset_browser::is_ktx2_non_2d(&path)
            {
                continue;
            }

            if let Some(caps) = re.captures(&file_name) {
                let base_name = caps[1].to_string();
                let tag = caps[2].to_string();

                let asset_path = path.to_string_lossy().replace('\\', "/");

                groups
                    .entry(base_name.to_lowercase())
                    .or_default()
                    .push((tag, asset_path));
            }
        }
    }
}

fn scan_material_definitions(
    mut state: ResMut<MaterialBrowserState>,
    mut registry: ResMut<MaterialRegistry>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
    project_root: Option<Res<crate::project::ProjectRoot>>,
) {
    let assets_dir = project_root
        .map(|p| p.assets_dir())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default().join("assets"));
    state.scan_directory = assets_dir.clone();

    let detected = detect_and_create_materials(&state.scan_directory, &asset_server, &mut materials);
    for (name, handle) in detected {
        if registry.get_by_name(&name).is_none() {
            registry.add(name, handle);
        }
    }
}

fn rescan_material_definitions(
    mut state: ResMut<MaterialBrowserState>,
    mut registry: ResMut<MaterialRegistry>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
    project_root: Option<Res<crate::project::ProjectRoot>>,
) {
    if !state.needs_rescan {
        return;
    }
    state.needs_rescan = false;

    state.scan_directory = project_root
        .map(|p| p.assets_dir())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default().join("assets"));

    registry.entries.clear();

    let detected = detect_and_create_materials(&state.scan_directory, &asset_server, &mut materials);
    for (name, handle) in detected {
        registry.add(name, handle);
    }
}

fn apply_material_filter(
    filter_input: Query<&TextEditValue, (With<MaterialBrowserFilter>, Changed<TextEditValue>)>,
    mut state: ResMut<MaterialBrowserState>,
) {
    for input in &filter_input {
        if state.filter != input.0 {
            state.filter = input.0.clone();
        }
    }
}

fn handle_apply_material(
    event: On<ApplyMaterialDefToFaces>,
    brush_selection: Res<BrushSelection>,
    edit_mode: Res<EditMode>,
    selection: Res<Selection>,
    mut brushes: Query<&mut Brush>,
    mut history: ResMut<CommandHistory>,
) {
    if *edit_mode == EditMode::BrushEdit(BrushEditMode::Face) && !brush_selection.faces.is_empty() {
        if let Some(entity) = brush_selection.entity {
            if let Ok(mut brush) = brushes.get_mut(entity) {
                let old = brush.clone();
                for &face_idx in &brush_selection.faces {
                    if face_idx < brush.faces.len() {
                        brush.faces[face_idx].material = event.material.clone();
                    }
                }
                let cmd = SetBrush {
                    entity,
                    old,
                    new: brush.clone(),
                    label: "Apply material".into(),
                };
                history.undo_stack.push(Box::new(cmd));
                history.redo_stack.clear();
            }
        }
    } else {
        for &entity in &selection.entities {
            if let Ok(mut brush) = brushes.get_mut(entity) {
                let old = brush.clone();
                for face in brush.faces.iter_mut() {
                    face.material = event.material.clone();
                }
                let cmd = SetBrush {
                    entity,
                    old,
                    new: brush.clone(),
                    label: "Apply material".into(),
                };
                history.undo_stack.push(Box::new(cmd));
                history.redo_stack.clear();
            }
        }
    }
}

fn handle_select_material_preview(
    event: On<SelectMaterialPreview>,
    mut preview_state: ResMut<MaterialPreviewState>,
) {
    if preview_state.active_material.as_ref() == Some(&event.handle) {
        preview_state.active_material = None;
    } else {
        preview_state.active_material = Some(event.handle.clone());
        preview_state.orbit_yaw = 0.5;
        preview_state.orbit_pitch = -0.3;
        preview_state.zoom_distance = 3.0;
    }
}

/// Update the interactive preview area visibility and content.
fn update_preview_area(
    mut commands: Commands,
    preview_state: Res<MaterialPreviewState>,
    registry: Res<MaterialRegistry>,
    container_query: Query<(Entity, Option<&Children>), With<PreviewAreaContainer>>,
) {
    if !preview_state.is_changed() {
        return;
    }

    let Ok((container, children)) = container_query.single() else {
        return;
    };

    // Clear existing children
    if let Some(children) = children {
        for child in children.iter() {
            commands.entity(child).despawn();
        }
    }

    let Some(ref active_handle) = preview_state.active_material else {
        return;
    };

    // Show the preview image
    let preview_img = preview_state.preview_image.clone();
    commands.spawn((
        PreviewAreaImage,
        ImageNode::new(preview_img),
        Node {
            width: Val::Px(128.0),
            height: Val::Px(128.0),
            align_self: AlignSelf::Center,
            ..Default::default()
        },
        ChildOf(container),
    ));

    // Material name
    let active_name = registry
        .entries
        .iter()
        .find(|e| e.handle == *active_handle)
        .map(|e| e.name.clone())
        .unwrap_or_else(|| format!("{:?}", active_handle.id()));
    commands.spawn((
        PreviewAreaLabel,
        Text::new(active_name),
        TextFont {
            font_size: tokens::FONT_SM,
            ..Default::default()
        },
        TextColor(tokens::TEXT_PRIMARY),
        Node {
            align_self: AlignSelf::Center,
            margin: UiRect::vertical(Val::Px(tokens::SPACING_XS)),
            ..Default::default()
        },
        ChildOf(container),
    ));

    // Apply button
    let handle_for_apply = active_handle.clone();
    let apply_btn = commands
        .spawn((
            Node {
                padding: UiRect::axes(Val::Px(tokens::SPACING_MD), Val::Px(tokens::SPACING_XS)),
                border_radius: BorderRadius::all(Val::Px(tokens::BORDER_RADIUS_SM)),
                align_self: AlignSelf::Center,
                margin: UiRect::top(Val::Px(tokens::SPACING_XS)),
                ..Default::default()
            },
            BackgroundColor(tokens::INPUT_BG),
            ChildOf(container),
        ))
        .id();
    commands.spawn((
        Text::new("Apply"),
        TextFont {
            font_size: tokens::FONT_SM,
            ..Default::default()
        },
        TextColor(tokens::TEXT_PRIMARY),
        ChildOf(apply_btn),
    ));
    commands
        .entity(apply_btn)
        .observe(move |_: On<Pointer<Click>>, mut commands: Commands| {
            commands.trigger(ApplyMaterialDefToFaces {
                material: handle_for_apply.clone(),
            });
        });
    commands.entity(apply_btn).observe(
        |hover: On<Pointer<Over>>, mut bg: Query<&mut BackgroundColor>| {
            if let Ok(mut bg) = bg.get_mut(hover.event_target()) {
                bg.0 = tokens::HOVER_BG;
            }
        },
    );
    commands.entity(apply_btn).observe(
        |out: On<Pointer<Out>>, mut bg: Query<&mut BackgroundColor>| {
            if let Ok(mut bg) = bg.get_mut(out.event_target()) {
                bg.0 = tokens::INPUT_BG;
            }
        },
    );
}

fn spawn_material_folder_dialog(
    _: On<Pointer<Click>>,
    mut commands: Commands,
    raw_handle: Query<&RawHandleWrapper, With<PrimaryWindow>>,
) {
    let mut dialog = AsyncFileDialog::new().set_title("Select materials directory");
    if let Ok(rh) = raw_handle.single() {
        let handle = unsafe { rh.get_handle() };
        dialog = dialog.set_parent(&handle);
    }
    let task = AsyncComputeTaskPool::get().spawn(async move { dialog.pick_folder().await });
    commands.insert_resource(MaterialBrowserFolderTask(task));
}

fn poll_material_browser_folder(world: &mut World) {
    let Some(mut task_res) = world.get_resource_mut::<MaterialBrowserFolderTask>() else {
        return;
    };
    let Some(result) = future::block_on(future::poll_once(&mut task_res.0)) else {
        return;
    };
    world.remove_resource::<MaterialBrowserFolderTask>();

    if let Some(handle) = result {
        let path = handle.path().to_path_buf();
        let mut state = world.resource_mut::<MaterialBrowserState>();
        state.scan_directory = path.clone();
        state.needs_rescan = true;

        let mut label_query = world.query_filtered::<&mut Text, With<MaterialBrowserRootLabel>>();
        for mut text in label_query.iter_mut(world) {
            **text = path.to_string_lossy().to_string();
        }
    }
}

fn update_material_browser_ui(
    mut commands: Commands,
    registry: Res<MaterialRegistry>,
    state: Res<MaterialBrowserState>,
    materials: Res<Assets<StandardMaterial>>,
    grid_query: Query<(Entity, Option<&Children>), With<MaterialBrowserGrid>>,
    mut root_label_query: Query<&mut Text, With<MaterialBrowserRootLabel>>,
) {
    let needs_rebuild = registry.is_changed() || state.is_changed();
    if !needs_rebuild {
        return;
    }

    for mut text in root_label_query.iter_mut() {
        **text = state.scan_directory.to_string_lossy().to_string();
    }

    let Ok((grid_entity, grid_children)) = grid_query.single() else {
        return;
    };

    if let Some(children) = grid_children {
        for child in children.iter() {
            commands.entity(child).despawn();
        }
    }

    let filter_lower = state.filter.to_lowercase();

    for entry in &registry.entries {
        if !filter_lower.is_empty() && !entry.name.to_lowercase().contains(&filter_lower) {
            continue;
        }

        let name = entry.name.clone();
        let handle = entry.handle.clone();

        let thumb_entity = commands
            .spawn((
                Node {
                    width: Val::Px(64.0),
                    height: Val::Px(80.0),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    padding: UiRect::all(Val::Px(2.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(4.0)),
                    ..Default::default()
                },
                BorderColor::all(Color::NONE),
                BackgroundColor(Color::NONE),
                ChildOf(grid_entity),
            ))
            .id();

        // Use base_color_texture as thumbnail if available
        let thumbnail = materials
            .get(&handle)
            .and_then(|m| m.base_color_texture.clone());

        if let Some(img) = thumbnail {
            commands.spawn((
                ImageNode::new(img),
                Node {
                    width: Val::Px(56.0),
                    height: Val::Px(56.0),
                    ..Default::default()
                },
                ChildOf(thumb_entity),
            ));
        } else {
            commands.spawn((
                Node {
                    width: Val::Px(56.0),
                    height: Val::Px(56.0),
                    ..Default::default()
                },
                BackgroundColor(Color::srgb(0.3, 0.3, 0.3)),
                ChildOf(thumb_entity),
            ));
        }

        let is_truncated = name.len() > 10;
        let display_name = if is_truncated {
            format!("{}...", &name[..8])
        } else {
            name.clone()
        };
        let name_entity = commands
            .spawn((
                Text::new(display_name),
                TextFont {
                    font_size: 9.0,
                    ..Default::default()
                },
                TextColor(tokens::TEXT_SECONDARY),
                Node {
                    max_width: Val::Px(60.0),
                    overflow: Overflow::clip(),
                    ..Default::default()
                },
                ChildOf(thumb_entity),
            ))
            .id();
        if is_truncated {
            attach_tooltip(&mut commands, name_entity, name.clone());
        }

        // Hover
        commands.entity(thumb_entity).observe(
            |hover: On<Pointer<Over>>, mut borders: Query<&mut BorderColor>| {
                if let Ok(mut border) = borders.get_mut(hover.event_target()) {
                    *border = BorderColor::all(tokens::SELECTED_BORDER);
                }
            },
        );
        commands.entity(thumb_entity).observe(
            |out: On<Pointer<Out>>, mut borders: Query<&mut BorderColor>| {
                if let Ok(mut border) = borders.get_mut(out.event_target()) {
                    *border = BorderColor::all(Color::NONE);
                }
            },
        );

        // Single-click: select for preview
        let handle_for_select = handle.clone();
        commands.entity(thumb_entity).observe(
            move |click: On<Pointer<Click>>, mut commands: Commands| {
                if click.event().button == PointerButton::Primary {
                    commands.trigger(SelectMaterialPreview {
                        handle: handle_for_select.clone(),
                    });
                }
            },
        );
    }
}

pub fn material_browser_panel(icon_font: Handle<Font>) -> impl Bundle {
    (
        MaterialBrowserPanel,
        EditorEntity,
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            ..Default::default()
        },
        BackgroundColor(tokens::PANEL_BG),
        children![
            // Header
            (
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::SpaceBetween,
                    width: Val::Percent(100.0),
                    height: Val::Px(tokens::ROW_HEIGHT),
                    padding: UiRect::horizontal(Val::Px(tokens::SPACING_MD)),
                    flex_shrink: 0.0,
                    ..Default::default()
                },
                BackgroundColor(tokens::PANEL_HEADER_BG),
                children![
                    // Left side: title + path
                    (
                        Node {
                            flex_direction: FlexDirection::Row,
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(tokens::SPACING_MD),
                            overflow: Overflow::clip(),
                            flex_shrink: 1.0,
                            ..Default::default()
                        },
                        children![
                            (
                                Text::new("Materials"),
                                TextFont {
                                    font_size: tokens::FONT_MD,
                                    ..Default::default()
                                },
                                ThemedText,
                            ),
                            (
                                MaterialBrowserRootLabel,
                                Text::new(""),
                                TextFont {
                                    font_size: tokens::FONT_SM,
                                    ..Default::default()
                                },
                                TextColor(tokens::TEXT_SECONDARY),
                            ),
                        ],
                    ),
                    // Right side: folder picker + rescan
                    (
                        Node {
                            flex_direction: FlexDirection::Row,
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(tokens::SPACING_XS),
                            ..Default::default()
                        },
                        children![
                            material_folder_button(icon_font.clone()),
                            rescan_button(icon_font),
                        ],
                    ),
                ],
            ),
            // Interactive preview area (content populated dynamically)
            (
                PreviewAreaContainer,
                EditorEntity,
                Node {
                    flex_direction: FlexDirection::Column,
                    width: Val::Percent(100.0),
                    padding: UiRect::all(Val::Px(tokens::SPACING_SM)),
                    flex_shrink: 0.0,
                    ..Default::default()
                },
            ),
            // Filter input
            (
                Node {
                    padding: UiRect::axes(Val::Px(tokens::SPACING_SM), Val::Px(tokens::SPACING_XS),),
                    flex_shrink: 0.0,
                    ..Default::default()
                },
                children![(
                    MaterialBrowserFilter,
                    text_edit::text_edit(
                        TextEditProps::default()
                            .with_placeholder("Filter materials")
                            .allow_empty()
                    )
                ),],
            ),
            // Grid
            (
                MaterialBrowserGrid,
                EditorEntity,
                Node {
                    flex_direction: FlexDirection::Row,
                    flex_wrap: FlexWrap::Wrap,
                    align_content: AlignContent::FlexStart,
                    width: Val::Percent(100.0),
                    flex_grow: 1.0,
                    min_height: Val::Px(0.0),
                    overflow: Overflow::scroll_y(),
                    padding: UiRect::all(Val::Px(tokens::SPACING_SM)),
                    row_gap: Val::Px(tokens::SPACING_XS),
                    column_gap: Val::Px(tokens::SPACING_XS),
                    ..Default::default()
                },
            ),
        ],
    )
}

fn material_folder_button(icon_font: Handle<Font>) -> impl Bundle {
    (
        Node {
            padding: UiRect::all(Val::Px(tokens::SPACING_XS)),
            border_radius: BorderRadius::all(Val::Px(tokens::BORDER_RADIUS_SM)),
            ..Default::default()
        },
        icons::icon_colored(
            icons::Icon::FolderOpen,
            tokens::FONT_MD,
            icon_font,
            tokens::TEXT_SECONDARY,
        ),
        observe(spawn_material_folder_dialog),
    )
}

fn rescan_button(icon_font: Handle<Font>) -> impl Bundle {
    (
        Node {
            padding: UiRect::all(Val::Px(tokens::SPACING_XS)),
            border_radius: BorderRadius::all(Val::Px(tokens::BORDER_RADIUS_SM)),
            ..Default::default()
        },
        icons::icon_colored(
            icons::Icon::RefreshCw,
            tokens::FONT_MD,
            icon_font,
            tokens::TEXT_SECONDARY,
        ),
        observe(
            |_: On<Pointer<Click>>, mut state: ResMut<MaterialBrowserState>| {
                state.needs_rescan = true;
            },
        ),
    )
}
