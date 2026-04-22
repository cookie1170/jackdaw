//! `SceneJsnAst`-backed implementation of the snapshotter traits.
//!
//! Swapped out for a BSN-backed implementation on BSN migration day.

use std::any::Any;

use bevy::prelude::*;
use jackdaw_api_internal::snapshot::{ActiveSnapshotter, SceneSnapshot, SceneSnapshotter};
use jackdaw_jsn::SceneJsnAst;

pub(super) fn plugin(app: &mut App) {
    app.insert_resource(ActiveSnapshotter(Box::new(JsnAstSnapshotter)));
}

pub struct JsnAstSnapshotter;

impl SceneSnapshotter for JsnAstSnapshotter {
    fn capture(&self, world: &World) -> Box<dyn SceneSnapshot> {
        Box::new(JsnAstSnapshot {
            ast: world.resource::<SceneJsnAst>().clone(),
        })
    }
}

pub struct JsnAstSnapshot {
    ast: SceneJsnAst,
}

impl SceneSnapshot for JsnAstSnapshot {
    fn apply(&self, world: &mut World) {
        crate::scene_io::apply_ast_to_world(world, &self.ast);
    }

    fn equals(&self, other: &dyn SceneSnapshot) -> bool {
        other
            .as_any()
            .downcast_ref::<Self>()
            .is_some_and(|o| self.ast == o.ast)
    }

    fn clone_box(&self) -> Box<dyn SceneSnapshot> {
        Box::new(Self {
            ast: self.ast.clone(),
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
