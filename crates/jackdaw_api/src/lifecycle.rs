//! Entity-based lifecycle primitives for extensions.
//!
//! An extension is represented as an [`Entity`] carrying an [`Extension`]
//! component. Everything it registers (operators, BEI context entities,
//! dock windows, workspaces) is spawned as a child of that entity.
//! Unloading is `world.entity_mut(ext).despawn()`; Bevy cascades through
//! the children. A small set of observers in `ExtensionLoaderPlugin`
//! handles cleanup that can't be expressed purely as entity despawn:
//! unregistering stored `SystemId`s, removing entries from the dock
//! `WindowRegistry`, and so on.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::ecs::system::SystemId;
use bevy::prelude::*;

use crate::operator::OperatorResult;

/// Root component for an extension.
///
/// Despawning this entity tears down all of the extension's child entities:
/// operators, BEI context/action entities, registered windows/workspaces, and
/// observer entities. Non-ECS cleanup (unregistering `SystemId`s, removing
/// entries from `WindowRegistry`) is handled by observers reacting to the
/// child-entity despawns.
#[derive(Component, Debug)]
pub struct Extension {
    pub name: String,
}

/// Child of an [`Extension`]; represents a single operator.
///
/// Holds the `SystemId`s that the dispatcher runs. An observer on
/// `On<Remove, OperatorEntity>` unregisters those systems when this entity
/// despawns, and keeps the [`OperatorIndex`] in sync.
#[derive(Component, Clone)]
pub struct OperatorEntity {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub execute: SystemId<(), OperatorResult>,
    pub invoke: SystemId<(), OperatorResult>,
    pub poll: Option<SystemId<(), bool>>,
    /// Mirrors [`crate::Operator::MODAL`]. Set at registration so the
    /// dispatcher can enter modal mode without re-resolving the generic
    /// operator type.
    pub modal: bool,
}

/// Tracks the currently-active modal operator. Exactly zero or one is
/// active at any time; starting a second modal while one is running is
/// refused.
///
/// While set, `tick_modal_operator` re-runs the invoke system every frame
/// and the [`crate::OperatorCommandBuffer`] stays prepared across frames
/// so every `record` call lands in the same `CommandGroup`.
#[derive(Resource, Default)]
pub struct ActiveModalOperator {
    pub(crate) id: Option<&'static str>,
    pub(crate) operator_entity: Option<Entity>,
    pub(crate) invoke_system: Option<SystemId<(), OperatorResult>>,
    pub(crate) label: Option<String>,
}

impl ActiveModalOperator {
    pub fn is_active(&self) -> bool {
        self.id.is_some()
    }

    pub fn id(&self) -> Option<&'static str> {
        self.id
    }
}

/// Marks an entity as tracking a dock window registration.
///
/// Spawned as a child of the [`Extension`] entity when `register_window` is
/// called. An observer on `On<Remove, RegisteredWindow>` calls
/// `WindowRegistry::unregister(id)` so the window disappears from the
/// add-window popup when the extension unloads.
#[derive(Component, Clone, Debug)]
pub struct RegisteredWindow {
    pub id: String,
}

/// Marks an entity as tracking a workspace registration.
#[derive(Component, Clone, Debug)]
pub struct RegisteredWorkspace {
    pub id: String,
}

/// Marks an entity as tracking a panel-extension registration (a section
/// injected into an existing panel via `ExtensionContext::extend_window`).
#[derive(Component, Clone, Debug)]
pub struct RegisteredPanelExtension {
    pub panel_id: String,
    pub section_index: usize,
}

/// An extension-contributed entry in the editor menu bar.
///
/// Spawned as a child of the [`Extension`] entity via
/// [`crate::ExtensionContext::register_menu_entry`]. The editor's
/// `populate_menu` system queries these and inserts them into the right
/// menu. Clicking one dispatches the referenced operator.
///
/// `menu` is the top-level menu name (`"Add"`, `"Tools"`, etc.). The
/// menu system is flat today; using a path-like string here leaves room
/// for nested menus later without breaking callers.
#[derive(Component, Clone, Debug)]
pub struct RegisteredMenuEntry {
    pub menu: String,
    pub label: String,
    pub operator_id: &'static str,
}

/// Reactive index from operator id → operator entity. Maintained by the
/// `index_operator_on_add` / `deindex_operator_on_remove` observers.
/// Lets the dispatcher resolve an id to a `SystemId` in O(1).
#[derive(Resource, Default)]
pub struct OperatorIndex {
    pub(crate) by_id: HashMap<&'static str, Entity>,
}

impl OperatorIndex {
    pub fn get(&self, id: &str) -> Option<Entity> {
        self.by_id.get(id).copied()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&'static str, Entity)> + '_ {
        self.by_id.iter().map(|(k, v)| (*k, *v))
    }
}

/// Constructor function for an extension. Stored in [`ExtensionCatalog`].
pub type ExtensionCtor = Arc<dyn Fn() -> Box<dyn crate::JackdawExtension> + Send + Sync>;

/// Registry of all extensions compiled into this build of Jackdaw.
///
/// Populated once during startup by calling `ExtensionCatalog::register` for
/// each built-in extension. External extensions (if/when dylib loading lands)
/// would register themselves here too. Toggle UIs read the catalog to list
/// available extensions.
#[derive(Resource, Default)]
pub struct ExtensionCatalog {
    entries: HashMap<String, CatalogEntry>,
}

struct CatalogEntry {
    ctor: ExtensionCtor,
    kind: ExtensionKind,
}

/// Classifies an entry in the catalog. Surfaced in toggle UIs so
/// Jackdaw-shipped feature areas and third-party extensions can be
/// presented separately. Extensions declare their own kind via
/// [`crate::JackdawExtension::kind`]; registration captures it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtensionKind {
    /// Ships with Jackdaw as a core feature area (scene tree, inspector,
    /// asset browser, etc.). Present in every build.
    Builtin,
    /// Everything else: example extensions bundled for demonstration,
    /// third-party extensions loaded from disk, user-authored addons.
    Custom,
}

impl ExtensionCatalog {
    /// Register a constructor with its declared kind. Most callers
    /// should use [`register_extension`] instead, which handles BEI
    /// context registration.
    pub fn register<F>(&mut self, name: impl Into<String>, kind: ExtensionKind, ctor: F)
    where
        F: Fn() -> Box<dyn crate::JackdawExtension> + Send + Sync + 'static,
    {
        self.entries.insert(
            name.into(),
            CatalogEntry {
                ctor: Arc::new(ctor),
                kind,
            },
        );
    }

    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(|s| s.as_str())
    }

    /// Iterate names with their declared [`ExtensionKind`]. Useful for
    /// grouping the Extensions dialog into Built-in and Custom sections.
    pub fn iter_with_kind(&self) -> impl Iterator<Item = (&str, ExtensionKind)> {
        self.entries
            .iter()
            .map(|(name, entry)| (name.as_str(), entry.kind))
    }

    /// Look up the declared [`ExtensionKind`] for a registered name.
    pub fn kind(&self, name: &str) -> Option<ExtensionKind> {
        self.entries.get(name).map(|e| e.kind)
    }

    /// Whether the named extension is a Jackdaw-shipped built-in.
    /// Returns `false` for unknown names.
    pub fn is_builtin(&self, name: &str) -> bool {
        self.kind(name) == Some(ExtensionKind::Builtin)
    }

    /// Construct a fresh instance of the named extension, if registered.
    pub fn construct(&self, name: &str) -> Option<Box<dyn crate::JackdawExtension>> {
        self.entries.get(name).map(|e| (e.ctor)())
    }
}

/// Register an extension into the catalog and perform its one-time BEI
/// input-context registration.
///
/// Call this once per extension during app setup. Registering the constructor
/// lets the Plugins dialog list the extension; running
/// `register_input_contexts` ensures its BEI context types are known to the
/// framework. Enabling and disabling the extension later only re-runs
/// `register()`, never `register_input_contexts()` (BEI panics on duplicate
/// registrations).
pub fn register_extension<F>(app: &mut App, name: &str, ctor: F)
where
    F: Fn() -> Box<dyn crate::JackdawExtension> + Send + Sync + 'static,
{
    // Construct a throwaway instance to (a) register context types and
    // (b) read the extension's declared `kind`. Doing both against the
    // same instance avoids a second construction just to classify.
    let sample = ctor();
    sample.register_input_contexts(app);
    let kind = sample.kind();
    drop(sample);

    app.world_mut()
        .resource_mut::<ExtensionCatalog>()
        .register(name, kind, ctor);
}

use crate::operator::OperatorCommandBuffer;
use jackdaw_commands::{CommandGroup, CommandHistory};

/// Dispatch an operator by id. Used by the BEI trigger observers spawned
/// in `ExtensionContext::register_operator`, and callable directly for
/// `Trigger::Manual` operators (UI buttons, F3 search, etc.).
///
/// - Non-modal operators: runs once and pushes a history entry immediately.
/// - Modal operators: if the invoke returns `Running`, enters modal mode.
///   The tick system takes over from the next frame.
///
/// If another modal operator is already active, the dispatch is refused
/// with a warn log (matches Blender's "one modal at a time" rule).
pub fn dispatch_operator_by_id(world: &mut World, id: &str, creates_history_entry: bool) {
    // Refuse if another modal operator is active.
    if let Some(active_id) = world.resource::<ActiveModalOperator>().id {
        warn!("Ignoring operator '{id}': modal operator '{active_id}' is currently active");
        return;
    }

    // Resolve operator via the reactive index. The index keys are
    // `&'static str` from each operator's trait, but `HashMap` lookup
    // hashes by string content, so a plain `&str` works.
    let Some(op_entity) = world.resource::<OperatorIndex>().by_id.get(id).copied() else {
        warn!("Tried to dispatch unknown operator: {}", id);
        return;
    };
    let Some(op) = world.get::<OperatorEntity>(op_entity).cloned() else {
        return;
    };

    // Poll (optional).
    if let Some(poll) = op.poll {
        if !world.run_system(poll).unwrap_or(false) {
            return;
        }
    }

    // Prep the command buffer, run the invoke system.
    world
        .resource_mut::<OperatorCommandBuffer>()
        .prepare(creates_history_entry);

    let result = match world.run_system(op.invoke) {
        Ok(r) => r,
        Err(err) => {
            error!("Failed to run operator {}: {:?}", op.id, err);
            world.resource_mut::<OperatorCommandBuffer>().take();
            return;
        }
    };

    match result {
        OperatorResult::Running if op.modal => {
            // Enter modal mode. The tick system picks up from next frame;
            // the command buffer stays prepared across frames.
            let mut active = world.resource_mut::<ActiveModalOperator>();
            active.id = Some(op.id);
            active.operator_entity = Some(op_entity);
            active.invoke_system = Some(op.invoke);
            active.label = Some(op.label.to_string());
        }
        OperatorResult::Running | OperatorResult::Finished => {
            // Non-modal `Running` collapses to `Finished` for one-shot behavior.
            finalize_operator_session(world, op.label, true);
        }
        OperatorResult::Cancelled => {
            finalize_operator_session(world, op.label, false);
        }
    }
}

/// Drain the command buffer and, if committing and history-tracked, push a
/// `CommandGroup` to `CommandHistory`.
fn finalize_operator_session(world: &mut World, label: &str, commit: bool) {
    let (recorded, creates_history) = world.resource_mut::<OperatorCommandBuffer>().take();
    if !commit {
        return;
    }
    if creates_history && !recorded.is_empty() {
        let group = Box::new(CommandGroup {
            commands: recorded,
            label: label.to_string(),
        });
        world.resource_mut::<CommandHistory>().push_executed(group);
    }
}

/// Tick system added to Update by `ExtensionLoaderPlugin`. If a modal
/// operator is active, re-runs its invoke system once per frame and
/// transitions out of modal on `Finished` or `Cancelled`.
pub fn tick_modal_operator(world: &mut World) {
    let Some(invoke) = world.resource::<ActiveModalOperator>().invoke_system else {
        return;
    };
    let result = match world.run_system(invoke) {
        Ok(r) => r,
        Err(err) => {
            error!(
                "Modal operator's invoke system failed: {:?}; cancelling",
                err
            );
            finalize_modal(world, false);
            return;
        }
    };
    match result {
        OperatorResult::Running => { /* stay modal */ }
        OperatorResult::Finished => finalize_modal(world, true),
        OperatorResult::Cancelled => finalize_modal(world, false),
    }
}

/// Exit modal mode. Drains the command buffer; commits as a history entry
/// if `commit` is true and the buffer is non-empty, otherwise discards.
fn finalize_modal(world: &mut World, commit: bool) {
    let label = {
        let mut active = world.resource_mut::<ActiveModalOperator>();
        let label = active.label.take().unwrap_or_default();
        active.id = None;
        active.operator_entity = None;
        active.invoke_system = None;
        label
    };
    finalize_operator_session(world, &label, commit);
}

/// Unload an extension. Despawns the root entity; the cascade and
/// cleanup observers handle the rest.
pub fn unload_extension(world: &mut World, ext_entity: Entity) {
    let ext_name = world
        .get::<Extension>(ext_entity)
        .map(|e| e.name.clone())
        .unwrap_or_default();
    info!("Unloading extension: {}", ext_name);

    if let Some(stored) = world
        .entity_mut(ext_entity)
        .take::<crate::StoredExtension>()
    {
        stored.0.unregister(world, ext_entity);
    }
    if let Ok(ec) = world.get_entity_mut(ext_entity) {
        ec.despawn();
    }
}

/// Enable a named extension via the catalog. Returns the new extension
/// entity, or `None` if the name is unknown or already loaded.
pub fn enable_extension(world: &mut World, name: &str) -> Option<Entity> {
    {
        let mut query = world.query::<&Extension>();
        if query.iter(world).any(|e| e.name == name) {
            return None;
        }
    }

    let extension = world.resource::<ExtensionCatalog>().construct(name)?;
    Some(crate::load_static_extension(world, extension))
}

/// Disable a named extension by despawning its root entity.
pub fn disable_extension(world: &mut World, name: &str) -> bool {
    let mut query = world.query::<(Entity, &Extension)>();
    let Some(ext_entity) = query
        .iter(world)
        .find(|(_, e)| e.name == name)
        .map(|(e, _)| e)
    else {
        return false;
    };
    unload_extension(world, ext_entity);
    true
}

/// Keep [`OperatorIndex`] in sync when an operator entity is spawned.
pub fn index_operator_on_add(
    trigger: On<Add, OperatorEntity>,
    operators: Query<&OperatorEntity>,
    mut index: ResMut<OperatorIndex>,
) {
    if let Ok(op) = operators.get(trigger.event_target()) {
        index.by_id.insert(op.id, trigger.event_target());
    }
}

/// Keep [`OperatorIndex`] in sync and free the operator's `SystemId`s
/// when its entity is removed, so they don't leak across enable /
/// disable cycles.
pub fn deindex_and_cleanup_operator_on_remove(
    trigger: On<Remove, OperatorEntity>,
    operators: Query<&OperatorEntity>,
    mut index: ResMut<OperatorIndex>,
    mut commands: Commands,
) {
    let Ok(op) = operators.get(trigger.event_target()) else {
        return;
    };
    info!("Unregistering operator: {}", op.id);
    index.by_id.remove(op.id);
    let (exec, inv, poll) = (op.execute, op.invoke, op.poll);
    commands.queue(move |world: &mut World| {
        let _ = world.unregister_system(exec);
        if exec != inv {
            let _ = world.unregister_system(inv);
        }
        if let Some(p) = poll {
            let _ = world.unregister_system(p);
        }
    });
}

/// Unregister a dock window from [`jackdaw_panels::WindowRegistry`] and
/// purge it from the live dock tree and every stored workspace tree when
/// its marker entity despawns, so disabling an extension visibly removes
/// its windows.
pub fn cleanup_window_on_remove(
    trigger: On<Remove, RegisteredWindow>,
    windows: Query<&RegisteredWindow>,
    mut registry: ResMut<jackdaw_panels::WindowRegistry>,
    mut dock_tree: ResMut<jackdaw_panels::tree::DockTree>,
    mut workspaces: ResMut<jackdaw_panels::WorkspaceRegistry>,
) {
    let Ok(w) = windows.get(trigger.event_target()) else {
        return;
    };
    info!("Unregistering window: {}", w.id);
    registry.unregister(&w.id);
    dock_tree.remove_window(&w.id);
    for workspace in workspaces.workspaces.iter_mut() {
        workspace.tree.remove_window(&w.id);
    }
}

/// Unregister a workspace when its marker entity despawns.
pub fn cleanup_workspace_on_remove(
    trigger: On<Remove, RegisteredWorkspace>,
    workspaces: Query<&RegisteredWorkspace>,
    mut registry: ResMut<jackdaw_panels::WorkspaceRegistry>,
) {
    if let Ok(w) = workspaces.get(trigger.event_target()) {
        registry.unregister(&w.id);
    }
}

/// Remove a panel extension section from the registry when its marker
/// entity despawns.
pub fn cleanup_panel_extension_on_remove(
    trigger: On<Remove, RegisteredPanelExtension>,
    registrations: Query<&RegisteredPanelExtension>,
    mut registry: ResMut<crate::PanelExtensionRegistry>,
) {
    if let Ok(r) = registrations.get(trigger.event_target()) {
        registry.remove(&r.panel_id, r.section_index);
    }
}

/// Log menu-entry registrations. Actual menu rebuilds are driven by
/// the main crate's `MenuBarDirty` flag because this crate doesn't know
/// the menu-bar implementation.
pub fn log_menu_entry_on_add(
    trigger: On<Add, RegisteredMenuEntry>,
    entries: Query<&RegisteredMenuEntry>,
) {
    if let Ok(entry) = entries.get(trigger.event_target()) {
        info!(
            "Registered menu entry: {} > {} -> {}",
            entry.menu, entry.label, entry.operator_id
        );
    }
}
