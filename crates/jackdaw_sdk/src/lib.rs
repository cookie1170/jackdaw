//! Proxy dylib shipped with jackdaw.
//!
//! Extensions are built via `cargo rustc` with:
//!
//! ```text
//! -C prefer-dynamic
//! --extern bevy=<jackdaw>/target/debug/libjackdaw_sdk.so
//! --extern bevy=<jackdaw>/target/debug/deps/libjackdaw_sdk.rlib
//! --extern jackdaw_api=<jackdaw>/target/debug/libjackdaw_sdk.so
//! --extern jackdaw_api=<jackdaw>/target/debug/deps/libjackdaw_sdk.rlib
//! -L dependency=<jackdaw>/target/debug/deps
//! ```
//!
//! The `--extern` aliases rename this proxy as `bevy` and
//! `jackdaw_api` during compilation of the extension, so extension
//! code writes plain `use bevy::prelude::*;` and
//! `use jackdaw_api::prelude::*;`. Both resolve to this crate's
//! re-exports, which ultimately point at the one compilation of
//! bevy and jackdaw_api that was built alongside the editor.
//!
//! The re-exports below are explicit rather than glob-based because
//! `pub use bevy::*; pub use jackdaw_api::*;` would make `prelude`
//! (and any other same-named items) ambiguous and unusable.

// A single merged prelude that serves both aliased names.
// `use bevy::prelude::*` (aliased to `jackdaw_sdk::prelude`) and
// `use jackdaw_api::prelude::*` (also aliased to
// `jackdaw_sdk::prelude`) both land here.
pub mod prelude {
    // bevy::prelude and jackdaw_api::prelude both re-export a few
    // same-named items (e.g. `Press`, `Release` from `bevy_input`
    // vs. `bevy_enhanced_input`). Extensions that reference those
    // names unqualified will need to disambiguate. Globbing both
    // is still the best UX — authors rarely touch the overlapping
    // names.
    #[allow(ambiguous_glob_reexports)]
    pub use bevy::prelude::*;
    #[allow(ambiguous_glob_reexports)]
    pub use jackdaw_api::prelude::*;
}

// The `export_extension!` / `export_game!` macros. Their
// expansions reference `$crate::ffi::{ExtensionEntry,GameEntry}`
// and `$crate::JackdawExtension`, where `$crate` resolves to the
// macro's defining crate (`jackdaw_api`). After `--extern`
// aliasing, that path becomes `jackdaw_sdk::...`, which is why
// those items are re-exported at the crate root below.
pub use jackdaw_api::export_extension;
pub use jackdaw_api::export_game;

// Re-exports needed by `export_extension!` macro expansion.
pub use jackdaw_api::JackdawExtension;
pub use jackdaw_api::ffi;

// Common items extensions reference by full path (e.g.
// `jackdaw_api::WindowDescriptor`) rather than through the
// prelude. Keep the list explicit so name collisions between
// bevy and jackdaw_api surfaces stay visible.
pub use jackdaw_api::{
    ActiveModalOperator, CallOperatorError, CallOperatorSettings, Extension, ExtensionCatalog,
    ExtensionContext, ExtensionKind, ExtensionPoint, HierarchyWindow, InspectorWindow,
    MenuEntryDescriptor, Operator, OperatorEntity, OperatorIndex, OperatorResult, OperatorSession,
    OperatorWorldExt, PanelContext, PanelExtensionRegistry, PlayState, RegisteredMenuEntry,
    RegisteredPanelExtension, RegisteredWindow, RegisteredWorkspace, SectionBuildFn,
    WindowDescriptor, operator,
};

// Bevy root surface for extension code that walks bevy paths
// beyond prelude. The glob's safe because the explicit jackdaw_api
// re-exports above are all items bevy doesn't define at its root.
pub use bevy::*;
