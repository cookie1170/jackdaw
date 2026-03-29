mod apply;
mod ast;
mod emitter;
mod loader;
mod sync;

pub use apply::*;
pub use ast::*;
pub use emitter::*;
pub use loader::*;
pub use sync::*;

use bevy::prelude::*;

pub struct JackdawBsnPlugin;

impl Plugin for JackdawBsnPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SceneBsnAst>();
        // Note: apply_dirty_ast_patches is NOT a per-frame system.
        // It's called explicitly during scene loading only.
    }
}
