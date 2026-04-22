//! Play-In-Editor runtime.
//!
//! Jackdaw hosts a game's systems in its own `App` (same World,
//! not a SubApp). Games are dylibs loaded at startup via the
//! `jackdaw_game_entry_v1` FFI symbol; their `build(&mut App)`
//! callback registers systems into the editor's schedule. Game
//! systems gate their execution on [`PlayState::Playing`] so they
//! only tick when the user has Play engaged.
//!
//! This module provides:
//! - [`PlayState`] — the `Stopped` / `Playing` / `Paused` state.
//! - [`PrePlayScene`] — scene AST snapshot captured at Play time,
//!   restored on Stop so the authored scene is the revert baseline.
//! - [`PieButton`] — marker component for the toolbar transport
//!   buttons; the `PiePlugin` auto-wires a click observer to each.
//! - [`GameSpawned`] — marker added automatically to any entity that
//!   receives a `Transform` during `PlayState::Playing`. Editor
//!   surfaces (hierarchy, inspector) use it to distinguish
//!   authored-then-played entities from ones the game spawned.
//! - [`PiePlugin`] — registers state, resource, and observers.
//!
//! Handlers [`handle_play`], [`handle_pause`], [`handle_stop`] are
//! exposed for direct `commands.queue(...)` use in case other
//! surfaces (keybinds, menu entries) want to trigger PIE
//! transitions without going through a button.

use bevy::prelude::*;
use jackdaw_api::pie::PlayState;
use jackdaw_jsn::SceneJsnAst;

/// Frozen AST captured when the user clicks Play from `Stopped`.
/// Restored on Stop so any game-spawned entities or authored-entity
/// mutations are reverted.
#[derive(Resource, Default)]
pub struct PrePlayScene {
    snapshot: Option<SceneJsnAst>,
}

/// Marker for the toolbar transport buttons. `PiePlugin` installs
/// an `On<Add, PieButton>` observer that wires each button's
/// `Pointer<Click>` to the corresponding handler.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub enum PieButton {
    Play,
    Pause,
    Stop,
}

/// Marker added to any entity spawned while the editor is in
/// [`PlayState::Playing`]. The hierarchy tints these rows a
/// distinct colour so it's visually obvious which entities are
/// game-owned (and therefore will disappear on Stop) versus
/// authored.
///
/// Tagged automatically via the `On<Add, Transform>` observer in
/// `tag_game_spawned`. Entities that spawn without a `Transform`
/// aren't tagged; in practice this covers the 99% of game-spawned
/// entities that have one (meshes, lights, cameras, sprites, UI).
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct GameSpawned;

pub struct PiePlugin;

impl Plugin for PiePlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<PlayState>()
            .init_resource::<PrePlayScene>()
            .add_observer(wire_pie_button)
            .add_observer(tag_game_spawned);
    }
}

/// Observer: tag entities that receive a `Transform` while
/// `PlayState::Playing` is active with [`GameSpawned`]. Fires once
/// per entity because `On<Add, Transform>` is a one-shot event.
fn tag_game_spawned(
    trigger: On<Add, Transform>,
    state: Res<State<PlayState>>,
    already_tagged: Query<(), With<GameSpawned>>,
    mut commands: Commands,
) {
    if *state.get() != PlayState::Playing {
        return;
    }
    let entity = trigger.event_target();
    if already_tagged.get(entity).is_ok() {
        return;
    }
    commands.entity(entity).insert(GameSpawned);
}

/// Spawn a click observer on each `PieButton` as it's added.
///
/// The observer captures the button kind by value so there's no
/// need for a per-variant query at click time.
fn wire_pie_button(
    trigger: On<Add, PieButton>,
    buttons: Query<&PieButton>,
    mut commands: Commands,
) {
    let entity = trigger.event_target();
    let Ok(kind) = buttons.get(entity).copied() else {
        return;
    };
    commands.entity(entity).observe(
        move |_: On<Pointer<Click>>, mut commands: Commands| match kind {
            PieButton::Play => commands.queue(handle_play),
            PieButton::Pause => commands.queue(handle_pause),
            PieButton::Stop => commands.queue(handle_stop),
        },
    );
}

/// Transition into `Playing`. If currently `Stopped`, snapshot the
/// scene first so Stop has something to restore. No-op if already
/// `Playing`.
pub fn handle_play(world: &mut World) {
    let current = world.resource::<State<PlayState>>().get().clone();
    match current {
        PlayState::Stopped => {
            let snapshot = world.resource::<SceneJsnAst>().clone();
            world.resource_mut::<PrePlayScene>().snapshot = Some(snapshot);
            world
                .resource_mut::<NextState<PlayState>>()
                .set(PlayState::Playing);
            info!("PIE: Play (fresh start, scene snapshot captured)");
        }
        PlayState::Paused => {
            world
                .resource_mut::<NextState<PlayState>>()
                .set(PlayState::Playing);
            info!("PIE: Play (resumed)");
        }
        PlayState::Playing => {}
    }
}

/// Transition `Playing` → `Paused`. No-op otherwise.
pub fn handle_pause(world: &mut World) {
    if *world.resource::<State<PlayState>>().get() == PlayState::Playing {
        world
            .resource_mut::<NextState<PlayState>>()
            .set(PlayState::Paused);
        info!("PIE: Pause");
    }
}

/// Transition to `Stopped`, restoring the pre-Play scene snapshot.
/// The snapshot restore uses [`crate::scene_io::apply_ast_to_world`],
/// which despawns non-editor scene entities (including any spawned
/// by game systems) and respawns from the AST.
pub fn handle_stop(world: &mut World) {
    let current = world.resource::<State<PlayState>>().get().clone();
    if current == PlayState::Stopped {
        return;
    }

    if let Some(snapshot) = world.resource_mut::<PrePlayScene>().snapshot.take() {
        crate::scene_io::apply_ast_to_world(world, &snapshot);
        info!("PIE: Stop (scene restored from snapshot)");
    } else {
        info!("PIE: Stop (no snapshot to restore)");
    }

    world
        .resource_mut::<NextState<PlayState>>()
        .set(PlayState::Stopped);
}
