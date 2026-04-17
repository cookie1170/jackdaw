use bevy::ecs::system::SystemId;
use bevy::prelude::*;
use bevy_enhanced_input::prelude::InputAction;
use jackdaw_commands::EditorCommand;

/// A Blender-style operator.
///
/// The trait is bounded on [`InputAction`] so the operator type itself can be
/// used as a BEI action:
///
/// ```ignore
/// use bevy_enhanced_input::prelude::*;
///
/// #[derive(Default, InputAction)]
/// #[action_output(bool)]
/// struct PlaceCube;
///
/// impl Operator for PlaceCube {
///     const ID: &'static str = "sample.place_cube";
///     const LABEL: &'static str = "Place Cube";
///
///     fn register_execute(commands: &mut Commands) -> SystemId<(), OperatorResult> {
///         commands.register_system(place_cube_system)
///     }
/// }
/// ```
///
/// Extensions then bind the operator to a key via pure BEI syntax:
///
/// ```ignore
/// ctx.spawn((
///     MyPluginContext,
///     actions!(MyPluginContext[
///         Action::<PlaceCube>::new(),
///         bindings![KeyCode::C],
///     ]),
/// ));
/// ```
pub trait Operator: InputAction + 'static {
    const ID: &'static str;
    const LABEL: &'static str;
    const DESCRIPTION: &'static str = "";

    /// Which BEI event triggers the operator's invoke system.
    ///
    /// - `Trigger::Start` (default): fires on key down. Discrete,
    ///   one-shot semantics matching Blender's common case.
    /// - `Trigger::Fire`: fires every frame the input is held. Usually
    ///   paired with `MODAL = true` for state-tracking sessions; also
    ///   useful for per-frame probes.
    /// - `Trigger::Complete`: fires on key up. Useful for "press to arm,
    ///   release to commit" flows that don't need a modal state machine.
    /// - `Trigger::Manual`: no BEI observer is wired up. Callers invoke
    ///   the operator directly through
    ///   [`crate::lifecycle::dispatch_operator_by_id`], for example from
    ///   UI buttons, F3 search, or other code paths.
    const TRIGGER: Trigger = Trigger::Start;

    /// Modal operators stay active across frames.
    ///
    /// When `MODAL = true` and the invoke system returns
    /// [`OperatorResult::Running`], the dispatcher keeps the
    /// [`OperatorCommandBuffer`] prepared and re-runs the invoke system
    /// every frame until it returns `Finished` or `Cancelled`. Every
    /// `record` call across those frames lands in the same `CommandGroup`,
    /// so the entire modal session commits as one undo entry.
    ///
    /// When `MODAL = false` (default), `Running` is treated like
    /// `Finished`: the operator runs once, the buffer is drained, and a
    /// history entry is pushed immediately.
    const MODAL: bool = false;

    /// Register the primary execute system. Called once during
    /// `ExtensionContext::register_operator::<Self>()`. The returned
    /// `SystemId` is stored on the operator entity and unregistered on
    /// despawn.
    fn register_execute(commands: &mut Commands) -> SystemId<(), OperatorResult>;

    /// Register an optional poll system. Returns `true` if the operator is
    /// currently callable; `false` skips execution. Default: always callable.
    fn register_poll(_commands: &mut Commands) -> Option<SystemId<(), bool>> {
        None
    }

    /// Register an optional invoke system. `invoke` is what UI,
    /// keybinds, and F3 search run; it can differ from `execute` when
    /// the caller wants to open a dialog or start a drag before the
    /// primary work happens. Defaults to `execute`.
    fn register_invoke(commands: &mut Commands) -> SystemId<(), OperatorResult> {
        Self::register_execute(commands)
    }
}

/// Which BEI event an operator reacts to. See [`Operator::TRIGGER`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// Key/button down (`Start<A>`). One-shot, discrete.
    Start,
    /// Every frame while held (`Fire<A>`). Usually paired with `MODAL = true`.
    Fire,
    /// Key/button up (`Complete<A>`).
    Complete,
    /// No auto-wired observer. Caller dispatches explicitly.
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatorResult {
    /// Operator finished successfully. Any recorded commands are grouped and
    /// pushed to `CommandHistory` as a single undo entry.
    Finished,
    /// Operator explicitly cancelled. Recorded commands are dropped.
    Cancelled,
    /// Operator is in a modal state (drag, dialog). The dispatcher re-runs
    /// the invoke system next frame until `Finished` or `Cancelled`.
    /// Modal support is future work; first pass treats `Running` like
    /// `Finished` to simplify the dispatcher.
    Running,
}

/// Resource operator systems use to record `EditorCommand`s for undo.
///
/// Operators call [`Self::record`] from their execute/invoke system.
/// The dispatcher handles preparing and draining the buffer around each
/// invocation. The command's `execute` must already have been run by
/// the caller (or be implicit in how the system modified state).
///
/// All scene mutations go through an `EditorCommand`. Operators never
/// mutate `SceneJsnAst` directly.
#[derive(Resource, Default)]
pub struct OperatorCommandBuffer {
    pub(crate) recorded: Vec<Box<dyn EditorCommand>>,
    pub(crate) creates_history_entry: bool,
}

impl OperatorCommandBuffer {
    /// Record an already-executed command for undo. Use this when your
    /// operator system constructs commands that have already been applied
    /// to the world (e.g. by using `cmd.execute(world)` before calling
    /// record, or by doing the mutation directly and then recording a
    /// command that can reverse it on undo).
    pub fn record(&mut self, cmd: Box<dyn EditorCommand>) {
        self.recorded.push(cmd);
    }

    /// Execute a command and record it. Convenience for operators that
    /// have `&mut World` (exclusive systems).
    pub fn execute_and_record(&mut self, mut cmd: Box<dyn EditorCommand>, world: &mut World) {
        cmd.execute(world);
        self.recorded.push(cmd);
    }

    /// Called by the dispatcher before running the operator's invoke system.
    pub(crate) fn prepare(&mut self, creates_history_entry: bool) {
        self.recorded.clear();
        self.creates_history_entry = creates_history_entry;
    }

    /// Called by the dispatcher after the operator finishes. Returns the
    /// recorded commands and whether they should be turned into a history
    /// entry.
    pub(crate) fn take(&mut self) -> (Vec<Box<dyn EditorCommand>>, bool) {
        let recorded = std::mem::take(&mut self.recorded);
        let creates_history = self.creates_history_entry;
        self.creates_history_entry = false;
        (recorded, creates_history)
    }

    /// Whether the current operator run will create a history entry. Useful
    /// if an operator's execute system wants to behave differently when
    /// called from a nested context (e.g. skip dialog prompts).
    pub fn creates_history_entry(&self) -> bool {
        self.creates_history_entry
    }
}
