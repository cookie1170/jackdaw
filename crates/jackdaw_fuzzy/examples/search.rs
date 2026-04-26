use bevy::color::palettes::tailwind;
use bevy::feathers::FeathersPlugins;
use bevy::input_focus::InputDispatchPlugin;
use bevy::prelude::*;
use jackdaw_feathers::text_edit::{TextEditProps, text_edit};
use jackdaw_fuzzy::prelude::*;

fn spawn_fuzzy_picker(mut commands: Commands) {
    commands.spawn(Camera2d);

    let items = vec![
        SearchableItem {
            some_text: String::from("Hello world"),
            another_field: String::from("This field"),
        },
        SearchableItem {
            some_text: String::from("Hello there"),
            another_field: String::from("Isn't searched"),
        },
        SearchableItem {
            some_text: String::from("How are you?"),
            another_field: String::from("How cool"),
        },
    ];

    // `bsn!` will make this much less painful
    let input = commands
        .spawn(text_edit(
            TextEditProps::default().with_placeholder("Search"),
        ))
        .id();

    let input_wrapper = commands
        .spawn(Node {
            width: px(240),
            ..default()
        })
        .add_child(input)
        .id();

    let list = commands
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            ..Default::default()
        })
        .id();

    let picker = FuzzyPicker::new(spawn_item, on_select).with_items(items);

    commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                width: percent(100),
                height: percent(100),
                row_gap: px(8),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..Default::default()
            },
            picker,
        ))
        .add_one_related::<PickerListOf>(list)
        .add_one_related::<PickerInputOf>(input)
        .add_children(&[input_wrapper, list]);
}

fn spawn_item(
    In(SpawnItemInput {
        matched: Match {
            index,
            segments,
            score,
        },
        entities: PickerEntities { picker, list, .. },
    }): In<SpawnItemInput>,
    pickers: Query<&FuzzyPicker<SearchableItem>>,
    mut commands: Commands,
) {
    let item = &pickers.get(picker).unwrap().items()[index];

    let mut text_spans = Vec::with_capacity(segments.len());
    for segment in segments {
        let mut span = commands.spawn(TextSpan(segment.text));
        if segment.is_match {
            span.insert(TextColor(Color::Srgba(tailwind::ROSE_300)));
        }

        text_spans.push(span.id());
    }

    let text = commands
        .spawn(Text(format!("({score}) {}, ", item.another_field)))
        .add_children(&text_spans)
        .id();

    commands.entity(list).add_child(text);
}

fn on_select(
    In(SelectInput {
        index,
        entities: PickerEntities { picker, .. },
    }): In<SelectInput>,
    pickers: Query<&FuzzyPicker<SearchableItem>>,
) {
    let item = &pickers.get(picker).unwrap().items()[index];

    info!("Picked '{}'", item.some_text);
}

struct SearchableItem {
    some_text: String,
    another_field: String,
}

impl FuzzyItem for SearchableItem {
    fn get_text(&self) -> String {
        self.some_text.clone()
    }
}

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins,
            FeathersPlugins.build().disable::<InputDispatchPlugin>(),
            jackdaw_feathers::EditorFeathersPlugin,
        ))
        .add_systems(Startup, spawn_fuzzy_picker)
        .register_fuzzy_item::<SearchableItem>()
        .run();
}
