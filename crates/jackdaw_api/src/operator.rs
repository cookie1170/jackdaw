use std::{borrow::Cow, collections::BTreeMap};

use bevy::ecs::system::{SystemId, SystemState};
use bevy::prelude::*;
use bevy_enhanced_input::prelude::InputAction;
use jackdaw_commands::{CommandHistory, EditorCommand};
use jackdaw_jsn::PropertyValue;

use crate::lifecycle::ActiveModalQuery;
use crate::{
    ActiveSnapshotter, SceneSnapshot,
    lifecycle::{ActiveModalOperator, OperatorEntity, OperatorIndex},
};

pub(super) fn plugin(app: &mut App) {
    app.add_systems(Update, tick_modal_operator);
}

/// A Blender-style operator.
///
/// The trait is bounded on [`InputAction`] so the operator type itself
/// can be used as a BEI action.
/// Usually you will want to use [`operator`](crate::prelude::operator) to define your operator, but it can be manually implemented if needed:
///
/// ```rust
/// use bevy_enhanced_input::prelude::*;
/// use jackdaw_api::prelude::*;
/// use bevy::prelude::*;
///
/// #[derive(Default, InputAction)]
/// #[action_output(bool)]
/// struct PlaceCubeOp;
///
/// impl Operator for PlaceCubeOp {
///     const ID: &'static str = "sample.place_cube";
///     const LABEL: &'static str = "Place Cube";
///
///     fn register_execute(commands: &mut Commands) -> SystemId<In<OperatorParameters>, OperatorResult> {
///         commands.register_system(place_cube)
///     }
/// }
///
/// fn place_cube(_: In<OperatorParameters>, mut commands: Commands) -> OperatorResult {
///     commands.spawn((Name::new("Cube"), Transform::default()));
///     OperatorResult::Finished
/// }
/// ```
///
/// Extensions then bind the operator to a key via pure BEI syntax. Use
/// BEI binding modifiers (`Press`, `Release`, `Hold`) when specific
/// input timing is needed. See the [jackdaw_api documentation](crate).
pub trait Operator: InputAction + 'static {
    const ID: &'static str;
    const LABEL: &'static str;
    const DESCRIPTION: &'static str = "";

    /// Whether an observer should be auto-wired to call this operator.
    ///
    /// When `false` (default), registration spawns a `Fire<Self>`
    /// observer that dispatches the operator whenever any bound input
    /// fires it. Authors shape timing via BEI binding modifiers
    /// (`Press`, `Release`, `Hold`, etc.) on the binding.
    ///
    /// When `true`, no observer is spawned. The operator is invocable
    /// only through `World::operator(Self::ID)`. Useful for
    /// operators driven by menus, UI buttons, or F3-search without
    /// a keybind.
    const MANUAL: bool = false;

    /// Modal operators stay active across frames.
    ///
    /// When `MODAL = true` and the invoke system returns
    /// [`OperatorResult::Running`], the dispatcher re-runs the invoke
    /// system every frame until it returns `Finished` or `Cancelled`.
    /// The scene snapshot captured at `Start` is diffed against the
    /// state at `Finished`, so the whole session commits as one undo
    /// entry.
    ///
    /// When `MODAL = false` (default), `Running` is treated like
    /// `Finished` and one invoke runs to completion.
    const MODAL: bool = false;

    /// Register the primary execute system. Called once during
    /// `ExtensionContext::register_operator::<Self>()`. The returned
    /// `SystemId` is stored on the operator entity and unregistered
    /// on despawn.
    fn register_execute(commands: &mut Commands) -> OperatorSystemId;

    /// Register an optional availability check. Returns `true` if the
    /// operator can run in the current editor state, `false` if it
    /// should be skipped. Default: always callable.
    #[expect(unused_variables, reason = "The default implementation noops")]
    fn register_availability_check(commands: &mut Commands) -> Option<SystemId<(), bool>> {
        None
    }

    /// Register an optional invoke system. `invoke` is what UI,
    /// keybinds, and F3 search run; it can differ from `execute`
    /// when the caller wants to open a dialog or start a drag before
    /// the primary work happens. Defaults to `execute`.
    fn register_invoke(commands: &mut Commands) -> OperatorSystemId {
        Self::register_execute(commands)
    }

    /// Register an optional cancel system. `invoke` is what UI,
    #[expect(unused_variables, reason = "The default implementation noops")]
    fn register_cancel(commands: &mut Commands) -> Option<SystemId<()>> {
        None
    }
}

#[derive(Debug, Clone, Default, Deref, DerefMut, Reflect)]
pub struct OperatorParameters(pub BTreeMap<String, PropertyValue>);

pub type OperatorSystemId = SystemId<In<OperatorParameters>, OperatorResult>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "Operators may not be `Finished`, which should usually be handled"]
pub enum OperatorResult {
    /// Operator finished successfully. The dispatcher captures the
    /// resulting scene diff as a single undo entry.
    Finished,
    /// Operator explicitly cancelled. No history entry is pushed.
    Cancelled,
    /// Operator is in a modal session (drag, dialog, multi-frame
    /// edit). The dispatcher re-runs the invoke system every frame
    /// until it returns `Finished` or `Cancelled`. Non-modal
    /// operators that return `Running` collapse to `Finished`.
    Running,
}

impl OperatorResult {
    /// Returns `true` if the operator finished successfully.
    pub fn is_finished(&self) -> bool {
        matches!(self, OperatorResult::Finished)
    }
}

/// Extension trait on [`World`] for calling operators by id.
///
/// Usage:
///
/// ```
/// use jackdaw_api::prelude::*;
/// use bevy::prelude::*;
///
/// fn my_button(world: &mut World) {
///     let result = world.operator("avian.add_rigid_body").call().unwrap();
///     if !result.is_finished() {
///        warn!("heck!");
///     }
/// }
/// ```
pub trait OperatorWorldExt {
    #[must_use = "Operators must be called with `.call()` to execute them"]
    fn operator<'a>(
        &'a mut self,
        id: impl Into<Cow<'static, str>>,
    ) -> OperatorCallBuilder<'a, World>;

    fn cancel_active_modal(&mut self) -> Result;
}

impl OperatorWorldExt for World {
    fn operator<'a>(
        &'a mut self,
        id: impl Into<Cow<'static, str>>,
    ) -> OperatorCallBuilder<'a, World> {
        OperatorCallBuilder {
            world_commands: self,
            id: id.into(),
            params: OperatorParameters::default(),
            settings: CallOperatorSettings::default(),
        }
    }

    fn cancel_active_modal(&mut self) -> Result {
        self.run_system_cached(cancel_active_modal)
            .map_err(From::from)
    }
}

pub trait OperatorCommandsExt<'w, 's> {
    #[must_use = "Operators must be called with `.call()` to execute them"]
    fn operator<'a>(
        &'a mut self,
        id: impl Into<Cow<'static, str>>,
    ) -> OperatorCallBuilder<'a, Commands<'w, 's>>;
}

impl<'w, 's> OperatorCommandsExt<'w, 's> for Commands<'w, 's> {
    fn operator<'a>(
        &'a mut self,
        id: impl Into<Cow<'static, str>>,
    ) -> OperatorCallBuilder<'a, Commands<'w, 's>> {
        OperatorCallBuilder {
            world_commands: self,
            id: id.into(),
            params: OperatorParameters::default(),
            settings: CallOperatorSettings::default(),
        }
    }
}

/// Knobs passed to [`OperatorCallBuilder::settings`].
#[derive(Clone, Debug, Copy)]
pub struct CallOperatorSettings {
    /// Whether a successful call should push an undo entry. Default
    /// `true`. Set `false` for view-local effects (camera moves,
    /// preview toggles) that should not be undoable.
    pub creates_history_entry: bool,
    pub execution_context: ExecutionContext,
}

impl Default for CallOperatorSettings {
    fn default() -> Self {
        Self {
            creates_history_entry: false,
            execution_context: default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub enum ExecutionContext {
    #[default]
    Execute,
    Invoke,
}

#[derive(Clone, Debug)]
pub enum CallOperatorError {
    UnknownId(Cow<'static, str>),
    ModalAlreadyActive(&'static str),
    NotAvailable,
    ExecuteFailed,
}

impl std::fmt::Display for CallOperatorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownId(id) => write!(f, "unknown operator: {id}"),
            Self::ModalAlreadyActive(id) => {
                write!(f, "modal operator '{id}' is currently active")
            }
            Self::NotAvailable => f.write_str("operator's availability check failed"),
            Self::ExecuteFailed => f.write_str("operator's execute system failed"),
        }
    }
}

impl std::error::Error for CallOperatorError {}

pub struct OperatorCallBuilder<'a, T> {
    // Either `World` or `Commands`
    world_commands: &'a mut T,
    id: Cow<'static, str>,
    params: OperatorParameters,
    settings: CallOperatorSettings,
}

impl<'a, T> OperatorCallBuilder<'a, T> {
    #[must_use = "Operators must be called with `.call()` to execute them"]
    pub fn param(
        mut self,
        key: impl Into<Cow<'static, str>>,
        value: impl Into<PropertyValue>,
    ) -> Self {
        self.params.insert(key.into().to_string(), value.into());
        self
    }

    #[must_use = "Operators must be called with `.call()` to execute them"]
    pub fn settings(mut self, settings: CallOperatorSettings) -> Self {
        self.settings = settings;
        self
    }
}

impl<'a> OperatorCallBuilder<'a, Commands<'_, '_>> {
    /// Call an operator by id. The availability check runs before the
    /// invoke system, so validation logic lives only on the operator
    /// itself.
    pub fn call(self) {
        self.world_commands.queue(move |world: &mut World| {
            world.run_system_cached_with(dispatch_operator, (self.id, self.params, self.settings))
        });
    }

    pub fn cancel(self) {
        self.world_commands.queue(|world: &mut World| {
            let res: Result = world
                .run_system_cached(cancel_active_modal)
                .map_err(BevyError::from);
            if let Err(err) = res {
                error!("Failed to cancel active modal: {err}")
            }
        });
    }
}

impl<'a> OperatorCallBuilder<'a, World> {
    /// Whether the operator would run in the current editor state.
    /// `Ok(true)` if it's ready, `Ok(false)` if not, `Err` for unknown
    /// ids.
    pub fn is_available(self) -> Result<bool, CallOperatorError> {
        let Some(op_entity) = self
            .world_commands
            .resource::<OperatorIndex>()
            .by_id
            .get(self.id.as_ref())
            .copied()
        else {
            return Err(CallOperatorError::UnknownId(self.id));
        };
        let Some(op) = self
            .world_commands
            .get::<OperatorEntity>(op_entity)
            .cloned()
        else {
            return Err(CallOperatorError::UnknownId(self.id));
        };
        if op.modal
            && self
                .world_commands
                .query_filtered::<Entity, With<ActiveModalOperator>>()
                .iter(self.world_commands)
                .next()
                .is_some()
        {
            return Err(CallOperatorError::ModalAlreadyActive(op.id));
        }
        let Some(check) = op.availability_check else {
            return Ok(true);
        };
        self.world_commands
            .run_system(check)
            .map_err(|_| CallOperatorError::NotAvailable)
    }

    /// Call an operator by id. The availability check runs before the
    /// invoke system, so validation logic lives only on the operator
    /// itself.
    pub fn call(self) -> Result<OperatorResult, CallOperatorError> {
        let result = self
            .world_commands
            .run_system_cached_with(dispatch_operator, (self.id, self.params, self.settings));
        match result {
            Ok(result) => result,
            Err(_) => Err(CallOperatorError::ExecuteFailed),
        }
    }

    /// Checks if an operator is the currently running modal operator.
    pub fn is_running(self) -> bool {
        let result = self
            .world_commands
            .run_system_cached_with(is_op_running, self.id.clone());
        match result {
            Ok(result) => result,
            Err(_) => {
                error!(
                    "Failed to check if operator is running: {}, treating as `false`",
                    self.id
                );
                false
            }
        }
    }

    /// Calls an operator's cancel system, if one is defined, and stops the execution if it is running as a modal.
    ///
    /// In general, calling this only makes sense with a modal operator.
    pub fn cancel(self) -> Result {
        self.world_commands
            .run_system_cached_with(cancel_operator, self.id)
            .map_err(From::from)
    }
}

fn is_op_running(
    In(id): In<Cow<'static, str>>,
    world: &mut World,
    active: &mut SystemState<ActiveModalQuery>,
) -> bool {
    active.get(world).is_operator(id)
}

fn dispatch_operator(
    In((id, params, settings)): In<(Cow<'static, str>, OperatorParameters, CallOperatorSettings)>,
    world: &mut World,
    active: &mut SystemState<ActiveModalQuery>,
) -> Result<OperatorResult, CallOperatorError> {
    let Some(op_entity) = world
        .resource::<OperatorIndex>()
        .by_id
        .get(id.as_ref())
        .copied()
    else {
        return Err(CallOperatorError::UnknownId(id));
    };
    let Some(op) = world.get::<OperatorEntity>(op_entity).cloned() else {
        return Err(CallOperatorError::UnknownId(id));
    };

    if op.modal
        && let Some(active_op) = active.get(world).get_operator()
    {
        return Err(CallOperatorError::ModalAlreadyActive(active_op.id));
    }

    if let Some(check) = op.availability_check {
        let available = world
            .run_system(check)
            .map_err(|_| CallOperatorError::NotAvailable)?;
        if !available {
            // FIXME: this should read
            // return Err(CallOperatorError::NotAvailable);
            // but this is much too chatty for the default error handler. Since we don't have severities on BevyError yet,
            // we need to manually move this to `debug` on the error handler.
            // Which I leave as an exercise for the reader.
            // Until then, this erroneously returns `Ok()` :)
            debug!("Availability check failed for operator: {id}");
            return Ok(OperatorResult::Cancelled);
        }
    }

    // Only the outermost operator in a nesting chain captures the
    // snapshot. Inner `operator` calls mutate inside the outer's
    // span and their changes roll into the outer's diff.
    let before_snapshot = settings
        .creates_history_entry
        .then(|| world.resource::<ActiveSnapshotter>().0.capture(world));

    let system = match settings.execution_context {
        ExecutionContext::Execute => op.execute,
        ExecutionContext::Invoke => op.invoke,
    };
    info!("OPERATOR: {id}");
    let result = world.run_system_with(system, params);

    let result = result.map_err(|_| CallOperatorError::ExecuteFailed)?;
    match result {
        OperatorResult::Running if op.modal => {
            world
                .entity_mut(op_entity)
                .insert(ActiveModalOperator { before_snapshot });
        }
        OperatorResult::Running => {}
        OperatorResult::Finished => {
            if let Err(err) =
                world.run_system_cached_with(save_history, (op.label, before_snapshot))
            {
                error!("Failed to finalize modal operator {}: {err:?}", op.label);
            }
        }
        OperatorResult::Cancelled => {
            let res: Result = world
                .run_system_cached_with(cancel_operator, op.id.into())
                .map_err(From::from);
            if let Err(err) = res {
                error!("Failed to finalize cancel operator: {err:?}");
            }
        }
    }

    Ok(result)
}

/// Capture the current state, diff against `before`, and push a
/// `SnapshotDiff` onto [`CommandHistory`] if the scene changed.
fn save_history(
    In((label, before)): In<(&'static str, Option<Box<dyn SceneSnapshot>>)>,
    world: &mut World,
) {
    let Some(before) = before else { return };
    let after = world.resource::<ActiveSnapshotter>().0.capture(world);
    if before.equals(&*after) {
        return;
    }
    world
        .resource_mut::<CommandHistory>()
        .push_executed(Box::new(SnapshotDiff {
            before,
            after,
            label: label.to_string(),
        }));
}

/// One undo entry. Swaps the active scene snapshot on execute / undo.
struct SnapshotDiff {
    before: Box<dyn SceneSnapshot>,
    after: Box<dyn SceneSnapshot>,
    label: String,
}

impl EditorCommand for SnapshotDiff {
    fn execute(&mut self, world: &mut World) {
        self.after.apply(world);
    }
    fn undo(&mut self, world: &mut World) {
        self.before.apply(world);
    }
    fn description(&self) -> &str {
        &self.label
    }
}
/// Tick system added to Update by `ExtensionLoaderPlugin`. Re-runs the
/// active modal operator's invoke system each frame; exits modal on
/// `Finished` (committing) or `Cancelled` (discarding).
pub(crate) fn tick_modal_operator(world: &mut World, active: &mut SystemState<ActiveModalQuery>) {
    let Some(op) = active.get(world).get_operator().cloned() else {
        return;
    };
    let result = match world.run_system_with(op.invoke, default()) {
        Ok(r) => r,
        Err(err) => {
            error!("Modal operator's invoke system failed: {err:?}; cancelling");
            if let Err(err) = world.run_system_cached_with(finalize_modal, false) {
                error!("Failed to finalize modal operator: {err:?}");
            }
            return;
        }
    };
    match result {
        OperatorResult::Running => {}
        OperatorResult::Finished => {
            if let Err(err) = world.run_system_cached_with(finalize_modal, true) {
                error!("Failed to finalize modal operator: {err:?}");
            }
        }
        OperatorResult::Cancelled => {
            // variable needed due to the type system being annoyed with us
            let res: Result = world
                .run_system_cached_with(cancel_operator, op.id.into())
                .map_err(BevyError::from);
            if let Err(err) = res {
                error!("Failed to finalize cancel operator: {err:?}");
            }
        }
    }
}

pub(crate) fn cancel_active_modal(
    world: &mut World,
    active: &mut SystemState<ActiveModalQuery>,
) -> Result {
    let Some(op) = active.get(world).get_operator().cloned() else {
        return Ok(());
    };
    world
        .run_system_cached_with(cancel_operator, op.id.into())
        .map_err(From::from)
}

pub(crate) fn cancel_operator(
    In(id): In<Cow<'static, str>>,
    world: &mut World,
    ops: &mut QueryState<&OperatorEntity>,
    active: &mut SystemState<ActiveModalQuery>,
) -> Result {
    let Some(op) = ops.iter(world).find(|o| o.id == id).cloned() else {
        warn!("Tried to cancel non-existent operator: {id}");
        return Ok(());
    };

    let mut cancel_err = None;
    if let Some(cancel) = op.cancel {
        if let Err(err) = world.run_system(cancel) {
            error!("Failed to cancel modal operator {}: {err:?}", op.label);
            cancel_err = Some(err);
        }
    }
    let mut finalize_err = None;
    if active.get(world).is_operator(id) {
        if let Err(err) = world.run_system_cached_with(finalize_modal, false) {
            error!("Failed to finalize modal operator: {err:?}");
            finalize_err = Some(err);
        }
    }
    match (cancel_err, finalize_err) {
        (Some(cancel_err), Some(_finalize_err)) => {
            // BevyError cannot accumulate errors, so we gotta pick one :/
            Err(cancel_err.into())
        }
        (Some(cancel_err), None) => Err(BevyError::from(cancel_err)),
        (None, Some(finalize_err)) => Err(BevyError::from(finalize_err)),
        (None, None) => Ok(()),
    }
}

/// Exit modal mode. Commits the before-snapshot diff as a history entry
/// if `commit`, otherwise discards it.
fn finalize_modal(
    In(commit): In<bool>,
    world: &mut World,
    active: &mut SystemState<Option<Single<(Entity, &OperatorEntity), With<ActiveModalOperator>>>>,
) {
    let Some((entity, op)) = active
        .get(world)
        .map(|t| t.into_inner())
        .map(|(e, o)| (e, o.clone()))
    else {
        return;
    };
    let Some(snapshot) = world.entity_mut(entity).take::<ActiveModalOperator>() else {
        return;
    };
    if !commit {
        return;
    }
    if let Err(err) =
        world.run_system_cached_with(save_history, (op.label, snapshot.before_snapshot))
    {
        error!("Failed to finalize modal operator {}: {err:?}", op.label);
    }
}
