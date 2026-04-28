use bevy::feathers::FeathersPlugins;
use bevy::input_focus::InputDispatchPlugin;
use bevy::prelude::*;
use jackdaw_feathers::EditorFeathersPlugin;
use jackdaw_feathers::picker::{
    PickerItems, PickerProps, SelectInput, SpawnItemInput, match_text, picker_item,
};
use jackdaw_fuzzy::Matchable;

struct Searchable(String);

impl Matchable for Searchable {
    fn get_text(&self) -> String {
        self.0.clone()
    }
}

fn spawn_picker(mut commands: Commands) {
    commands.spawn(Camera2d);

    let items = vec![
        Searchable("Hello world".into()),
        Searchable("Hello there".into()),
        Searchable("Hi there".into()),
        Searchable("Some text".into()),
        Searchable("Some more text".into()),
        Searchable("Another bit of text".into()),
        Searchable("A bunch more text".into()),
        Searchable("And another item to search".into()),
        Searchable("Yet more items to search".into()),
        Searchable("I'm running out of things to say".into()),
        Searchable("Hello world 2: Electric Boogaloo".into()),
        Searchable("Hello there 2: Electric Boogaloo".into()),
        Searchable("Hi there 2: Electric Boogaloo".into()),
        Searchable("Some text 2: Electric Boogaloo".into()),
        Searchable("Some more text 2: Electric Boogaloo".into()),
        Searchable("Another bit of text 2: Electric Boogaloo".into()),
        Searchable("A bunch more text 2: Electric Boogaloo".into()),
        Searchable("And another item to search 2: Electric Boogaloo".into()),
        Searchable("Yet more items to search 2: Electric Boogaloo".into()),
        Searchable("I'm running out of things to say 2: Electric Boogaloo".into()),
    ];

    let props = PickerProps::new(spawn_item, on_select)
        .with_items(items)
        .with_title("Hello world!");

    commands.spawn(props);
}

fn spawn_item(
    In(SpawnItemInput { matched, entities }): In<SpawnItemInput>,
    mut commands: Commands,
) -> Result {
    commands.entity(entities.list).with_child((
        picker_item(matched.index),
        children![match_text(matched.segments)],
    ));

    Ok(())
}

fn on_select(input: In<SelectInput>, items: Query<&PickerItems<Searchable>>) -> Result {
    let item = &items.get(input.entities.picker)?.at(input.index)?;
    info!("Got item {}", item.0);

    Ok(())
}

fn main() -> AppExit {
    App::new()
        .add_plugins((
            DefaultPlugins,
            // text edit enables InputDispatchPlugin unconditionally
            FeathersPlugins.build().disable::<InputDispatchPlugin>(),
            EditorFeathersPlugin,
        ))
        .add_systems(Startup, spawn_picker)
        .insert_resource(ClearColor(jackdaw_feathers::tokens::WINDOW_BG))
        .run()
}
