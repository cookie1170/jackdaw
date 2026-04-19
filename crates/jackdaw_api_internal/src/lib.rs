//! Public API for Jackdaw editor extensions.
//!
//! Extensions are entities. An extension entity holds an [`Extension`]
//! component, and every registration (operators, windows, BEI contexts,
//! panel extensions) spawns child entities under it. Unloading an
//! extension is `world.entity_mut(ext).despawn()`; Bevy cascades through
//! the children and a few observers handle the non-ECS cleanup.
//!
//! Minimal extension:
//!
//! ```ignore
//! use bevy::prelude::*;
//! use bevy_enhanced_input::prelude::*;
//! use jackdaw_api::prelude::*;
//!
//! // An operator is also a BEI action, so one type covers both.
//! #[derive(Default, InputAction)]
//! #[action_output(bool)]
//! pub struct PlaceCube;
//!
//! impl Operator for PlaceCube {
//!     const ID: &'static str = "sample.place_cube";
//!     const LABEL: &'static str = "Place Cube";
//!     fn register_execute(commands: &mut Commands) -> SystemId<(), OperatorResult> {
//!         commands.register_system(place_cube)
//!     }
//! }
//!
//! // Operators are plain Bevy systems. Mutate the world however you
//! // like; the dispatcher snapshots the scene before invoke and diffs
//! // after, so a single Ctrl+Z reverses the entire call.
//! fn place_cube(mut commands: Commands) -> OperatorResult {
//!     commands.spawn((Name::new("Cube"), Transform::default()));
//!     OperatorResult::Finished
//! }
//!
//! #[derive(Component, Default)]
//! pub struct SamplePluginContext;
//!
//! pub struct SamplePlugin;
//!
//! impl JackdawExtension for SamplePlugin {
//!     fn name(&self) -> &str { "Sample Plugin" }
//!     fn register(&self, ctx: &mut ExtensionContext) {
//!         ctx.register_operator::<PlaceCube>();
//!         ctx.add_input_context::<SamplePluginContext>();
//!         ctx.spawn((
//!             SamplePluginContext,
//!             actions!(SamplePluginContext[
//!                 Action::<PlaceCube>::new(),
//!                 bindings![KeyCode::C],
//!             ]),
//!         ));
//!     }
//! }
//! ```

mod export;
pub mod ffi;
pub mod lifecycle;
mod operator;
pub mod pie;
mod registries;
pub mod snapshot;

use std::sync::Arc;

use bevy::ecs::world::EntityWorldMut;
use bevy::prelude::*;
use jackdaw_panels::{
    DockWindowDescriptor, WindowRegistry, WorkspaceDescriptor, WorkspaceRegistry,
};

pub use jackdaw_api_macros::operator;
pub use lifecycle::{
    ActiveModalOperator, CallOperatorError, CallOperatorSettings, Extension, ExtensionCatalog,
    ExtensionCtor, ExtensionKind, OperatorEntity, OperatorIndex, OperatorSession, OperatorWorldExt,
    RegisteredMenuEntry, RegisteredPanelExtension, RegisteredWindow, RegisteredWorkspace,
    disable_extension, enable_extension, register_extension, tick_modal_operator, unload_extension,
};
pub use operator::{Operator, OperatorResult};
pub use pie::PlayState;
pub use registries::PanelExtensionRegistry;
pub use snapshot::{ActiveSnapshotter, SceneSnapshot, SceneSnapshotter};

/// Re-exports plugin authors will want in one import.
pub mod prelude {
    pub use crate::lifecycle::{
        CallOperatorError, CallOperatorSettings, Extension, ExtensionCatalog, ExtensionKind,
        OperatorEntity, OperatorIndex, OperatorWorldExt,
    };
    pub use crate::operator::{Operator, OperatorResult};
    pub use crate::pie::PlayState;
    pub use crate::{
        ExtensionContext, ExtensionPoint, JackdawExtension, MenuEntryDescriptor, PanelContext,
        SectionBuildFn, WindowDescriptor, operator,
    };
    // BEI types extension authors need for `actions!` / `bindings!` / observers.
    pub use bevy_enhanced_input::prelude::*;
    // Re-export Bevy's SystemId here so Operator impls don't need to import it.
    pub use bevy::ecs::system::SystemId;
}

/// Trait implemented by every extension. Declares the extension's name
/// and registration logic; the framework handles everything else.
pub trait JackdawExtension: Send + Sync + 'static {
    fn name(&self) -> &str;

    /// Classify this extension. Defaults to [`ExtensionKind::Custom`].
    ///
    /// The Extensions dialog reads this to split the list into Built-in
    /// and Custom sections. Reserved as a future hook for marketplace
    /// categories.
    fn kind(&self) -> ExtensionKind {
        ExtensionKind::Custom
    }

    /// Hook for one-time BEI input-context registration.
    ///
    /// Called once per catalog entry at app startup, before any
    /// `register()` call. BEI's `add_input_context::<C>()` must run
    /// exactly once per context type per app lifetime, so it cannot live
    /// inside `register` which runs on every enable.
    ///
    /// Defaults to no-op; override only if the extension adds BEI
    /// contexts.
    fn register_input_contexts(&self, _app: &mut App) {}

    /// Main registration logic. Called each time the extension is
    /// enabled. Spawn operators, windows, BEI action entities, and any
    /// other owned state here.
    fn register(&self, ctx: &mut ExtensionContext);

    /// Optional hook called before the extension entity despawns.
    ///
    /// Child-entity cleanup handles registered windows, operators, BEI
    /// contexts, and observers automatically. Override only for non-ECS
    /// state (file handles, network sessions, and the like).
    fn unregister(&self, _world: &mut World, _extension_entity: Entity) {}
}

/// Passed to [`JackdawExtension::register`]. Holds the extension entity
/// and provides helpers that spawn child entities under it.
///
/// Wraps `&mut World` rather than `&mut App` because extensions may be
/// loaded from world-only contexts such as the Extensions dialog's
/// enable/disable observer. One-time setup that genuinely requires App
/// access (BEI input-context registration) runs through
/// [`JackdawExtension::register_input_contexts`] at catalog-registration
/// time.
pub struct ExtensionContext<'a> {
    world: &'a mut World,
    extension_entity: Entity,
}

impl<'a> ExtensionContext<'a> {
    pub fn new(world: &'a mut World, extension_entity: Entity) -> Self {
        Self {
            world,
            extension_entity,
        }
    }

    /// Direct access to the underlying `World`. Extensions that need to
    /// insert resources or spawn additional entities use this.
    pub fn world(&mut self) -> &mut World {
        self.world
    }

    /// The root [`Extension`] entity. Useful when an extension wants to
    /// spawn additional child entities that should be torn down on
    /// unload.
    pub fn entity(&self) -> Entity {
        self.extension_entity
    }

    /// Register a dock window. Spawns a [`RegisteredWindow`] marker
    /// entity as a child of the extension entity; a cleanup observer
    /// calls `WindowRegistry::unregister` when the marker despawns.
    pub fn register_window(&mut self, descriptor: WindowDescriptor) {
        let ext = self.extension_entity;
        let dock_descriptor = DockWindowDescriptor {
            id: descriptor.id.clone(),
            name: descriptor.name,
            icon: descriptor.icon,
            default_area: descriptor.default_area.unwrap_or_default(),
            priority: descriptor.priority.unwrap_or(100),
            build: descriptor.build,
        };
        self.world
            .resource_mut::<WindowRegistry>()
            .register(dock_descriptor);
        self.world
            .spawn((RegisteredWindow { id: descriptor.id }, ChildOf(ext)));
    }

    /// Register a workspace.
    pub fn register_workspace(&mut self, descriptor: WorkspaceDescriptor) {
        let ext = self.extension_entity;
        let id = descriptor.id.clone();
        self.world
            .resource_mut::<WorkspaceRegistry>()
            .register(descriptor);
        self.world.spawn((RegisteredWorkspace { id }, ChildOf(ext)));
    }

    /// Spawn an entity as a child of the extension entity. Typically
    /// used for BEI context entities with action bindings:
    /// `ctx.spawn((MyContext, actions!(MyContext[...])))`.
    ///
    /// The returned [`EntityWorldMut`] lets the caller keep adding
    /// components or children. Anything spawned this way is torn down
    /// when the extension unloads.
    pub fn spawn<'w>(&'w mut self, bundle: impl Bundle) -> EntityWorldMut<'w> {
        let ext = self.extension_entity;
        let mut ec = self.world.spawn(bundle);
        ec.insert(ChildOf(ext));
        ec
    }

    /// Register an operator. Spawns an [`OperatorEntity`] as a child
    /// of the extension entity and, unless [`Operator::MANUAL`] is
    /// `true`, a `Fire<O>` observer that dispatches the operator
    /// through [`crate::OperatorWorldExt::call_operator`]. BEI binding
    /// modifiers on the actions shape timing (press / release / hold).
    pub fn register_operator<O: Operator>(&mut self) {
        let ext = self.extension_entity;

        let (execute, invoke, availability_check) = {
            let mut queue = bevy::ecs::world::CommandQueue::default();
            let mut commands = Commands::new(&mut queue, self.world);
            let execute = O::register_execute(&mut commands);
            let invoke = O::register_invoke(&mut commands);
            let availability_check = O::register_availability_check(&mut commands);
            queue.apply(self.world);
            (execute, invoke, availability_check)
        };

        let op_entity = self
            .world
            .spawn((
                OperatorEntity {
                    id: O::ID,
                    label: O::LABEL,
                    description: O::DESCRIPTION,
                    execute,
                    invoke,
                    availability_check,
                    modal: O::MODAL,
                },
                ChildOf(ext),
            ))
            .id();

        if !O::MANUAL {
            let observer = Observer::new(
                move |_: bevy::prelude::On<bevy_enhanced_input::prelude::Fire<O>>,
                      mut commands: Commands| {
                    commands.queue(move |world: &mut World| {
                        use crate::OperatorWorldExt;
                        let _ = world.call_operator(O::ID);
                    });
                },
            );
            self.world.spawn((observer, ChildOf(op_entity)));
        }
    }

    /// Inject a section into an existing panel (e.g. add a sub-section to
    /// the Inspector window). Section runs with `In<PanelContext>` each time
    /// the panel re-renders.
    pub fn extend_window<W: ExtensionPoint>(&mut self, section: SectionBuildFn) {
        let ext = self.extension_entity;
        let panel_id = W::ID.to_string();
        let mut registry = self.world.resource_mut::<PanelExtensionRegistry>();
        let section_index = registry.get(&panel_id).len();
        registry.add(panel_id.clone(), section);
        self.world.spawn((
            RegisteredPanelExtension {
                panel_id,
                section_index,
            },
            ChildOf(ext),
        ));
    }

    /// Contribute an entry to one of the editor's top-level menus
    /// (`"Add"`, `"Tools"`, etc.). Clicking the entry dispatches the
    /// referenced operator.
    ///
    /// The menu bar rebuilds automatically when entries are added or
    /// removed. When the extension unloads, its menu entries despawn
    /// with it and the menu rebuilds without them.
    ///
    /// ```ignore
    /// ctx.register_menu_entry(MenuEntryDescriptor {
    ///     menu: "Add".into(),
    ///     label: "My Camera".into(),
    ///     operator_id: PlaceMyCamera::ID,
    /// });
    /// ```
    pub fn register_menu_entry(&mut self, descriptor: MenuEntryDescriptor) {
        let ext = self.extension_entity;
        self.world.spawn((
            RegisteredMenuEntry {
                menu: descriptor.menu,
                label: descriptor.label,
                operator_id: descriptor.operator_id,
            },
            ChildOf(ext),
        ));
    }
}

/// Extension-facing descriptor for a menu bar entry. See
/// [`ExtensionContext::register_menu_entry`].
pub struct MenuEntryDescriptor {
    /// Top-level menu name (`"Add"`, `"Tools"`, etc.).
    pub menu: String,
    /// Text shown on the menu item.
    pub label: String,
    /// ID of an operator registered on the same extension, or any other
    /// loaded extension. Operator IDs are global. Clicking the menu
    /// entry dispatches this operator.
    pub operator_id: &'static str,
}

/// Extension-facing descriptor for a dock window. Mirrors
/// [`jackdaw_panels::DockWindowDescriptor`] but with `default_area`
/// optional: third-party extensions leave it `None` so their windows are
/// not auto-placed, while built-in Jackdaw extensions set it to preserve
/// the default layout.
pub struct WindowDescriptor {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub default_area: Option<String>,
    pub priority: Option<i32>,
    pub build: Arc<dyn Fn(&mut World, Entity) + Send + Sync>,
}

impl Default for WindowDescriptor {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            icon: None,
            default_area: None,
            priority: None,
            build: Arc::new(|_, _| {}),
        }
    }
}

/// Marker trait for panels that accept extension sections.
pub trait ExtensionPoint: 'static {
    const ID: &'static str;
}

pub struct InspectorWindow;
impl ExtensionPoint for InspectorWindow {
    const ID: &'static str = "jackdaw.inspector.components";
}

pub struct HierarchyWindow;
impl ExtensionPoint for HierarchyWindow {
    const ID: &'static str = "jackdaw.hierarchy";
}

/// Context passed to a panel-extension section when it's rendered.
pub struct PanelContext {
    pub window_id: String,
    pub panel_entity: Entity,
}

pub type SectionBuildFn = Arc<dyn Fn(&mut World, PanelContext) + Send + Sync>;

/// Load an extension statically. Spawns an `Extension` entity, runs
/// `extension.register()` against it, returns the entity.
///
/// Takes `&mut World` (not `&mut App`) so this can be called from
/// world-scoped contexts like observer callbacks. BEI input context
/// registration belongs in
/// [`JackdawExtension::register_input_contexts`], which is called at
/// catalog registration time with App access.
pub fn load_static_extension(world: &mut World, extension: Box<dyn JackdawExtension>) -> Entity {
    let name = extension.name().to_string();
    info!("Loading extension: {}", name);

    let extension_entity = world.spawn(Extension { name }).id();

    let mut ctx = ExtensionContext::new(world, extension_entity);
    extension.register(&mut ctx);

    // Store the extension trait object on the entity so `unload_extension`
    // can call `unregister` before despawn.
    world
        .entity_mut(extension_entity)
        .insert(StoredExtension(extension));

    extension_entity
}

/// Internal component holding the extension trait object for the duration
/// of its lifetime. Used by `unload_extension` to invoke the optional
/// `unregister` hook before despawning.
#[derive(Component)]
pub(crate) struct StoredExtension(pub(crate) Box<dyn JackdawExtension>);
