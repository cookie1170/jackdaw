mod operator;
mod registries;

pub use operator::{Operator, OperatorContext, OperatorResult};
pub use registries::{
    KeyCombo, KeybindRegistry, Modifiers, OperatorRegistry, PanelExtensionRegistry,
};

use std::sync::Arc;

use bevy::prelude::*;
use jackdaw_panels::{
    DockWindowDescriptor, WindowRegistry, WorkspaceDescriptor, WorkspaceRegistry,
};

pub mod prelude {
    pub use crate::{
        ExtensionContext, ExtensionPoint, JackdawExtension, PanelContext, SectionBuildFn,
        WindowDescriptor,
    };
    pub use crate::{KeyCombo, Modifiers};
    pub use crate::{Operator, OperatorContext, OperatorResult};
}

pub trait JackdawExtension: Send + Sync + 'static {
    fn name(&self) -> &str;
    fn register(&self, ctx: &mut ExtensionContext);
    fn unregister(&self, _ctx: &mut ExtensionContext) {}
}

pub struct ExtensionContext<'a> {
    app: &'a mut App,
}

impl<'a> ExtensionContext<'a> {
    pub fn new(app: &'a mut App) -> Self {
        Self { app }
    }

    pub fn app(&mut self) -> &mut App {
        self.app
    }

    pub fn register_window(&mut self, descriptor: WindowDescriptor) {
        let dock_descriptor = DockWindowDescriptor {
            id: descriptor.id,
            name: descriptor.name,
            icon: descriptor.icon,
            default_area: String::new(),
            priority: 100,
            build: descriptor.build,
        };
        self.app
            .world_mut()
            .resource_mut::<WindowRegistry>()
            .register(dock_descriptor);
    }

    pub fn register_workspace(&mut self, descriptor: WorkspaceDescriptor) {
        self.app
            .world_mut()
            .resource_mut::<WorkspaceRegistry>()
            .register(descriptor);
    }

    pub fn register_operator<O: Operator + Default>(&mut self) {
        self.app
            .world_mut()
            .resource_mut::<OperatorRegistry>()
            .register::<O>();
    }

    pub fn register_keybind(&mut self, keys: KeyCombo, operator_id: &str) {
        self.app
            .world_mut()
            .resource_mut::<KeybindRegistry>()
            .bind(keys, operator_id.to_string());
    }

    pub fn extend_window<W: ExtensionPoint>(&mut self, section: SectionBuildFn) {
        self.app
            .world_mut()
            .resource_mut::<PanelExtensionRegistry>()
            .add(W::ID.to_string(), section);
    }
}

pub struct WindowDescriptor {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub build: Arc<dyn Fn(&mut World, Entity) + Send + Sync>,
}

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

pub struct PanelContext {
    pub window_id: String,
    pub panel_entity: Entity,
}

pub type SectionBuildFn = Arc<dyn Fn(&mut World, PanelContext) + Send + Sync>;

pub fn load_static_extension(app: &mut App, extension: &dyn JackdawExtension) {
    info!("Loading extension: {}", extension.name());
    let mut ctx = ExtensionContext::new(app);
    extension.register(&mut ctx);
}
