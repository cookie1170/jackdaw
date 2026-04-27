use bevy::prelude::*;
use jackdaw_api::prelude::*;
use jackdaw_api_internal::lifecycle::OperatorEntity;
use jackdaw_feathers::icons::EditorFont;
use jackdaw_feathers::picker::{
    Matchable, PickerItems, PickerProps, SelectInput, SpawnItemInput, match_text, picker,
    picker_item,
};
use jackdaw_feathers::tokens;

use crate::core_extension::CoreExtensionInputContext;

#[derive(Component)]
struct CommandPalette;

pub(crate) fn add_to_extension(ctx: &mut ExtensionContext) {
    ctx.register_operator::<ToggleCommandPaletteOp>();

    let ext = ctx.id();
    ctx.spawn((
        ActionOf::<CoreExtensionInputContext>::new(ext),
        Action::<ToggleCommandPaletteOp>::new(),
        bindings![
            (
                KeyCode::Space.with_mod_keys(ModKeys::CONTROL),
                bevy_enhanced_input::prelude::Press::default()
            ),
            (KeyCode::F3, bevy_enhanced_input::prelude::Press::default())
        ],
    ));
}

#[operator(
    id = "command_palette.toggle",
    label = "Toggle command palette",
    allows_undo = false
)]
pub(crate) fn toggle_command_palette(
    _: In<OperatorParameters>,
    // need world access to run the availability checks :(
    world: &mut World,
) -> OperatorResult {
    for existing in world
        .query_filtered::<Entity, With<CommandPalette>>()
        .query(world)
    {
        world.entity_mut(existing).despawn();
        return OperatorResult::Finished;
    }

    let operators = get_operators(world);
    let props = PickerProps::new(spawn_item, on_select)
        .with_items(operators)
        .with_title("Command Palette");

    world.spawn((picker(props), CommandPalette));

    OperatorResult::Finished
}

fn get_operators(world: &mut World) -> impl Iterator<Item = RegisteredOperator> {
    let mut operator_entities = world.query::<&OperatorEntity>();
    let operator_entities = operator_entities.query(world);
    let mut operators = Vec::with_capacity(operator_entities.iter().len());

    for operator in operator_entities {
        operators.push(RegisteredOperator {
            label: operator.label(),
            id: operator.id(),
        });
    }

    operators
        .into_iter()
        .filter(|op| world.operator(op.id).is_available().unwrap_or(false))
}

fn spawn_item(
    In(SpawnItemInput { matched, entities }): In<SpawnItemInput>,
    items: Query<&PickerItems<RegisteredOperator>>,
    font: Res<EditorFont>,
    mut commands: Commands,
) {
    let item = items.get(entities.picker).unwrap().at(matched.index);

    let item = commands
        .spawn((picker_item(matched.index), children![(
            Node {
                width: percent(100),
                align_items: AlignItems::Center,
                flex_direction: FlexDirection::Row,
                justify_content: JustifyContent::SpaceBetween,
                ..Default::default()
            },
            children![
                match_text(matched.segments),
                (
                    Text::new(item.id),
                    TextFont::from(font.0.clone()).with_font_size(tokens::TEXT_SIZE_SM),
                    TextColor(tokens::TEXT_MUTED_COLOR.into())
                )
            ]
        )]))
        .id();

    commands.entity(entities.list).add_child(item);
}

fn on_select(
    input: In<SelectInput>,
    items: Query<&PickerItems<RegisteredOperator>>,
    mut commands: Commands,
) {
    let item = items.get(input.entities.picker).unwrap().at(input.index);

    commands.operator(item.id).call();

    commands.entity(input.entities.picker).try_despawn();
}

#[derive(Debug, PartialEq, Clone, Copy)]
struct RegisteredOperator {
    label: &'static str,
    id: &'static str,
}

impl Matchable for RegisteredOperator {
    fn get_text(&self) -> String {
        String::from(self.label)
    }
}
