use bevy::{
    prelude::*,
    render::{
        RenderPlugin,
        settings::{RenderCreation, WgpuSettings},
    },
    winit::WinitPlugin,
};
use jackdaw::prelude::*;

pub fn headless_app() -> App {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(RenderPlugin {
                render_creation: RenderCreation::Automatic(WgpuSettings {
                    backends: None,
                    ..default()
                }),
                ..default()
            })
            .disable::<WinitPlugin>(),
    )
    .add_plugins(EditorPlugin::new().build());
    app
}

pub trait OperatorResultExt: Copy {
    /// Asserts that the operator finished successfully and panics if it did not.
    /// Hidden away in test utils so extension devs don't fall into the trap of actually doing this in production.
    fn assert_finished(self);
}

impl OperatorResultExt for OperatorResult {
    fn assert_finished(self) {
        assert_eq!(self, OperatorResult::Finished, "Operator failed to finish");
    }
}
