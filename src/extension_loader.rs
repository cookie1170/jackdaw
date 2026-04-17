//! Plugin that wires up the extension framework into the editor.
//!
//! Adds BEI, sets up the required resources (`OperatorCommandBuffer`,
//! `OperatorIndex`, `PanelExtensionRegistry`, `ExtensionCatalog`,
//! `ActiveModalOperator`), and registers the cleanup observers that keep
//! non-ECS state in sync when extension entities are despawned.
//!
//! Also runs `tick_modal_operator` each frame in Update so modal
//! operators (Blender-style grab/rotate/scale) re-run their invoke
//! system until they return `Finished` or `Cancelled`.

use bevy::prelude::*;
use bevy_enhanced_input::prelude::EnhancedInputPlugin;
use jackdaw_api::{
    ActiveModalOperator, ExtensionCatalog, OperatorCommandBuffer, OperatorIndex,
    PanelExtensionRegistry,
    lifecycle::{
        cleanup_panel_extension_on_remove, cleanup_window_on_remove, cleanup_workspace_on_remove,
        deindex_and_cleanup_operator_on_remove, index_operator_on_add,
    },
    tick_modal_operator,
};

pub struct ExtensionLoaderPlugin;

impl Plugin for ExtensionLoaderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(EnhancedInputPlugin)
            .init_resource::<ExtensionCatalog>()
            .init_resource::<OperatorCommandBuffer>()
            .init_resource::<OperatorIndex>()
            .init_resource::<PanelExtensionRegistry>()
            .init_resource::<ActiveModalOperator>()
            .add_observer(index_operator_on_add)
            .add_observer(deindex_and_cleanup_operator_on_remove)
            .add_observer(cleanup_window_on_remove)
            .add_observer(cleanup_workspace_on_remove)
            .add_observer(cleanup_panel_extension_on_remove)
            .add_systems(Update, tick_modal_operator);
    }
}
