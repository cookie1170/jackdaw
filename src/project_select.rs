use std::path::PathBuf;

use bevy::{
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, futures_lite::future},
    window::{PrimaryWindow, RawHandleWrapper},
};
use jackdaw_feathers::{
    button::{ButtonVariant, IconButtonProps, icon_button},
    icons::{EditorFont, Icon},
    text_edit::{TextEditProps, TextEditValue, text_edit},
    tokens,
};
use rfd::{AsyncFileDialog, FileHandle};

use crate::{
    AppState,
    new_project::{ScaffoldError, TemplatePreset, scaffold_project},
    project::{self, ProjectRoot},
};

pub struct ProjectSelectPlugin;

impl Plugin for ProjectSelectPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NewProjectState>()
            .add_systems(OnEnter(AppState::ProjectSelect), spawn_project_selector)
            .add_systems(
                Update,
                (poll_folder_dialog, poll_new_project_tasks)
                    .run_if(in_state(AppState::ProjectSelect)),
            );
    }
}

/// Marker for the project selector root UI node.
#[derive(Component)]
struct ProjectSelectorRoot;

/// When set, the project selector will skip UI and auto-open the given project.
#[derive(Resource)]
pub struct PendingAutoOpen {
    pub path: PathBuf,
    /// `true` when we got here via a post-restart auto-open —
    /// the parent process already built + installed the dylib,
    /// so we skip that step (preventing an infinite
    /// build→restart→auto-open→build loop).
    pub skip_build: bool,
}

/// Resource holding the async folder picker task.
#[derive(Resource)]
struct FolderDialogTask(Task<Option<rfd::FileHandle>>);

/// Root marker for the New Project modal overlay. Spawned when the
/// user clicks **+ New Extension** / **+ New Game**; despawned on
/// Cancel or on successful scaffold.
#[derive(Component)]
struct NewProjectModalRoot;

/// Wraps the Name `TextEdit` so the Create handler can read its
/// current value.
#[derive(Component)]
struct NewProjectNameInput;

/// Wraps the Template URL `TextEdit`. Pre-filled with the default
/// URL for the active preset; always editable so users can paste
/// any Bevy-CLI-compatible URL.
#[derive(Component)]
struct NewProjectTemplateInput;

#[derive(Component)]
struct NewProjectLocationText;

#[derive(Component)]
struct NewProjectStatusText;

#[derive(Component)]
struct NewProjectCancelButton;

#[derive(Component)]
struct NewProjectCreateButton;

#[derive(Component)]
struct NewProjectBrowseButton;

/// Drives the modal's async operations.
#[derive(Resource, Default)]
struct NewProjectState {
    /// Which preset the user opened the dialog with. `None` when
    /// the modal isn't open.
    preset: Option<TemplatePreset>,
    /// Parent directory the new project will be placed under.
    /// Scaffolder produces `location/name/`.
    location: PathBuf,
    /// In-flight folder picker (rfd).
    folder_task: Option<Task<Option<FileHandle>>>,
    /// In-flight scaffold (bevy-cli subprocess).
    scaffold_task: Option<Task<Result<PathBuf, ScaffoldError>>>,
    /// In-flight initial build after scaffold. Queued immediately
    /// after the scaffold task succeeds so the user lands in the
    /// editor with the game/extension dylib already installed.
    build_task: Option<Task<Result<PathBuf, crate::ext_build::BuildError>>>,
    /// Path to the freshly-scaffolded project, kept around so the
    /// build-completion handler can transition into the editor
    /// pointing at the right root.
    pending_project: Option<PathBuf>,
    /// Last user-visible message (used for both progress and errors).
    status: Option<String>,
}

fn default_projects_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join("Projects"))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn spawn_project_selector(
    mut commands: Commands,
    editor_font: Res<EditorFont>,
    icon_font: Res<jackdaw_feathers::icons::IconFont>,
    pending: Option<Res<PendingAutoOpen>>,
) {
    if let Some(pending) = pending {
        let path = pending.path.clone();
        let skip_build = pending.skip_build;
        commands.remove_resource::<PendingAutoOpen>();
        commands.queue(move |world: &mut World| {
            enter_project_with(world, path, skip_build);
        });
        return;
    }

    let recent = project::read_recent_projects();
    let font = editor_font.0.clone();
    let icon_font_handle = icon_font.0.clone();

    // Detect CWD project candidate
    let cwd = std::env::current_dir().unwrap_or_default();
    let cwd_has_project = cwd.join(".jsn/project.jsn").is_file()
        || cwd.join("project.jsn").is_file()
        || cwd.join("assets").is_dir();

    // UI camera for the project selector screen
    commands.spawn((ProjectSelectorRoot, Camera2d));

    commands
        .spawn((
            ProjectSelectorRoot,
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..Default::default()
            },
            BackgroundColor(tokens::WINDOW_BG),
        ))
        .with_children(|parent| {
            // Card container
            parent
                .spawn(Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    padding: UiRect::all(Val::Px(32.0)),
                    row_gap: Val::Px(24.0),
                    min_width: Val::Px(420.0),
                    max_width: Val::Px(520.0),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(8.0)),
                    ..Default::default()
                })
                .insert(BackgroundColor(tokens::PANEL_BG))
                .insert(BorderColor::all(tokens::BORDER_SUBTLE))
                .with_children(|card| {
                    // Title
                    card.spawn((
                        Text::new("jackdaw"),
                        TextFont {
                            font: font.clone(),
                            font_size: 28.0,
                            ..Default::default()
                        },
                        TextColor(tokens::TEXT_PRIMARY),
                    ));

                    // Subtitle
                    card.spawn((
                        Text::new("Select a project to open"),
                        TextFont {
                            font: font.clone(),
                            font_size: tokens::FONT_LG,
                            ..Default::default()
                        },
                        TextColor(tokens::TEXT_SECONDARY),
                    ));

                    // CWD option (if it looks like a project)
                    if cwd_has_project {
                        let cwd_name = cwd
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| cwd.to_string_lossy().to_string());
                        let cwd_clone = cwd.clone();
                        spawn_project_row(
                            card,
                            &cwd_name,
                            &cwd.to_string_lossy(),
                            font.clone(),
                            icon_font_handle.clone(),
                            cwd_clone,
                            true,
                        );
                    }

                    // Recent projects
                    if !recent.projects.is_empty() {
                        card.spawn((
                            Text::new("Recent Projects"),
                            TextFont {
                                font: font.clone(),
                                font_size: tokens::FONT_MD,
                                ..Default::default()
                            },
                            TextColor(tokens::TEXT_SECONDARY),
                            Node {
                                margin: UiRect::top(Val::Px(8.0)),
                                ..Default::default()
                            },
                        ));

                        for entry in &recent.projects {
                            // Skip CWD if already shown above
                            if cwd_has_project && entry.path == cwd {
                                continue;
                            }
                            spawn_project_row(
                                card,
                                &entry.name,
                                &entry.path.to_string_lossy(),
                                font.clone(),
                                icon_font_handle.clone(),
                                entry.path.clone(),
                                false,
                            );
                        }
                    }

                    // New Extension / New Game row
                    let new_row = card
                        .spawn(Node {
                            flex_direction: FlexDirection::Row,
                            column_gap: Val::Px(8.0),
                            margin: UiRect::top(Val::Px(8.0)),
                            ..Default::default()
                        })
                        .id();
                    spawn_new_project_button(
                        card,
                        new_row,
                        "+ New Extension",
                        font.clone(),
                        TemplatePreset::Extension,
                    );
                    spawn_new_project_button(
                        card,
                        new_row,
                        "+ New Game",
                        font.clone(),
                        TemplatePreset::Game,
                    );
                    spawn_new_project_button(
                        card,
                        new_row,
                        "+ From URL…",
                        font.clone(),
                        TemplatePreset::Custom(String::new()),
                    );

                    // Browse button
                    let browse_entity = card
                        .spawn((
                            Node {
                                padding: UiRect::axes(Val::Px(20.0), Val::Px(10.0)),
                                border_radius: BorderRadius::all(Val::Px(tokens::BORDER_RADIUS_MD)),
                                margin: UiRect::top(Val::Px(4.0)),
                                justify_content: JustifyContent::Center,
                                ..Default::default()
                            },
                            BackgroundColor(tokens::SELECTED_BG),
                            children![(
                                Text::new("Open existing project..."),
                                TextFont {
                                    font: font.clone(),
                                    font_size: tokens::FONT_LG,
                                    ..Default::default()
                                },
                                TextColor(tokens::TEXT_PRIMARY),
                            )],
                        ))
                        .id();

                    // Hover effects for browse button
                    card.commands().entity(browse_entity).observe(
                        |hover: On<Pointer<Over>>, mut bg: Query<&mut BackgroundColor>| {
                            if let Ok(mut bg) = bg.get_mut(hover.event_target()) {
                                bg.0 = tokens::SELECTED_BORDER;
                            }
                        },
                    );
                    card.commands().entity(browse_entity).observe(
                        |out: On<Pointer<Out>>, mut bg: Query<&mut BackgroundColor>| {
                            if let Ok(mut bg) = bg.get_mut(out.event_target()) {
                                bg.0 = tokens::SELECTED_BG;
                            }
                        },
                    );
                    card.commands()
                        .entity(browse_entity)
                        .observe(spawn_browse_dialog);
                });
        });
}

fn spawn_project_row(
    parent: &mut ChildSpawnerCommands,
    name: &str,
    path_display: &str,
    font: Handle<Font>,
    icon_font: Handle<Font>,
    project_path: PathBuf,
    is_cwd: bool,
) {
    // Outer row: info column on left, optional X button on right
    let row_entity = parent
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                width: Val::Percent(100.0),
                padding: UiRect::all(Val::Px(10.0)),
                border_radius: BorderRadius::all(Val::Px(tokens::BORDER_RADIUS_MD)),
                align_items: AlignItems::Center,
                ..Default::default()
            },
            BackgroundColor(tokens::TOOLBAR_BG),
        ))
        .id();

    // Left side: info column (flex_grow so it fills space)
    let info_column = parent
        .commands()
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                flex_grow: 1.0,
                row_gap: Val::Px(2.0),
                ..Default::default()
            },
            children![
                (
                    Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(8.0),
                        align_items: AlignItems::Center,
                        ..Default::default()
                    },
                    children![
                        (
                            Text::new(name.to_string()),
                            TextFont {
                                font: font.clone(),
                                font_size: tokens::FONT_LG,
                                ..Default::default()
                            },
                            TextColor(tokens::TEXT_PRIMARY),
                        ),
                        if_cwd_badge(is_cwd, font.clone()),
                    ],
                ),
                (
                    Text::new(path_display.to_string()),
                    TextFont {
                        font: font.clone(),
                        font_size: tokens::FONT_SM,
                        ..Default::default()
                    },
                    TextColor(tokens::TEXT_SECONDARY),
                ),
            ],
            Pickable::IGNORE,
        ))
        .id();

    parent.commands().entity(row_entity).add_child(info_column);

    // Right side: X button (only for recent projects, not CWD)
    if !is_cwd {
        let remove_path = project_path.clone();
        let x_button = parent
            .commands()
            .spawn(icon_button(
                IconButtonProps::new(Icon::X).variant(ButtonVariant::Ghost),
                &icon_font,
            ))
            .id();

        // X button click: remove from recent + despawn row
        parent.commands().entity(x_button).observe(
            move |mut click: On<Pointer<Click>>, mut commands: Commands| {
                click.propagate(false);
                let path = remove_path.clone();
                project::remove_recent(&path);
                commands.entity(row_entity).try_despawn();
            },
        );

        parent.commands().entity(row_entity).add_child(x_button);
    }

    // Hover effects on the row
    parent.commands().entity(row_entity).observe(
        |hover: On<Pointer<Over>>, mut bg: Query<&mut BackgroundColor>| {
            if let Ok(mut bg) = bg.get_mut(hover.event_target()) {
                bg.0 = tokens::HOVER_BG;
            }
        },
    );
    parent.commands().entity(row_entity).observe(
        |out: On<Pointer<Out>>, mut bg: Query<&mut BackgroundColor>| {
            if let Ok(mut bg) = bg.get_mut(out.event_target()) {
                bg.0 = tokens::TOOLBAR_BG;
            }
        },
    );

    // Click: select project
    parent.commands().entity(row_entity).observe(
        move |_: On<Pointer<Click>>, mut commands: Commands| {
            let path = project_path.clone();
            commands.queue(move |world: &mut World| {
                enter_project(world, path);
            });
        },
    );
}

fn if_cwd_badge(is_cwd: bool, font: Handle<Font>) -> impl Bundle {
    let text = if is_cwd { "current dir" } else { "" };
    (
        Text::new(text.to_string()),
        TextFont {
            font,
            font_size: tokens::FONT_SM,
            ..Default::default()
        },
        TextColor(tokens::TEXT_ACCENT),
    )
}

fn spawn_browse_dialog(
    _: On<Pointer<Click>>,
    mut commands: Commands,
    raw_handle: Query<&RawHandleWrapper, With<PrimaryWindow>>,
) {
    let mut dialog = AsyncFileDialog::new().set_title("Select project folder");

    if let Ok(rh) = raw_handle.single() {
        // SAFETY: called on the main thread during an observer
        let handle = unsafe { rh.get_handle() };
        dialog = dialog.set_parent(&handle);
    }

    let task = AsyncComputeTaskPool::get().spawn(async move { dialog.pick_folder().await });
    commands.insert_resource(FolderDialogTask(task));
}

fn poll_folder_dialog(world: &mut World) {
    let Some(mut task_res) = world.get_resource_mut::<FolderDialogTask>() else {
        return;
    };
    let Some(result) = future::block_on(future::poll_once(&mut task_res.0)) else {
        return;
    };
    world.remove_resource::<FolderDialogTask>();

    if let Some(handle) = result {
        let path = handle.path().to_path_buf();
        enter_project(world, path);
    }
}

/// Entry point for **every** "open a project" action from the
/// launcher (new-scaffold completion, recent-project click, manual
/// folder browse). If the project has a `Cargo.toml`, we kick off a
/// `cargo build` task and the poller decides whether to restart
/// (game) or transition into the editor (extension / non-building
/// project) once it finishes. If there's no `Cargo.toml`, we
/// transition straight to the editor.
///
/// All per-session rebuilds therefore happen at the launcher, never
/// mid-edit. Games' restart-to-activate requirement becomes
/// invisible — the launcher → editor transition already carries a
/// build step, so folding a process restart into it is just a
/// slightly-longer wait.
pub fn enter_project(world: &mut World, root: PathBuf) {
    enter_project_with(world, root, false);
}

/// Same as [`enter_project`] but lets the caller bypass the build
/// step. Used by the post-restart auto-open path: the parent
/// process already produced the dylib, the loader picked it up at
/// startup, so a second build-and-install would either be a no-op
/// or (for games) trigger another restart loop.
pub fn enter_project_with(world: &mut World, root: PathBuf, skip_build: bool) {
    if skip_build || !root.join("Cargo.toml").is_file() {
        transition_to_editor(world, root);
        return;
    }

    let project_name = root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("project")
        .to_owned();
    {
        let mut state = world.resource_mut::<NewProjectState>();
        state.pending_project = Some(root.clone());
        state.status = Some(format!(
            "Building `{project_name}` (first build compiles bevy — a few minutes)…"
        ));
    }

    let root_for_task = root;
    let task = AsyncComputeTaskPool::get()
        .spawn(async move { crate::ext_build::build_extension_project(&root_for_task) });
    world.resource_mut::<NewProjectState>().build_task = Some(task);
}

/// Apply the project-root state change and flip `AppState` to
/// `Editor`. Called from [`enter_project`] (no build needed) and
/// from the build-complete poller (build finished, transitioning).
fn transition_to_editor(world: &mut World, root: PathBuf) {
    let config = project::load_project_config(&root)
        .unwrap_or_else(|| project::create_default_project(&root));

    project::touch_recent(&root, &config.project.name);

    world.insert_resource(ProjectRoot {
        root: root.clone(),
        config,
    });

    // Despawn the launcher UI.
    let mut to_despawn = Vec::new();
    let mut query = world.query_filtered::<Entity, With<ProjectSelectorRoot>>();
    for entity in query.iter(world) {
        to_despawn.push(entity);
    }
    for entity in to_despawn {
        if let Ok(ec) = world.get_entity_mut(entity) {
            ec.despawn();
        }
    }

    let mut next_state = world.resource_mut::<NextState<AppState>>();
    next_state.set(AppState::Editor);
}

/// Spawn a pill-style button inside the "+ New Extension / + New
/// Game" row. Clicking opens the New Project modal with the given
/// preset already selected.
fn spawn_new_project_button(
    card: &mut ChildSpawnerCommands,
    parent: Entity,
    label: &str,
    font: Handle<Font>,
    preset: TemplatePreset,
) {
    let button = card
        .commands()
        .spawn((
            Node {
                padding: UiRect::axes(Val::Px(16.0), Val::Px(8.0)),
                border_radius: BorderRadius::all(Val::Px(tokens::BORDER_RADIUS_MD)),
                justify_content: JustifyContent::Center,
                ..Default::default()
            },
            BackgroundColor(tokens::TOOLBAR_BG),
            children![(
                Text::new(label.to_string()),
                TextFont {
                    font,
                    font_size: tokens::FONT_MD,
                    ..Default::default()
                },
                TextColor(tokens::TEXT_PRIMARY),
            )],
        ))
        .id();

    card.commands().entity(parent).add_child(button);

    card.commands().entity(button).observe(
        |hover: On<Pointer<Over>>, mut bg: Query<&mut BackgroundColor>| {
            if let Ok(mut bg) = bg.get_mut(hover.event_target()) {
                bg.0 = tokens::HOVER_BG;
            }
        },
    );
    card.commands().entity(button).observe(
        |out: On<Pointer<Out>>, mut bg: Query<&mut BackgroundColor>| {
            if let Ok(mut bg) = bg.get_mut(out.event_target()) {
                bg.0 = tokens::TOOLBAR_BG;
            }
        },
    );
    card.commands()
        .entity(button)
        .observe(move |_: On<Pointer<Click>>, mut commands: Commands| {
            let preset = preset.clone();
            commands.queue(move |world: &mut World| {
                open_new_project_modal(world, preset);
            });
        });
}

/// Tear down any existing New Project modal. Idempotent.
pub fn close_new_project_modal(world: &mut World) {
    let mut q = world.query_filtered::<Entity, With<NewProjectModalRoot>>();
    let entities: Vec<Entity> = q.iter(world).collect();
    for entity in entities {
        if let Ok(ec) = world.get_entity_mut(entity) {
            ec.despawn();
        }
    }
    let mut state = world.resource_mut::<NewProjectState>();
    state.preset = None;
    state.folder_task = None;
    state.scaffold_task = None;
    state.status = None;
}

/// Show the New Project modal with the given preset pre-selected.
///
/// Callable from any `AppState` — the launcher (`ProjectSelect`)
/// and the editor's **File → New Project** menu both invoke this.
/// The modal is a full-window overlay so it renders regardless of
/// which camera is active.
pub fn open_new_project_modal(world: &mut World, preset: TemplatePreset) {
    close_new_project_modal(world);

    let location = default_projects_dir();
    {
        let mut state = world.resource_mut::<NewProjectState>();
        state.preset = Some(preset.clone());
        state.location = location.clone();
        state.status = None;
    }

    let editor_font = world.resource::<EditorFont>().0.clone();
    let (heading, name_placeholder) = match preset {
        TemplatePreset::Extension => ("New Extension", "my_extension"),
        TemplatePreset::Game => ("New Game", "my_game"),
        TemplatePreset::Custom(_) => ("New Project", "my_project"),
    };

    // Full-window scrim that catches clicks behind the modal.
    let scrim = world
        .spawn((
            NewProjectModalRoot,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..Default::default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
            GlobalZIndex(100),
        ))
        .id();

    // Modal card.
    let card = world
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(12.0),
                padding: UiRect::all(Val::Px(24.0)),
                min_width: Val::Px(420.0),
                max_width: Val::Px(520.0),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(8.0)),
                ..Default::default()
            },
            BackgroundColor(tokens::PANEL_BG),
            BorderColor::all(tokens::BORDER_SUBTLE),
            ChildOf(scrim),
        ))
        .id();

    // Heading
    world.spawn((
        Text::new(heading.to_string()),
        TextFont {
            font: editor_font.clone(),
            font_size: 24.0,
            ..Default::default()
        },
        TextColor(tokens::TEXT_PRIMARY),
        ChildOf(card),
    ));

    // Name field
    world.spawn((
        Text::new("Name"),
        TextFont {
            font: editor_font.clone(),
            font_size: tokens::FONT_SM,
            ..Default::default()
        },
        TextColor(tokens::TEXT_SECONDARY),
        ChildOf(card),
    ));
    world.spawn((
        NewProjectNameInput,
        ChildOf(card),
        text_edit(
            TextEditProps::default()
                .with_placeholder(name_placeholder.to_string())
                .with_default_value(name_placeholder.to_string())
                .auto_focus(),
        ),
    ));

    // Location field
    world.spawn((
        Text::new("Location"),
        TextFont {
            font: editor_font.clone(),
            font_size: tokens::FONT_SM,
            ..Default::default()
        },
        TextColor(tokens::TEXT_SECONDARY),
        ChildOf(card),
    ));
    let location_row = world
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(8.0),
                ..Default::default()
            },
            ChildOf(card),
        ))
        .id();
    world.spawn((
        NewProjectLocationText,
        Text::new(location.to_string_lossy().into_owned()),
        TextFont {
            font: editor_font.clone(),
            font_size: tokens::FONT_MD,
            ..Default::default()
        },
        TextColor(tokens::TEXT_PRIMARY),
        Node {
            flex_grow: 1.0,
            ..Default::default()
        },
        ChildOf(location_row),
    ));
    let browse = world
        .spawn((
            NewProjectBrowseButton,
            Node {
                padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                border_radius: BorderRadius::all(Val::Px(tokens::BORDER_RADIUS_MD)),
                ..Default::default()
            },
            BackgroundColor(tokens::TOOLBAR_BG),
            children![(
                Text::new("Browse…"),
                TextFont {
                    font: editor_font.clone(),
                    font_size: tokens::FONT_SM,
                    ..Default::default()
                },
                TextColor(tokens::TEXT_PRIMARY),
            )],
            ChildOf(location_row),
        ))
        .id();
    world.entity_mut(browse).observe(on_browse_new_location);

    // Template URL field — prefilled from the preset so Extension /
    // Game paths don't require typing, and editable so power users
    // can point at their own templates.
    world.spawn((
        Text::new("Template"),
        TextFont {
            font: editor_font.clone(),
            font_size: tokens::FONT_SM,
            ..Default::default()
        },
        TextColor(tokens::TEXT_SECONDARY),
        ChildOf(card),
    ));
    world.spawn((
        NewProjectTemplateInput,
        ChildOf(card),
        text_edit(
            TextEditProps::default()
                .with_placeholder("https://github.com/…/your_template".to_string())
                .with_default_value(preset.url())
                .allow_empty(),
        ),
    ));

    // Status line
    world.spawn((
        NewProjectStatusText,
        Text::new(String::new()),
        TextFont {
            font: editor_font.clone(),
            font_size: tokens::FONT_SM,
            ..Default::default()
        },
        TextColor(tokens::TEXT_SECONDARY),
        ChildOf(card),
    ));

    // Action buttons
    let actions = world
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                justify_content: JustifyContent::FlexEnd,
                column_gap: Val::Px(8.0),
                margin: UiRect::top(Val::Px(8.0)),
                ..Default::default()
            },
            ChildOf(card),
        ))
        .id();

    let cancel = world
        .spawn((
            NewProjectCancelButton,
            Node {
                padding: UiRect::axes(Val::Px(16.0), Val::Px(8.0)),
                border_radius: BorderRadius::all(Val::Px(tokens::BORDER_RADIUS_MD)),
                ..Default::default()
            },
            BackgroundColor(tokens::TOOLBAR_BG),
            children![(
                Text::new("Cancel"),
                TextFont {
                    font: editor_font.clone(),
                    font_size: tokens::FONT_MD,
                    ..Default::default()
                },
                TextColor(tokens::TEXT_PRIMARY),
            )],
            ChildOf(actions),
        ))
        .id();
    world.entity_mut(cancel).observe(on_cancel_new_project);

    let create = world
        .spawn((
            NewProjectCreateButton,
            Node {
                padding: UiRect::axes(Val::Px(20.0), Val::Px(8.0)),
                border_radius: BorderRadius::all(Val::Px(tokens::BORDER_RADIUS_MD)),
                ..Default::default()
            },
            BackgroundColor(tokens::SELECTED_BG),
            children![(
                Text::new("Create"),
                TextFont {
                    font: editor_font,
                    font_size: tokens::FONT_MD,
                    ..Default::default()
                },
                TextColor(tokens::TEXT_PRIMARY),
            )],
            ChildOf(actions),
        ))
        .id();
    world.entity_mut(create).observe(on_create_new_project);
}

fn on_cancel_new_project(_: On<Pointer<Click>>, mut commands: Commands) {
    commands.queue(close_new_project_modal);
}

fn on_browse_new_location(
    _: On<Pointer<Click>>,
    mut commands: Commands,
    raw_handle: Query<&RawHandleWrapper, With<PrimaryWindow>>,
) {
    let mut dialog = AsyncFileDialog::new().set_title("Choose parent directory");
    if let Ok(rh) = raw_handle.single() {
        // SAFETY: called on the main thread during an observer.
        let handle = unsafe { rh.get_handle() };
        dialog = dialog.set_parent(&handle);
    }
    let task = AsyncComputeTaskPool::get().spawn(async move { dialog.pick_folder().await });
    commands.queue(move |world: &mut World| {
        world.resource_mut::<NewProjectState>().folder_task = Some(task);
    });
}

fn on_create_new_project(
    _: On<Pointer<Click>>,
    mut commands: Commands,
    name_inputs: Query<Entity, With<NewProjectNameInput>>,
    template_inputs: Query<Entity, With<NewProjectTemplateInput>>,
    text_edit_values: Query<&TextEditValue>,
) {
    // Read the name + template URL from the text inputs' synced
    // TextEditValue.
    let Some(name_entity) = name_inputs.iter().next() else {
        return;
    };
    let name = text_edit_values
        .get(name_entity)
        .map(|v| v.0.trim().to_string())
        .unwrap_or_default();
    let template_url_from_input = template_inputs
        .iter()
        .next()
        .and_then(|e| text_edit_values.get(e).ok())
        .map(|v| v.0.trim().to_string())
        .unwrap_or_default();

    commands.queue(move |world: &mut World| {
        let location = {
            let state = world.resource::<NewProjectState>();
            if state.preset.is_none() {
                return;
            }
            if state.scaffold_task.is_some() {
                return; // already running
            }
            state.location.clone()
        };

        let name = name.clone();
        if name.is_empty() {
            world.resource_mut::<NewProjectState>().status =
                Some("Please enter a project name.".into());
            return;
        }
        let template_url = template_url_from_input.clone();
        if template_url.is_empty() {
            world.resource_mut::<NewProjectState>().status =
                Some("Please enter a template URL.".into());
            return;
        }

        let name_for_task = name.clone();
        let location_for_task = location.clone();
        let url_for_task = template_url.clone();

        world.resource_mut::<NewProjectState>().status = Some(format!("Scaffolding `{name}`…"));

        let task = AsyncComputeTaskPool::get().spawn(async move {
            scaffold_project(&name_for_task, &location_for_task, &url_for_task)
        });
        world.resource_mut::<NewProjectState>().scaffold_task = Some(task);
    });
}

fn poll_new_project_tasks(
    mut state: ResMut<NewProjectState>,
    mut commands: Commands,
    mut location_texts: Query<&mut Text, With<NewProjectLocationText>>,
    mut status_texts: Query<
        &mut Text,
        (With<NewProjectStatusText>, Without<NewProjectLocationText>),
    >,
) {
    // Folder picker.
    if let Some(task) = state.folder_task.as_mut() {
        if let Some(result) = future::block_on(future::poll_once(task)) {
            state.folder_task = None;
            if let Some(handle) = result {
                state.location = handle.path().to_path_buf();
            }
        }
    }

    // Scaffold.
    if let Some(task) = state.scaffold_task.as_mut() {
        if let Some(result) = future::block_on(future::poll_once(task)) {
            state.scaffold_task = None;
            match result {
                Ok(project_path) => {
                    info!("Scaffolded project at {}", project_path.display());
                    // Immediately chain the first build so the user
                    // lands in the editor with the dylib installed.
                    // The first build compiles all of bevy and takes
                    // a couple of minutes — surface that in the
                    // status message so it doesn't look hung.
                    state.status = Some(
                        "Compiling initial build (first build compiles bevy — a few minutes)…"
                            .to_string(),
                    );
                    state.pending_project = Some(project_path.clone());
                    let project_for_task = project_path;
                    state.build_task = Some(AsyncComputeTaskPool::get().spawn(async move {
                        crate::ext_build::build_extension_project(&project_for_task)
                    }));
                }
                Err(err) => {
                    warn!("Scaffold failed: {err}");
                    state.status = Some(format!("Create failed: {err}"));
                }
            }
        }
    }

    // Build task completed (used by both scaffold-then-build and
    // open-then-build). Install the produced `.so`, then either
    // restart (game → systems need startup-time registration) or
    // transition directly into the editor (extension / live-load).
    if let Some(task) = state.build_task.as_mut() {
        if let Some(result) = future::block_on(future::poll_once(task)) {
            state.build_task = None;
            match result {
                Ok(artifact) => {
                    info!("Build produced {}", artifact.display());
                    let project = state.pending_project.take();
                    commands.queue(move |world: &mut World| {
                        let kind = crate::extensions_dialog::handle_install_from_path(
                            world,
                            artifact,
                        );
                        close_new_project_modal(world);
                        if matches!(kind, Some(jackdaw_loader::LoadedKind::Game(_))) {
                            // Persist the project as "last opened" so
                            // the respawned process reopens it.
                            if let Some(p) = &project {
                                let config = project::load_project_config(p)
                                    .unwrap_or_else(|| project::create_default_project(p));
                                project::touch_recent(p, &config.project.name);
                            }
                            info!("Auto-restarting jackdaw to activate the newly-installed game.");
                            crate::restart::restart_jackdaw();
                        } else if let Some(p) = project {
                            transition_to_editor(world, p);
                        }
                    });
                }
                Err(err) => {
                    warn!("Build failed: {err}");
                    state.status = Some(format!(
                        "Build failed: {err}.\n\
                         Fix the issue and try opening the project again."
                    ));
                    state.pending_project = None;
                }
            }
        }
    }

    // Sync UI.
    let desired_location = state.location.to_string_lossy().into_owned();
    for mut text in location_texts.iter_mut() {
        if text.0 != desired_location {
            text.0 = desired_location.clone();
        }
    }
    let desired_status = state.status.as_deref().unwrap_or("").to_string();
    for mut text in status_texts.iter_mut() {
        if text.0 != desired_status {
            text.0 = desired_status.clone();
        }
    }
}
