use bevy::{asset::{AssetPlugin, UnapprovedPathMode}, light::GlobalAmbientLight, prelude::*};
use jackdaw::EditorPlugin;

fn main() -> AppExit {
    let project_root = jackdaw::project::read_last_project()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    App::new()
        .add_plugins(DefaultPlugins.set(AssetPlugin {
            file_path: project_root.join("assets").to_string_lossy().to_string(),
            unapproved_path_mode: UnapprovedPathMode::Allow,
            ..default()
        }))
        .add_plugins(EditorPlugin)
        .add_systems(OnEnter(jackdaw::AppState::Editor), spawn_scene)
        .run()
}

fn spawn_scene(mut commands: Commands) {
    commands.insert_resource(GlobalAmbientLight {
        color: Color::WHITE,
        brightness: 400.0,
        affects_lightmapped_meshes: true,
    });

    commands.spawn((
        Name::new("Sun"),
        DirectionalLight {
            shadows_enabled: true,
            illuminance: 10000.0,
            ..default()
        },
        Transform::from_xyz(10.0, 20.0, 10.0).with_rotation(Quat::from_euler(
            EulerRot::XYZ,
            -0.8,
            0.4,
            0.0,
        )),
    ));
}
