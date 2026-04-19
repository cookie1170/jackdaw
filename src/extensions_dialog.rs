//! File > Extensions dialog. Toggles compiled-in extensions at runtime
//! and persists the current state to `extensions.json`.

use bevy::{
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, futures_lite::future},
};
use jackdaw_api::{Extension, ExtensionCatalog, ExtensionKind};
use jackdaw_feathers::{
    button::{ButtonClickEvent, ButtonProps, ButtonSize, ButtonVariant, button},
    checkbox::{CheckboxCommitEvent, CheckboxProps, checkbox},
    dialog::{CloseDialogEvent, DialogChildrenSlot, OpenDialogEvent},
    icons::{EditorFont, Icon, IconFont},
    tokens,
};
use rfd::{AsyncFileDialog, FileHandle};

use crate::extensions_config;

pub struct ExtensionsDialogPlugin;

impl Plugin for ExtensionsDialogPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ExtensionsDialogOpen>()
            .init_resource::<InstallStatus>()
            .add_systems(Update, populate_extensions_dialog)
            .add_systems(Update, poll_install_task)
            .add_observer(on_extension_checkbox_commit)
            .add_observer(on_install_button_click)
            .add_observer(on_dialog_closed);
    }
}

fn on_dialog_closed(_: On<CloseDialogEvent>, mut open: ResMut<ExtensionsDialogOpen>) {
    open.0 = false;
}

#[derive(Resource, Default)]
struct ExtensionsDialogOpen(bool);

/// Records the extension name on each checkbox so the commit observer
/// can look up which one to toggle.
#[derive(Component)]
struct ExtensionCheckbox {
    extension_name: String,
}

/// Marks the "Install from file..." button. A single click observer
/// resolves the button entity by querying for this component, so
/// adding more buttons won't cross-fire.
#[derive(Component)]
struct InstallFromFileButton;

/// Marks the status text row that sits under the install button.
/// Whenever an install finishes (or fails), the task poller replaces
/// its text.
#[derive(Component)]
struct InstallStatusText;

/// Marks the top-level list node inside the dialog. Cascade-
/// despawned after an install succeeds so
/// `populate_extensions_dialog` rebuilds from the updated catalog.
#[derive(Component)]
struct ExtensionsDialogContent;

/// Holds the in-flight file-picker task, if any. Populated when the
/// user clicks the install button; drained by `poll_install_task`
/// once the user picks (or cancels).
#[derive(Resource, Default)]
struct InstallStatus {
    task: Option<Task<Option<FileHandle>>>,
    /// Last user-visible message. Survives dialog re-opens so users
    /// can click around and come back to the success/failure line.
    message: Option<String>,
}

pub fn open_extensions_dialog(world: &mut World) {
    world.resource_mut::<ExtensionsDialogOpen>().0 = true;
    world.trigger(
        OpenDialogEvent::new("Extensions", "Close")
            .without_cancel()
            .with_max_width(Val::Px(380.0)),
    );
}

/// Fill the dialog's children slot with a row per catalog entry.
///
/// The slot is found by marker presence rather than `&Children` because
/// a freshly-spawned `DialogChildrenSlot` has no `Children` component
/// yet. Checking for existing `ExtensionCheckbox` entities prevents
/// double-populating a re-opened dialog.
fn populate_extensions_dialog(
    mut commands: Commands,
    catalog: Res<ExtensionCatalog>,
    open: Res<ExtensionsDialogOpen>,
    slots: Query<Entity, With<DialogChildrenSlot>>,
    loaded: Query<&Extension>,
    editor_font: Res<EditorFont>,
    icon_font: Res<IconFont>,
    existing: Query<(), With<ExtensionCheckbox>>,
) {
    if !open.0 {
        return;
    }
    if !existing.is_empty() {
        return;
    }
    let Some(slot_entity) = slots.iter().next() else {
        return;
    };

    let font = editor_font.0.clone();
    let ifont = icon_font.0.clone();

    // Split catalog entries into Built-in vs. Custom. Membership comes
    // from each extension's declared `ExtensionKind`.
    let enabled_names: std::collections::HashSet<String> =
        loaded.iter().map(|e| e.name.clone()).collect();
    let mut builtin_rows: Vec<(String, bool)> = Vec::new();
    let mut custom_rows: Vec<(String, bool)> = Vec::new();
    for (name, kind) in catalog.iter_with_kind() {
        let row = (name.to_string(), enabled_names.contains(name));
        match kind {
            ExtensionKind::Builtin => builtin_rows.push(row),
            ExtensionKind::Custom => custom_rows.push(row),
        }
    }
    builtin_rows.sort_by(|a, b| a.0.cmp(&b.0));
    custom_rows.sort_by(|a, b| a.0.cmp(&b.0));

    let list = commands
        .spawn((
            ChildOf(slot_entity),
            ExtensionsDialogContent,
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(tokens::SPACING_XS),
                min_width: Val::Px(280.0),
                ..default()
            },
        ))
        .id();

    spawn_section_header(&mut commands, list, "Built-in");
    for (name, checked) in builtin_rows {
        let label = prettify(&name);
        commands.spawn((
            ChildOf(list),
            ExtensionCheckbox {
                extension_name: name.clone(),
            },
            checkbox(CheckboxProps::new(label).checked(checked), &font, &ifont),
        ));
    }

    spawn_section_header(&mut commands, list, "Custom");
    if custom_rows.is_empty() {
        commands.spawn((
            ChildOf(list),
            Node {
                padding: UiRect::axes(Val::Px(tokens::SPACING_LG), Val::Px(tokens::SPACING_SM)),
                ..default()
            },
            children![(
                Text::new("No custom extensions installed"),
                TextFont {
                    font_size: tokens::FONT_SM,
                    ..default()
                },
                TextColor(tokens::TEXT_SECONDARY),
            )],
        ));
    } else {
        for (name, checked) in custom_rows {
            let label = prettify(&name);
            commands.spawn((
                ChildOf(list),
                ExtensionCheckbox {
                    extension_name: name.clone(),
                },
                checkbox(CheckboxProps::new(label).checked(checked), &font, &ifont),
            ));
        }
    }

    spawn_install_row(&mut commands, list);
}

/// Compose the install/build buttons plus the shared status line
/// under them. Lives inside `populate_extensions_dialog` so it's
/// rebuilt every time the dialog opens.
fn spawn_install_row(commands: &mut Commands, list: Entity) {
    let row = commands
        .spawn((
            ChildOf(list),
            Node {
                flex_direction: FlexDirection::Column,
                padding: UiRect::axes(Val::Px(tokens::SPACING_LG), Val::Px(tokens::SPACING_SM)),
                row_gap: Val::Px(tokens::SPACING_XS),
                ..default()
            },
        ))
        .id();

    // Only "install a prebuilt .so" lives in the editor: source-tree
    // builds happen at the launcher (File > Home) so every build
    // carries its potential process-restart with it. This keeps
    // mid-session surprises (sudden restart when clicking Build)
    // out of the editor experience.
    commands.spawn((
        ChildOf(row),
        InstallFromFileButton,
        button(
            ButtonProps::new("Install prebuilt dylib…")
                .with_variant(ButtonVariant::Default)
                .with_size(ButtonSize::MD)
                .with_left_icon(Icon::FilePlus),
        ),
    ));

    commands.spawn((
        ChildOf(row),
        InstallStatusText,
        Text::new(String::new()),
        TextFont {
            font_size: tokens::FONT_SM,
            ..default()
        },
        TextColor(tokens::TEXT_SECONDARY),
    ));
}

/// Underlined heading matching the Add Component dialog's style.
fn spawn_section_header(commands: &mut Commands, list: Entity, label: &str) {
    let header = commands
        .spawn((
            ChildOf(list),
            Node {
                padding: UiRect::new(
                    Val::Px(tokens::SPACING_LG),
                    Val::Px(tokens::SPACING_LG),
                    Val::Px(tokens::SPACING_MD),
                    Val::Px(tokens::SPACING_XS),
                ),
                width: Val::Percent(100.0),
                border: UiRect::bottom(Val::Px(1.0)),
                ..default()
            },
            BorderColor::all(tokens::BORDER_SUBTLE),
        ))
        .id();

    commands.spawn((
        ChildOf(header),
        Text::new(label.to_string()),
        TextFont {
            font_size: tokens::FONT_SM,
            ..default()
        },
        TextColor(tokens::TEXT_SECONDARY),
    ));
}

/// Enable or disable the matching extension when a checkbox commits,
/// then persist the new enabled list.
fn on_extension_checkbox_commit(
    event: On<CheckboxCommitEvent>,
    checkboxes: Query<&ExtensionCheckbox>,
    mut commands: Commands,
) {
    let Ok(cb) = checkboxes.get(event.entity) else {
        return;
    };
    let name = cb.extension_name.clone();
    let checked = event.checked;

    commands.queue(move |world: &mut World| {
        if checked {
            jackdaw_api::enable_extension(world, &name);
        } else {
            jackdaw_api::disable_extension(world, &name);
        }
        extensions_config::persist_current_enabled(world);
    });
}

/// Spawn an rfd file picker when the install button is clicked.
/// Skips if a picker is already in flight (rfd can't run two at
/// once on some platforms, and it'd be confusing UX).
fn on_install_button_click(
    event: On<ButtonClickEvent>,
    buttons: Query<(), With<InstallFromFileButton>>,
    mut commands: Commands,
) {
    if buttons.get(event.entity).is_err() {
        return;
    }
    commands.queue(|world: &mut World| {
        if world.resource::<InstallStatus>().task.is_some() {
            return;
        }
        let dialog = AsyncFileDialog::new().add_filter(
            "Extension dylib",
            // Platform-specific extensions mirror what the loader
            // recognises (`jackdaw_loader::is_dylib`).
            &["so", "dylib", "dll"],
        );
        let task = AsyncComputeTaskPool::get().spawn(async move { dialog.pick_file().await });
        world.resource_mut::<InstallStatus>().task = Some(task);
        world.resource_mut::<InstallStatus>().message = Some("Select a dylib file…".into());
    });
}

/// Drive the file picker task to completion. On selection, queue a
/// command that copies the file into the extensions directory,
/// attempts a live-load (so the extension activates without
/// restarting), and refreshes the dialog list.
fn poll_install_task(
    mut status: ResMut<InstallStatus>,
    mut texts: Query<&mut Text, With<InstallStatusText>>,
    mut commands: Commands,
) {
    let Some(task) = status.task.as_mut() else {
        sync_status_text(&status.message, &mut texts);
        return;
    };

    let Some(handle) = future::block_on(future::poll_once(task)) else {
        sync_status_text(&status.message, &mut texts);
        return;
    };

    status.task = None;

    match handle {
        Some(picked) => {
            let src = picked.path().to_path_buf();
            commands.queue(move |world: &mut World| {
                let kind = handle_install(world, src);
                if matches!(kind, Some(jackdaw_loader::LoadedKind::Game(_))) {
                    info!("Auto-restarting jackdaw to activate the newly-installed game.");
                    crate::restart::restart_jackdaw();
                }
            });
        }
        None => {
            status.message = None;
        }
    }

    sync_status_text(&status.message, &mut texts);
}

fn sync_status_text(
    message: &Option<String>,
    texts: &mut Query<&mut Text, With<InstallStatusText>>,
) {
    let desired = message.as_deref().unwrap_or("");
    for mut text in texts.iter_mut() {
        if text.0 != desired {
            text.0 = desired.to_string();
        }
    }
}

/// Copy the picked file into the extensions directory, then live-
/// load it from the copy. Updates `InstallStatus.message` and
/// despawns the dialog's content so the list rebuilds on the next
/// frame.
/// Route a freshly-built `.so` / `.dylib` / `.dll` through the
/// install pipeline: peek kind, copy to `extensions/` or `games/`,
/// try to live-load, and set an `InstallStatus` message describing
/// the result.
///
/// Returns the loaded kind on success so callers can decide whether
/// a restart is needed to activate the dylib (games require it;
/// extensions don't). Exposed so the scaffold flow
/// (project_select.rs) can reuse the same code path the
/// Build-from-folder button uses.
pub fn handle_install_from_path(
    world: &mut World,
    src: std::path::PathBuf,
) -> Option<jackdaw_loader::LoadedKind> {
    handle_install(world, src)
}

fn handle_install(
    world: &mut World,
    src: std::path::PathBuf,
) -> Option<jackdaw_loader::LoadedKind> {
    let target = classify_for_install(&src);
    let dest = match install_picked_file(&src, target) {
        Ok(d) => d,
        Err(err) => {
            warn!("Failed to install dylib: {err}");
            world.resource_mut::<InstallStatus>().message = Some(format!("Install failed: {err}"));
            return None;
        }
    };
    info!("Installed dylib to {}", dest.display());

    let result = jackdaw_loader::load_from_path(world, &dest);
    let msg = match &result {
        Ok(jackdaw_loader::LoadedKind::Extension(name)) => {
            info!("Live-loaded extension `{name}` from {}", dest.display());
            format!("Loaded extension `{name}`. BEI keybinds (if any) activate on next restart.")
        }
        Ok(jackdaw_loader::LoadedKind::Game(name)) => {
            info!(
                "Registered game `{name}` from {}; systems will activate on restart.",
                dest.display()
            );
            format!(
                "Installed game `{name}`. Restarting jackdaw to activate it…",
            )
        }
        Err(err) => {
            warn!("Live-load failed for {}: {err}", dest.display());
            format!(
                "Installed to {}, but live-load failed: {err}. Restart the editor to retry.",
                dest.display()
            )
        }
    };
    world.resource_mut::<InstallStatus>().message = Some(msg);

    // Despawn the existing list so `populate_extensions_dialog`
    // rebuilds it from the now-updated catalog.
    let mut q = world.query_filtered::<Entity, With<ExtensionsDialogContent>>();
    let targets: Vec<Entity> = q.iter(world).collect();
    for entity in targets {
        if let Ok(ec) = world.get_entity_mut(entity) {
            ec.despawn();
        }
    }

    result.ok()
}

/// Which directory under the user's config root a given dylib
/// should be installed to.
enum InstallTarget {
    Extension,
    Game,
}

/// Copy the picked file into the correct per-user subdirectory based
/// on the dylib's entry symbol. Returns the destination path on
/// success. Creates the directory if missing.
fn install_picked_file(
    src: &std::path::Path,
    target: InstallTarget,
) -> std::io::Result<std::path::PathBuf> {
    let Some(config) = crate::project::config_dir() else {
        return Err(std::io::Error::other(
            "platform config directory is unavailable",
        ));
    };
    let subdir = match target {
        InstallTarget::Extension => "extensions",
        InstallTarget::Game => "games",
    };
    let dest_dir = config.join(subdir);
    std::fs::create_dir_all(&dest_dir)?;
    let file_name = src.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "picked path has no file name",
        )
    })?;
    let dest = dest_dir.join(file_name);
    std::fs::copy(src, &dest)?;
    Ok(dest)
}

/// Peek at the dylib's entry symbol to decide whether it belongs in
/// `extensions/` or `games/`. Falls back to Extension if the peek
/// fails — the caller's own load-from-path will surface the real
/// error on the follow-up dlopen.
fn classify_for_install(path: &std::path::Path) -> InstallTarget {
    match jackdaw_loader::peek_kind(path) {
        Ok(jackdaw_loader::LoadedKind::Game(_)) => InstallTarget::Game,
        _ => InstallTarget::Extension,
    }
}

/// Convert `"jackdaw.asset_browser"` → `"Asset Browser"`.
fn prettify(name: &str) -> String {
    let stripped = name.strip_prefix("jackdaw.").unwrap_or(name);
    let mut out = String::new();
    for (i, part) in stripped.split(&['_', '.'][..]).enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let mut chars = part.chars();
        if let Some(c) = chars.next() {
            out.extend(c.to_uppercase());
            out.push_str(chars.as_str());
        }
    }
    out
}
