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
                (
                    poll_folder_dialog,
                    poll_new_project_tasks,
                    refresh_build_progress_snapshot,
                    refresh_build_progress_ui,
                )
                    .run_if(in_state(AppState::ProjectSelect)),
            )
            // The dylib-install step MUST run outside of `Update`'s
            // `schedule_scope`. The game's `GameApp::add_systems(Update, …)`
            // inserts into `Schedules`; doing that while bevy has
            // `Update` checked out via `schedule_scope` causes the
            // modification to be overwritten when the scope re-inserts
            // at exit. `Last` has its own scope and doesn't clash.
            .add_systems(
                Last,
                apply_pending_install.run_if(in_state(AppState::ProjectSelect)),
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

/// Outer container for the progress-bar + log-tail UI, toggled on
/// when a build is in flight so the idle modal doesn't leave a
/// visual gap.
#[derive(Component)]
struct NewProjectProgressContainer;

/// Wraps the "currently compiling `<crate>`" label.
#[derive(Component)]
struct NewProjectProgressCrateLabel;

/// Wraps the `progress_bar` widget so the refresh system can walk
/// its fill child.
#[derive(Component)]
struct NewProjectProgressBarSlot;

/// Wraps the log-tail text; refreshed with the last 20 lines of
/// cargo output each frame.
#[derive(Component)]
struct NewProjectLogText;

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
    /// In-flight `cargo clean -p <crate>` triggered by the
    /// auto-recovery path when the first install fails with an
    /// SDK symbol mismatch. When this completes successfully, the
    /// poller re-runs `build_task` against the same project path.
    clean_task: Option<Task<Result<(), crate::ext_build::BuildError>>>,
    /// `true` after we've triggered the clean+rebuild once. Stops
    /// us from infinite-looping if a second build still fails with
    /// the same symbol mismatch (indicating the project has some
    /// other issue).
    retry_attempted: bool,
    /// Tunnel from the install-runs-in-commands-queue closure back
    /// into this poller. The closure pushes its install result
    /// here; the next frame the poller drains it and either
    /// proceeds or triggers the auto-clean recovery.
    metadata_outcome:
        Option<std::sync::Arc<std::sync::Mutex<Option<Result<(), jackdaw_loader::LoadError>>>>>,
    /// Artifact waiting to be installed by `apply_pending_install`
    /// (runs in `Last`, not `Update`, so modifications to the
    /// `Update` schedule by the game's `GameApp::add_systems` don't
    /// collide with `Update`'s active `schedule_scope`).
    pending_install: Option<PathBuf>,
    /// Shared progress sink the build task writes to. The
    /// `refresh_build_progress_ui` system reads a snapshot from
    /// here each frame and copies it into `build_progress_snapshot`
    /// so the modal's bar/log nodes can update without locking on
    /// the hot path.
    build_progress: Option<std::sync::Arc<std::sync::Mutex<crate::ext_build::BuildProgress>>>,
    /// Latest snapshot of `build_progress`, copied each frame.
    build_progress_snapshot: Option<crate::ext_build::BuildProgress>,
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
    // If the Cargo.toml is a plain binary crate (e.g., the editor's
    // own source tree, or any non-extension cargo project the user
    // points at) there's no cdylib for the loader to pick up. Skip
    // the build rather than compile the whole dep graph just to fail
    // the artifact check at the end.
    if !crate::ext_build::manifest_declares_cdylib(&root) {
        info!(
            "Project at {} has a Cargo.toml but no cdylib target — \
             opening without building.",
            root.display()
        );
        transition_to_editor(world, root);
        return;
    }

    let project_name = root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("project")
        .to_owned();

    // Show the "Opening project" modal so the user sees the build
    // + any auto-recovery retry rather than staring at a frozen
    // launcher. The scaffold flow already has its own modal; when
    // called from there we skip spawning a second one.
    let scaffold_modal_already_open = {
        let mut q = world.query_filtered::<Entity, With<NewProjectModalRoot>>();
        q.iter(world).next().is_some()
    };
    if !scaffold_modal_already_open {
        open_project_progress_modal(world, &project_name);
    }

    let progress = std::sync::Arc::new(std::sync::Mutex::new(
        crate::ext_build::BuildProgress::default(),
    ));
    {
        let mut state = world.resource_mut::<NewProjectState>();
        state.pending_project = Some(root.clone());
        state.status = Some(format!("Building `{project_name}`…"));
        state.build_progress = Some(std::sync::Arc::clone(&progress));
        state.build_progress_snapshot = Some(crate::ext_build::BuildProgress::default());
        state.retry_attempted = false;
    }

    let root_for_task = root;
    let progress_for_task = std::sync::Arc::clone(&progress);
    let task = AsyncComputeTaskPool::get().spawn(async move {
        crate::ext_build::build_extension_project_with_progress(
            &root_for_task,
            Some(progress_for_task),
        )
    });
    world.resource_mut::<NewProjectState>().build_task = Some(task);
}

/// Apply the project-root state change and flip `AppState` to
/// `Editor`. Called from [`enter_project`] (no build needed) and
/// from the build-complete poller (build finished, transitioning).
///
/// If the project has a file at `<root>/assets/scene.jsn`, that
/// scene is auto-loaded so the user lands in a populated editor
/// rather than an empty one. This is the convention the game
/// template ships with.
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

    // Intentionally no scene auto-load here. Once a project has
    // multiple `.jsn` scenes, auto-loading a conventional path
    // would be confusing ("why is jackdaw showing me this one?").
    // Users open scenes explicitly via **File → Open** or via the
    // asset-browser panel. Last-opened-scene persistence per
    // project is tracked as a follow-up so users don't have to
    // re-pick on every open; until then the editor opens empty
    // after transition.
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

/// Lightweight modal shown while `enter_project_with` builds an
/// **existing** project — the user picked a recent entry or
/// browsed to a folder and we need something visual while cargo
/// runs + the auto-recovery retry may fire. Reuses the same
/// `NewProjectProgressContainer` / `NewProjectProgressCrateLabel`
/// / progress-bar / log-tail markers as the scaffold modal, so
/// the existing `refresh_build_progress_ui` system drives it
/// without extra wiring. Despawned via `close_new_project_modal`.
pub fn open_project_progress_modal(world: &mut World, project_name: &str) {
    close_new_project_modal(world);

    let editor_font = world.resource::<EditorFont>().0.clone();

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

    let card = world
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(12.0),
                padding: UiRect::all(Val::Px(24.0)),
                min_width: Val::Px(480.0),
                max_width: Val::Px(720.0),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(tokens::BORDER_RADIUS_MD)),
                ..Default::default()
            },
            BackgroundColor(tokens::PANEL_BG),
            BorderColor::all(tokens::BORDER_SUBTLE),
            ChildOf(scrim),
        ))
        .id();

    world.spawn((
        Text::new(format!("Opening `{project_name}`")),
        TextFont {
            font: editor_font.clone(),
            font_size: tokens::FONT_LG,
            ..Default::default()
        },
        TextColor(tokens::TEXT_PRIMARY),
        ChildOf(card),
    ));

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

    // Progress container + children mirror the scaffold modal so
    // `refresh_build_progress_ui` walks the same marker chain.
    let progress_container = world
        .spawn((
            NewProjectProgressContainer,
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(6.0),
                margin: UiRect::top(Val::Px(8.0)),
                display: Display::None,
                ..Default::default()
            },
            ChildOf(card),
        ))
        .id();

    world.spawn((
        NewProjectProgressCrateLabel,
        Text::new(String::new()),
        TextFont {
            font: editor_font.clone(),
            font_size: tokens::FONT_SM,
            ..Default::default()
        },
        TextColor(tokens::TEXT_SECONDARY),
        ChildOf(progress_container),
    ));

    let bar_slot = world
        .spawn((
            NewProjectProgressBarSlot,
            Node {
                width: Val::Percent(100.0),
                ..Default::default()
            },
            ChildOf(progress_container),
        ))
        .id();
    world.spawn((
        jackdaw_feathers::progress::progress_bar(0.0),
        ChildOf(bar_slot),
    ));

    world.spawn((
        NewProjectLogText,
        Text::new(String::new()),
        TextFont {
            font: editor_font.clone(),
            font_size: tokens::FONT_SM,
            ..Default::default()
        },
        TextColor(tokens::TEXT_SECONDARY),
        Node {
            max_height: Val::Px(220.0),
            overflow: Overflow::clip(),
            ..Default::default()
        },
        ChildOf(progress_container),
    ));
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

    // Build-progress UI (hidden until a build is in flight).
    let progress_container = world
        .spawn((
            NewProjectProgressContainer,
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(6.0),
                margin: UiRect::top(Val::Px(8.0)),
                display: Display::None,
                ..Default::default()
            },
            ChildOf(card),
        ))
        .id();

    // "Compiling <crate>" label.
    world.spawn((
        NewProjectProgressCrateLabel,
        Text::new(String::new()),
        TextFont {
            font: editor_font.clone(),
            font_size: tokens::FONT_SM,
            ..Default::default()
        },
        TextColor(tokens::TEXT_SECONDARY),
        ChildOf(progress_container),
    ));

    // Progress bar slot wrapping the `progress_bar` widget.
    let bar_slot = world
        .spawn((
            NewProjectProgressBarSlot,
            Node {
                width: Val::Percent(100.0),
                ..Default::default()
            },
            ChildOf(progress_container),
        ))
        .id();
    world.spawn((
        jackdaw_feathers::progress::progress_bar(0.0),
        ChildOf(bar_slot),
    ));

    // Log tail — fixed-height scrollable-ish (we don't enable real
    // scrolling; text wraps naturally and oldest lines age out via
    // the 20-line ring buffer).
    world.spawn((
        NewProjectLogText,
        Text::new(String::new()),
        TextFont {
            font: editor_font.clone(),
            font_size: tokens::FONT_SM,
            ..Default::default()
        },
        TextColor(tokens::TEXT_SECONDARY),
        Node {
            max_height: Val::Px(220.0),
            overflow: Overflow::clip(),
            ..Default::default()
        },
        ChildOf(progress_container),
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
                    // Chain the first build with live progress
                    // surfacing. The scaffold modal is already open
                    // and will render the bar + log tail each frame.
                    let project_name = project_path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("project")
                        .to_owned();
                    state.status = Some(format!("Building `{project_name}`…"));
                    state.pending_project = Some(project_path.clone());

                    let progress = std::sync::Arc::new(std::sync::Mutex::new(
                        crate::ext_build::BuildProgress::default(),
                    ));
                    state.build_progress = Some(std::sync::Arc::clone(&progress));
                    state.build_progress_snapshot =
                        Some(crate::ext_build::BuildProgress::default());

                    let project_for_task = project_path;
                    let progress_for_task = std::sync::Arc::clone(&progress);
                    state.build_task = Some(AsyncComputeTaskPool::get().spawn(async move {
                        crate::ext_build::build_extension_project_with_progress(
                            &project_for_task,
                            Some(progress_for_task),
                        )
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
    // open-then-build). Install the produced `.so`, transition to
    // the editor, or — on an SDK-symbol-mismatch — kick off the
    // auto-recovery path: cargo clean -p <crate> then re-build.
    if let Some(task) = state.build_task.as_mut() {
        if let Some(result) = future::block_on(future::poll_once(task)) {
            state.build_task = None;
            match result {
                Ok(artifact) => {
                    info!("Build produced {}", artifact.display());
                    // Stash for `apply_pending_install` (Last
                    // schedule). Setting up the outcome tunnel
                    // still, so the install's result feeds back
                    // into our auto-recovery path.
                    let outcome: std::sync::Arc<
                        std::sync::Mutex<Option<Result<(), jackdaw_loader::LoadError>>>,
                    > = std::sync::Arc::new(std::sync::Mutex::new(None));
                    state.metadata_outcome = Some(outcome);
                    state.pending_install = Some(artifact);
                }
                Err(err) => {
                    warn!("Build failed: {err}");
                    state.status = Some(format!(
                        "Build failed: {err}.\n\
                         Fix the issue and try opening the project again."
                    ));
                    state.pending_project = None;
                    state.retry_attempted = false;
                }
            }
        }
    }

    // Install-outcome poller: reads the Arc<Mutex<...>> we handed
    // to the commands.queue closure. On Ok we're done. On an
    // Err-with-symbol-mismatch, kick off `cargo clean` (one retry
    // max, tracked via `retry_attempted`).
    if let Some(outcome) = state.metadata_outcome.clone() {
        let taken = {
            let Ok(mut slot) = outcome.lock() else {
                return;
            };
            slot.take()
        };
        if let Some(result) = taken {
            state.metadata_outcome = None;
            match result {
                Ok(()) => {
                    state.retry_attempted = false;
                }
                Err(err) if err.is_symbol_mismatch() && !state.retry_attempted => {
                    state.retry_attempted = true;
                    let Some(project) = state.pending_project.clone() else {
                        state.status = Some("Auto-recovery failed: no project context".into());
                        return;
                    };
                    state.status = Some(
                        "Editor SDK changed since last build — cleaning project cache…".into(),
                    );
                    let project_for_task = project;
                    state.clean_task = Some(AsyncComputeTaskPool::get().spawn(async move {
                        crate::ext_build::cargo_clean_project(&project_for_task)
                    }));
                }
                Err(err) => {
                    warn!("Install failed (no retry): {err}");
                    state.status = Some(format!(
                        "Install failed: {err}. Try opening the project again."
                    ));
                    state.pending_project = None;
                    state.retry_attempted = false;
                }
            }
        }
    }

    // Clean-task completed — kick off a fresh build.
    if let Some(task) = state.clean_task.as_mut() {
        if let Some(result) = future::block_on(future::poll_once(task)) {
            state.clean_task = None;
            match result {
                Ok(()) => {
                    let Some(project) = state.pending_project.clone() else {
                        return;
                    };
                    let project_name = project
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("project")
                        .to_owned();
                    state.status = Some(format!("Rebuilding `{project_name}` from scratch…"));
                    // Fresh progress sink — the old one had the prior
                    // build's log tail, which would mislead the user.
                    let progress = std::sync::Arc::new(std::sync::Mutex::new(
                        crate::ext_build::BuildProgress::default(),
                    ));
                    state.build_progress = Some(std::sync::Arc::clone(&progress));
                    state.build_progress_snapshot =
                        Some(crate::ext_build::BuildProgress::default());

                    let project_for_task = project;
                    let progress_for_task = std::sync::Arc::clone(&progress);
                    state.build_task = Some(AsyncComputeTaskPool::get().spawn(async move {
                        crate::ext_build::build_extension_project_with_progress(
                            &project_for_task,
                            Some(progress_for_task),
                        )
                    }));
                }
                Err(err) => {
                    warn!("Cargo clean failed: {err}");
                    state.status = Some(format!("Auto-recovery failed during cargo clean: {err}"));
                    state.pending_project = None;
                    state.retry_attempted = false;
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

/// Copy the shared `BuildProgress` into the per-frame snapshot so
/// the UI-refresh system can read from a plain struct without
/// holding the mutex across rendering.
fn refresh_build_progress_snapshot(mut state: ResMut<NewProjectState>) {
    let Some(ref arc) = state.build_progress else {
        return;
    };
    let snap = {
        let Ok(guard) = arc.lock() else {
            return;
        };
        guard.clone()
    };
    state.build_progress_snapshot = Some(snap);
}

/// Reflect the current snapshot into the modal's progress UI:
/// toggles the container, updates the "compiling `<crate>`" label,
/// scrubs the progress-bar fill, and sets the log-tail text.
fn refresh_build_progress_ui(
    state: Res<NewProjectState>,
    mut containers: Query<&mut Node, With<NewProjectProgressContainer>>,
    mut crate_labels: Query<
        &mut Text,
        (
            With<NewProjectProgressCrateLabel>,
            Without<NewProjectLogText>,
        ),
    >,
    mut log_texts: Query<
        &mut Text,
        (
            With<NewProjectLogText>,
            Without<NewProjectProgressCrateLabel>,
        ),
    >,
    bar_slots: Query<&Children, With<NewProjectProgressBarSlot>>,
    children_q: Query<&Children>,
    mut fill_q: Query<
        &mut Node,
        (
            With<jackdaw_feathers::progress::ProgressBarFill>,
            Without<NewProjectProgressContainer>,
        ),
    >,
) {
    let snapshot = state.build_progress_snapshot.as_ref();

    // Toggle container visibility based on whether a build is active.
    let show = snapshot.is_some();
    for mut node in containers.iter_mut() {
        let desired = if show { Display::Flex } else { Display::None };
        if node.display != desired {
            node.display = desired;
        }
    }

    let Some(progress) = snapshot else {
        return;
    };

    // "Compiling <crate>" or "Preparing…" if we don't know yet.
    let crate_line = match (&progress.current_crate, progress.artifacts_total) {
        (Some(name), Some(total)) => {
            format!("Compiling {name} ({}/{})", progress.artifacts_done, total)
        }
        (Some(name), None) => format!("Compiling {name} ({} so far)", progress.artifacts_done),
        (None, Some(total)) => format!("Preparing build… (0/{total})"),
        (None, None) => "Preparing build…".to_string(),
    };
    for mut t in crate_labels.iter_mut() {
        if t.0 != crate_line {
            t.0 = crate_line.clone();
        }
    }

    // Progress bar fill — walk slot → bar → bar children → fill.
    let fraction = progress.fraction().unwrap_or(0.0).clamp(0.0, 1.0);
    let desired_width = Val::Percent(fraction * 100.0);
    for bar_children in bar_slots.iter() {
        for bar_entity in bar_children.iter() {
            let Ok(inner) = children_q.get(bar_entity) else {
                continue;
            };
            for fill_entity in inner.iter() {
                if let Ok(mut node) = fill_q.get_mut(fill_entity) {
                    if node.width != desired_width {
                        node.width = desired_width;
                    }
                }
            }
        }
    }

    // Log tail.
    let mut joined = String::new();
    for (i, line) in progress.recent_log_lines.iter().enumerate() {
        if i > 0 {
            joined.push('\n');
        }
        joined.push_str(line);
    }
    for mut t in log_texts.iter_mut() {
        if t.0 != joined {
            t.0 = joined.clone();
        }
    }
}

/// Install a freshly-built game/extension dylib, running in the
/// `Last` schedule so `GameApp::add_systems(Update, …)` inside the
/// game's build function mutates `Update` while nobody holds it in
/// `schedule_scope`. See the plugin-registration block at the top
/// of this file for context.
///
/// Takes `&mut World` directly; reads `NewProjectState.pending_install`,
/// calls `handle_install_from_path`, writes the install result into
/// `NewProjectState.metadata_outcome` for the auto-recovery poller
/// to pick up on the next frame.
fn apply_pending_install(world: &mut World) {
    let artifact_opt = world
        .resource_mut::<NewProjectState>()
        .pending_install
        .take();
    let Some(artifact) = artifact_opt else {
        return;
    };
    let outcome_arc = world.resource::<NewProjectState>().metadata_outcome.clone();

    let result = crate::extensions_dialog::handle_install_from_path(world, artifact);
    let is_ok = result.is_ok();

    if let Some(arc) = outcome_arc {
        if let Ok(mut slot) = arc.lock() {
            *slot = Some(result.map(|_| ()));
        }
    }

    if is_ok {
        let project = world
            .resource_mut::<NewProjectState>()
            .pending_project
            .clone();
        close_new_project_modal(world);
        if let Some(p) = project {
            transition_to_editor(world, p);
        }
    }
}
