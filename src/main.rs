use bevy::{
    asset::{AssetPlugin, UnapprovedPathMode},
    image::{ImageAddressMode, ImagePlugin, ImageSamplerDescriptor},
    prelude::*,
};
use jackdaw::EditorPlugin;

fn main() -> AppExit {
    // Install a SIGINT/SIGTERM handler before anything else gets a
    // chance to. Something in the dep tree (wgpu, gilrs, or one of
    // their transitive deps) installs its own `ctrlc` handler that
    // swallows the signal without propagating an exit intent — so
    // by default Ctrl+C in the terminal is a no-op for jackdaw.
    // Claiming the handler first with `std::process::exit(130)`
    // guarantees Ctrl+C actually kills the process.
    //
    // Error ignored: if another handler has already been claimed by
    // the time this runs, that's what bevy also reports ("Skipping
    // installing Ctrl+C handler as one was already installed"),
    // and we can't do anything about it from here.
    let _ = ctrlc::set_handler(|| {
        eprintln!("jackdaw: received Ctrl+C, exiting");
        std::process::exit(130);
    });

    let project_root = jackdaw::project::read_last_project()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    // If the parent process respawned us after scaffolding or
    // installing a game, skip the launcher entirely and jump back
    // to wherever the user was. The parent already built +
    // installed the dylib; the startup loader will pick it up
    // normally, so we don't need to rebuild.
    let respawn_skip_build = std::env::var_os(jackdaw::restart::ENV_SKIP_INITIAL_BUILD).is_some();
    let auto_open = if respawn_skip_build {
        jackdaw::project::read_last_project().map(|path| jackdaw::project_select::PendingAutoOpen {
            path,
            skip_build: true,
        })
    } else {
        None
    };

    let mut app = App::new();
    app
        // The default error handler panics, which we never *ever*
        // want to happen to the editor. Log an error instead.
        .set_error_handler(bevy::ecs::error::error)
        .add_plugins(
            DefaultPlugins
                .set(AssetPlugin {
                    file_path: project_root.join("assets").to_string_lossy().to_string(),
                    unapproved_path_mode: UnapprovedPathMode::Allow,
                    ..default()
                })
                .set(ImagePlugin {
                    default_sampler: ImageSamplerDescriptor {
                        address_mode_u: ImageAddressMode::Repeat,
                        address_mode_v: ImageAddressMode::Repeat,
                        address_mode_w: ImageAddressMode::Repeat,
                        ..ImageSamplerDescriptor::linear()
                    },
                }),
        )
        .add_plugins(editor_plugin().build())
        .add_systems(OnEnter(jackdaw::AppState::Editor), spawn_scene);

    if let Some(pending) = auto_open {
        app.insert_resource(pending);
    }

    app.run()
}

/// Build the editor plugin for the prebuilt `jackdaw` binary.
///
/// The dylib loader is always on in the prebuilt binary so users
/// who drop extensions into their config directory don't need to
/// rebuild. The in-tree example extensions (`sample`,
/// `viewable_camera`) are statically registered only when compiled
/// with the `dev` feature; release builds via
/// `--no-default-features` ship without them.
fn editor_plugin() -> EditorPlugin {
    let plugin = EditorPlugin::new().with_dylib_loader();

    #[cfg(feature = "dev")]
    let plugin = plugin
        .with_extension("sample", || Box::new(sample_extension::SampleExtension))
        .with_extension("viewable_camera", || {
            Box::new(viewable_camera_extension::ViewableCameraExtension)
        });

    plugin
}

fn spawn_scene(mut commands: Commands) {
    commands.queue(|world: &mut World| {
        jackdaw::scene_io::spawn_default_lighting(world);
    });
}
