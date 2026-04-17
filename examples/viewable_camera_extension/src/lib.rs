//! Viewable Camera extension.
//!
//! - **F6**: place a viewable camera at the editor camera position.
//!   Undoable via a custom `EditorCommand`.
//! - **F7**: toggle between the editor view and looking through the
//!   viewable camera. Preview is view state, not scene data, and is
//!   not recorded in the undo history.

use bevy::camera::RenderTarget;
use bevy::ecs::system::SystemId;
use bevy::prelude::*;
use bevy_enhanced_input::prelude::*;
use jackdaw_api::prelude::*;
use jackdaw_commands::EditorCommand;

pub struct ViewableCameraExtension;

impl JackdawExtension for ViewableCameraExtension {
    fn name(&self) -> &str {
        "viewable_camera"
    }

    fn register_input_contexts(&self, app: &mut App) {
        app.add_input_context::<ViewableCameraContext>();
    }

    fn register(&self, ctx: &mut ExtensionContext) {
        ctx.world().init_resource::<CameraPreviewState>();

        ctx.register_operator::<PlaceViewableCamera>();
        ctx.register_operator::<ToggleCameraPreview>();

        ctx.register_menu_entry(MenuEntryDescriptor {
            menu: "Add".into(),
            label: "Viewable Camera".into(),
            operator_id: PlaceViewableCamera::ID,
        });

        ctx.spawn((
            ViewableCameraContext,
            actions!(ViewableCameraContext[
                (Action::<PlaceViewableCamera>::new(), bindings![KeyCode::F6]),
                (Action::<ToggleCameraPreview>::new(), bindings![KeyCode::F7]),
            ]),
        ));

        // Exit preview if the previewed camera gets despawned (e.g. the
        // user undoes the placement while preview is active), so the
        // viewport falls back to the editor camera.
        let ext_entity = ctx.entity();
        let observer = Observer::new(
            move |trigger: On<Remove, ViewableCamera>,
                  mut state: ResMut<CameraPreviewState>,
                  mut commands: Commands| {
                if state.active == Some(trigger.event_target()) {
                    state.active = None;
                    commands.queue(|world: &mut World| {
                        restore_editor_camera(world);
                    });
                }
            },
        );
        ctx.world().spawn((observer, ChildOf(ext_entity)));
    }
}

/// BEI context for this extension; gives key-binding isolation.
#[derive(Component, Default)]
pub struct ViewableCameraContext;

/// Marker on camera entities created by this extension. Scene data,
/// not editor-local.
#[derive(Component, Default, Reflect)]
pub struct ViewableCamera;

/// Tracks the currently-previewed camera. Not saved to the scene.
#[derive(Resource, Default)]
struct CameraPreviewState {
    active: Option<Entity>,
    /// `(editor_camera, its_render_target)` captured on enter so it can
    /// be restored on exit.
    saved: Option<(Entity, RenderTarget)>,
}

/// Place a viewable camera at the editor camera's current position.
/// Undoable.
#[derive(Default, InputAction)]
#[action_output(bool)]
pub struct PlaceViewableCamera;

impl Operator for PlaceViewableCamera {
    const ID: &'static str = "viewable_camera.place";
    const LABEL: &'static str = "Place Viewable Camera";
    const DESCRIPTION: &'static str = "Place a camera at the viewport position";

    fn register_execute(commands: &mut Commands) -> SystemId<(), OperatorResult> {
        commands.register_system(place_viewable_camera)
    }
}

fn place_viewable_camera(world: &mut World) -> OperatorResult {
    // Match the editor camera's transform so "look through" feels
    // natural on the first toggle.
    let spawn_transform = find_editor_camera(world)
        .and_then(|e| world.get::<Transform>(e).copied())
        .unwrap_or_default();

    let mut cmd: Box<dyn EditorCommand> = Box::new(PlaceViewableCameraCommand {
        spawned: None,
        transform: spawn_transform,
    });
    cmd.execute(world);
    world.resource_mut::<OperatorCommandBuffer>().record(cmd);
    OperatorResult::Finished
}

/// Toggle "look through the viewable camera" against the editor view.
/// Preview is view state, not a scene edit, so it isn't undoable.
#[derive(Default, InputAction)]
#[action_output(bool)]
pub struct ToggleCameraPreview;

impl Operator for ToggleCameraPreview {
    const ID: &'static str = "viewable_camera.toggle_preview";
    const LABEL: &'static str = "Toggle Camera Preview";
    const DESCRIPTION: &'static str = "Look through the selected viewable camera";

    fn register_execute(commands: &mut Commands) -> SystemId<(), OperatorResult> {
        commands.register_system(toggle_preview)
    }
}

fn toggle_preview(world: &mut World) -> OperatorResult {
    let currently_active = world.resource::<CameraPreviewState>().active;
    if currently_active.is_some() {
        restore_editor_camera(world);
        info!("Exited viewable-camera preview");
    } else {
        let Some(target) = pick_preview_target(world) else {
            warn!("No viewable camera to preview; press F6 to place one first");
            return OperatorResult::Cancelled;
        };
        match enter_preview(world, target) {
            Ok(()) => info!("Entered preview through viewable camera {target:?}"),
            Err(reason) => warn!("Preview failed: {reason}"),
        }
    }
    OperatorResult::Finished
}

/// Find the active editor-viewport camera: an active `Camera3d` with an
/// `Image` render target that isn't one of ours.
fn find_editor_camera(world: &mut World) -> Option<Entity> {
    let mut q = world.query_filtered::<
        (Entity, &Camera, &RenderTarget),
        (With<Camera3d>, Without<ViewableCamera>),
    >();
    for (entity, camera, target) in q.iter(world) {
        if camera.is_active && matches!(target, RenderTarget::Image(_)) {
            return Some(entity);
        }
    }
    None
}

/// Pick which viewable camera to preview: the only one if there is
/// exactly one, otherwise the first one found. A richer rule honouring
/// the editor's `Selection` resource would require depending on the
/// main jackdaw crate.
fn pick_preview_target(world: &mut World) -> Option<Entity> {
    let cams: Vec<Entity> = world
        .query_filtered::<Entity, With<ViewableCamera>>()
        .iter(world)
        .collect();
    if cams.len() == 1 {
        return Some(cams[0]);
    }
    cams.first().copied()
}

fn enter_preview(world: &mut World, target: Entity) -> Result<(), &'static str> {
    let Some(editor_cam) = find_editor_camera(world) else {
        return Err("couldn't find the editor viewport camera");
    };
    let render_target = world
        .get::<RenderTarget>(editor_cam)
        .cloned()
        .ok_or("editor camera has no RenderTarget")?;

    // Disable the editor camera.
    if let Some(mut c) = world.get_mut::<Camera>(editor_cam) {
        c.is_active = false;
    }

    // Hand its render target to the viewable camera and activate it.
    if let Ok(mut ec) = world.get_entity_mut(target) {
        ec.insert(render_target.clone());
    }
    if let Some(mut c) = world.get_mut::<Camera>(target) {
        c.is_active = true;
    }
    // Bevy's `camera_system` recomputes `target_info` only when
    // Projection is marked changed, viewport size differs, or the
    // underlying image/window changes. Replacing `RenderTarget` via
    // `insert` triggers none of those, so without this the camera
    // renders into the stale 1x1 target from spawn.
    if let Some(mut proj) = world.get_mut::<Projection>(target) {
        proj.set_changed();
    }

    let mut state = world.resource_mut::<CameraPreviewState>();
    state.active = Some(target);
    state.saved = Some((editor_cam, render_target));
    Ok(())
}

/// Hand the render target back to the editor camera. Safe to call when
/// preview is already off.
fn restore_editor_camera(world: &mut World) {
    let Some((editor_cam, _target)) = world.resource_mut::<CameraPreviewState>().saved.take()
    else {
        return;
    };
    let active = world.resource_mut::<CameraPreviewState>().active.take();

    if let Some(preview) = active {
        if let Ok(mut ec) = world.get_entity_mut(preview) {
            // `Camera` requires a `RenderTarget`, so swap to `None`
            // rather than removing the component outright. Leaving the
            // Image target on an inactive camera triggers ambiguity
            // warnings when preview is re-entered.
            ec.insert(RenderTarget::None {
                size: UVec2::splat(1),
            });
        }
        if let Some(mut c) = world.get_mut::<Camera>(preview) {
            c.is_active = false;
        }
        // Mirrors the Projection poke in `enter_preview`. See its
        // comment for why.
        if let Some(mut proj) = world.get_mut::<Projection>(preview) {
            proj.set_changed();
        }
    }

    if let Some(mut c) = world.get_mut::<Camera>(editor_cam) {
        c.is_active = true;
    }
}

/// Undoable placement of a viewable camera. Redo re-executes, which
/// spawns a new entity id; the scene tree and inspector resolve by id
/// each frame so that's fine.
struct PlaceViewableCameraCommand {
    spawned: Option<Entity>,
    transform: Transform,
}

impl EditorCommand for PlaceViewableCameraCommand {
    fn execute(&mut self, world: &mut World) {
        let entity = world
            .spawn((
                Name::new("Viewable Camera"),
                ViewableCamera,
                Camera3d::default(),
                Camera {
                    // Stays off until `enter_preview` hands over the
                    // viewport image target.
                    is_active: false,
                    order: -1,
                    ..default()
                },
                // `Camera`'s required components default `RenderTarget`
                // to the primary window, which would render over the
                // editor UI if `is_active` ever flipped true. Keep the
                // camera inert until `enter_preview` swaps in the
                // viewport image target.
                RenderTarget::None {
                    size: UVec2::splat(1),
                },
                self.transform,
                Visibility::default(),
            ))
            .id();
        self.spawned = Some(entity);
    }

    fn undo(&mut self, world: &mut World) {
        if let Some(entity) = self.spawned.take()
            && let Ok(ec) = world.get_entity_mut(entity)
        {
            ec.despawn();
        }
    }

    fn description(&self) -> &str {
        "Place Viewable Camera"
    }
}
