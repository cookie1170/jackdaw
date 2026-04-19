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
//! - [`PiePlugin`] — registers state, resource, and observers.
//!
//! Handlers [`handle_play`], [`handle_pause`], [`handle_stop`] are
//! exposed for direct `commands.queue(...)` use in case other
//! surfaces (keybinds, menu entries) want to trigger PIE
//! transitions without going through a button.

use bevy::prelude::*;
use jackdaw_api::PlayState;
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

pub struct PiePlugin;

impl Plugin for PiePlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<PlayState>()
            .init_resource::<PrePlayScene>()
            .add_observer(wire_pie_button);
    }
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
