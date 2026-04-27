use bevy::ecs::lifecycle::HookContext;
use bevy::ecs::system::SystemId;
use bevy::ecs::world::DeferredWorld;
use bevy::feathers::font_styles::InheritableFont;
use bevy::feathers::theme::ThemedText;
use bevy::input_focus::InputFocus;
use bevy::input_focus::tab_navigation::{TabGroup, TabIndex};
use bevy::prelude::*;
use bevy::ui_widgets;
use bevy::ui_widgets::Activate;
use bevy_ui_text_input::SubmitText;
use jackdaw_fuzzy::FuzzyMatcher;
pub use jackdaw_fuzzy::{FuzzyItem, Match};

use crate::cursor::HoverCursor;
use crate::icons::EditorFont;
use crate::scroll::scrollbar;
use crate::text_edit::{TextEditProps, TextEditValue, text_edit};
use crate::tokens;

pub trait Matchable: FuzzyItem + Send + Sync + 'static {}

impl<T: FuzzyItem + Send + Sync + 'static> Matchable for T {}

#[derive(Component)]
#[component(on_replace)]
pub struct Picker {
    matcher: FuzzyMatcher<String>,
    spawn_item: SystemId<In<SpawnItemInput>>,
    on_select: SystemId<In<SelectInput>>,
}

#[derive(Component, Deref, Debug, PartialEq, Clone)]
#[relationship_target(relationship = PickerInputOf)]
pub struct WithPickerInput(Entity);

#[derive(Component, Deref, Debug, PartialEq, Clone)]
#[relationship_target(relationship = PickerListOf)]
pub struct WithPickerList(Entity);

#[derive(Component, Deref, Debug, PartialEq, Clone)]
#[relationship(relationship_target = WithPickerInput)]
pub struct PickerInputOf(pub Entity);

#[derive(Component, Deref, Debug, PartialEq, Clone)]
#[relationship(relationship_target = WithPickerList)]
pub struct PickerListOf(pub Entity);

#[derive(Debug, PartialEq, Clone)]
pub struct PickerEntities {
    pub picker: Entity,
    pub input: Entity,
    pub list: Entity,
}

#[derive(Debug, PartialEq, Clone)]
pub struct SpawnItemInput {
    pub matched: Match,
    pub entities: PickerEntities,
}

#[derive(Debug, PartialEq, Clone)]
pub struct SelectInput {
    pub index: usize,
    pub entities: PickerEntities,
}

#[derive(EntityEvent, Debug, PartialEq, Clone)]
pub struct PickerSelect {
    pub entity: Entity,
    pub index: usize,
}

pub struct PickerProps<T: Matchable> {
    items: Vec<T>,
    register_spawn_item:
        Option<Box<dyn FnOnce(&mut Commands) -> SystemId<In<SpawnItemInput>> + Send + Sync>>,
    register_on_select:
        Option<Box<dyn FnOnce(&mut Commands) -> SystemId<In<SelectInput>> + Send + Sync>>,
}

#[derive(Component)]
struct PickerConfig {
    matcher: Option<FuzzyMatcher<String>>,
    register_spawn_item:
        Option<Box<dyn FnOnce(&mut Commands) -> SystemId<In<SpawnItemInput>> + Send + Sync>>,
    register_on_select:
        Option<Box<dyn FnOnce(&mut Commands) -> SystemId<In<SelectInput>> + Send + Sync>>,
    initialized: bool,
}

#[derive(Component, Debug, Default, PartialEq, Clone)]
pub struct PickerItems<T: Matchable>(Box<[T]>);

impl<T: Matchable> PickerItems<T> {
    pub fn items(&self) -> &[T] {
        &self.0
    }

    pub fn at(&self, index: usize) -> &T {
        &self.0[index]
    }
}

pub fn picker<T: Matchable>(props: PickerProps<T>) -> impl Bundle {
    let PickerProps {
        items,
        register_spawn_item,
        register_on_select,
    } = props;

    let str_items = items.iter().map(|i| i.get_text());
    let matcher = FuzzyMatcher::from_items(str_items);

    (PickerItems(items.into_boxed_slice()), PickerConfig {
        matcher: Some(matcher),
        register_spawn_item,
        register_on_select,
        initialized: false,
    })
}

fn setup_picker(
    pickers: Query<(Entity, &mut PickerConfig), Added<PickerConfig>>,
    mut commands: Commands,
) {
    for (entity, mut config) in pickers {
        if config.initialized {
            continue;
        };
        config.initialized = true;

        let spawn_item = (config.register_spawn_item.take().unwrap())(&mut commands);
        let on_select = (config.register_on_select.take().unwrap())(&mut commands);
        let picker = Picker {
            matcher: config.matcher.take().unwrap(),
            spawn_item,
            on_select,
        };

        let input = commands
            .spawn(text_edit(
                TextEditProps::default()
                    .with_placeholder("Search")
                    .auto_focus(),
            ))
            .id();

        let list = commands
            .spawn(Node {
                flex_direction: FlexDirection::Column,
                width: percent(100),
                max_height: px(400),
                overflow: Overflow::scroll_y(),
                row_gap: px(tokens::SPACING_SM),
                ..Default::default()
            })
            .id();

        let scrollbar = commands.spawn(scrollbar(list)).id();

        let list_container = commands
            .spawn(Node {
                width: percent(100),
                ..Default::default()
            })
            .add_children(&[scrollbar, list])
            .id();

        let picker_entity = commands
            .spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    padding: px(tokens::SPACING_MD).all(),
                    border: px(tokens::SPACING_XS).all(),
                    border_radius: BorderRadius::all(px(tokens::BORDER_RADIUS_MD)),
                    row_gap: px(tokens::SPACING_MD),
                    width: px(600),
                    ..Default::default()
                },
                BorderColor::all(tokens::BORDER_STRONG),
                BackgroundColor(tokens::PANEL_BG),
                TabGroup::modal(),
            ))
            .add_children(&[input, list_container])
            .id();

        commands
            .entity(entity)
            .insert((
                Node {
                    height: percent(100),
                    width: percent(100),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    ..Default::default()
                },
                picker,
            ))
            .add_one_related::<PickerInputOf>(input)
            .add_one_related::<PickerListOf>(list)
            .add_child(picker_entity);
    }
}

#[derive(Component, Debug, Default, PartialEq, Clone, Copy)]
pub struct PickerItem(pub usize);

pub fn picker_item(index: usize) -> impl Bundle {
    (
        Node {
            width: percent(100),
            padding: px(tokens::SPACING_SM).all(),
            border_radius: BorderRadius::all(px(tokens::BORDER_RADIUS_SM)),
            ..Default::default()
        },
        BackgroundColor(tokens::ELEVATED_BG),
        Interaction::default(),
        HoverCursor(bevy::window::SystemCursorIcon::Pointer),
        PickerItem(index),
        // if everything is the same tab index, it's ordered by the child index
        TabIndex(0),
        ui_widgets::Button,
    )
}

fn on_picker_item_activated(
    trigger: On<Activate>,
    item: Query<&PickerItem>,
    list: Query<&PickerListOf>,
    child_of: Query<&ChildOf>,
    mut commands: Commands,
) {
    let Ok(item) = item.get(trigger.entity) else {
        return;
    };

    let Some(list_of) = std::iter::once(trigger.entity)
        .chain(child_of.iter_ancestors(trigger.entity))
        .find_map(|e| list.get(e).ok())
    else {
        return;
    };

    commands.trigger(PickerSelect {
        entity: list_of.0,
        index: item.0,
    });
}

fn scroll_to_picker_item(
    picker_items: Query<(&ComputedNode, &UiGlobalTransform, &ChildOf), With<PickerItem>>,
    mut scroll_position: Query<(&mut ScrollPosition, &ComputedNode, &UiGlobalTransform)>,
    focus: Res<InputFocus>,
) {
    if !focus.is_changed() {
        return;
    };

    let Some(focused) = focus.0 else {
        return;
    };

    let Ok((computed, transform, parent)) = picker_items.get(focused) else {
        return;
    };

    let Ok((mut scroll_position, parent_computed, parent_transform)) =
        scroll_position.get_mut(parent.0)
    else {
        return;
    };

    let child_top = transform.translation.y - computed.size().y / 2.0;
    let child_bottom = transform.translation.y + computed.size().y / 2.0;
    let parent_top = parent_transform.translation.y - parent_computed.content_box().size().y / 2.0;

    // since scrolling changes the child positions, we add back the scroll to counteract that
    let child_top_relative = child_top - parent_top + scroll_position.y;
    let child_bottom_relative = child_bottom - parent_top + scroll_position.y;

    // the bottom most visible point
    let bottom_visible = scroll_position.y + parent_computed.content_box().size().y;

    // ui position increases downwards, so if the top is above the scroll position, we scroll
    if child_top_relative < scroll_position.y {
        // off screen at the top
        scroll_position.y = child_top_relative;
    }

    // and if the bottom is below the bottom most visible point, we scroll
    if child_bottom_relative > bottom_visible {
        // off screen at the bottom
        // subtract to account for the parent size
        scroll_position.y = f32::max(
            child_bottom_relative - parent_computed.content_box().size().y,
            0.0,
        );
    }
}

fn handle_picker_item_hover(
    picker_items: Query<(Entity, &Interaction, &mut BackgroundColor), With<PickerItem>>,
    focus: Res<InputFocus>,
) {
    for (entity, interaction, mut background) in picker_items {
        let mut interaction = interaction.clone();
        if focus.0.is_some_and(|f| f == entity) && interaction != Interaction::Pressed {
            interaction = Interaction::Hovered;
        }

        match interaction {
            Interaction::Pressed => {
                background.0 = tokens::ACTIVE_BG;
            }
            Interaction::Hovered => {
                background.0 = tokens::HOVER_BG;
            }
            Interaction::None => {
                background.0 = tokens::ELEVATED_BG;
            }
        }
    }
}

#[derive(Component)]
#[component(on_insert)]
struct MatchText;

impl MatchText {
    fn on_insert(mut world: DeferredWorld, ctx: HookContext) {
        let font = world.resource::<EditorFont>().0.clone();
        let mut commands = world.commands();
        commands
            .entity(ctx.entity)
            .insert(InheritableFont::from_handle(font));
    }
}

pub fn match_text(matched: Match) -> impl Bundle {
    let mut spans = Vec::with_capacity(matched.segments.len());

    for segment in matched.segments {
        let color = if segment.is_match {
            tokens::TEXT_ACCENT
        } else {
            tokens::TEXT_PRIMARY.into()
        };
        spans.push((TextSpan(segment.text), ThemedText, TextColor(color)));
    }

    (Text::default(), Children::spawn(spans), MatchText)
}

fn process_fuzzy_pickers(
    pickers: Query<(Entity, &mut Picker, &WithPickerInput, &WithPickerList)>,
    text_edits: Query<&TextEditValue, Changed<TextEditValue>>,
    mut commands: Commands,
) {
    for (entity, mut picker, input_entity, list) in pickers {
        let Ok(input) = text_edits.get(input_entity.0) else {
            continue;
        };
        commands.entity(list.0).despawn_children();

        picker.matcher.update_pattern(&input.0);

        let spawn_item = picker.spawn_item;

        for matched in picker.matcher.matches() {
            let input = SpawnItemInput {
                matched,
                entities: PickerEntities {
                    picker: entity,
                    input: input_entity.0,
                    list: list.0,
                },
            };

            commands.run_system_with(spawn_item, input);
        }
    }
}

fn on_fuzzy_picker_select(
    trigger: On<PickerSelect>,
    pickers: Query<(&mut Picker, &WithPickerInput, &WithPickerList)>,
    mut commands: Commands,
) {
    let Ok((picker, input, list)) = pickers.get(trigger.entity) else {
        return;
    };

    let input = SelectInput {
        index: trigger.index,
        entities: PickerEntities {
            picker: trigger.entity,
            input: input.0,
            list: list.0,
        },
    };

    commands.run_system_with(picker.on_select, input);
}

fn on_text_edit_submit(
    mut submit_messages: MessageReader<SubmitText>,
    inputs: Query<&PickerInputOf>,
    child_of: Query<&ChildOf>,
    mut pickers: Query<(Entity, &mut Picker)>,
    mut commands: Commands,
) {
    for submit in submit_messages.read() {
        // please give me relational queries i'm begging
        let Some(input_of) = std::iter::once(submit.entity)
            .chain(child_of.iter_ancestors(submit.entity))
            .find_map(|e| inputs.get(e).ok())
        else {
            continue;
        };

        let Ok((picker_entity, mut picker)) = pickers.get_mut(input_of.0) else {
            continue;
        };

        picker.matcher.update_pattern(&submit.text);
        let Some(first) = picker.matcher.matches().next() else {
            continue;
        };

        commands.trigger(PickerSelect {
            entity: picker_entity,
            index: first.index,
        });
    }
}

impl<T: Matchable> PickerProps<T> {
    pub fn new<S1, M1, S2, M2>(spawn_item: S1, on_select: S2) -> Self
    where
        S1: IntoSystem<In<SpawnItemInput>, (), M1>,
        S2: IntoSystem<In<SelectInput>, (), M2>,
    {
        let spawn_item = IntoSystem::into_system(spawn_item);
        let on_select = IntoSystem::into_system(on_select);
        Self {
            items: vec![],
            register_spawn_item: Some(Box::new(move |commands| {
                commands.register_system(spawn_item)
            })),
            register_on_select: Some(Box::new(move |commands| {
                commands.register_system(on_select)
            })),
        }
    }

    pub fn with_items(mut self, items: impl IntoIterator<Item = T>) -> Self {
        self.items.extend(items);
        self
    }

    pub fn with_item(mut self, item: T) -> Self {
        self.items.push(item);
        self
    }
}

impl Picker {
    fn on_replace(mut world: DeferredWorld, ctx: HookContext) {
        let entity = world.entity(ctx.entity);
        let picker = entity.get::<Self>().unwrap();
        let (spawn_item, on_select) = (picker.spawn_item, picker.on_select);
        let mut commands = world.commands();
        // Clean up after ourselves!
        commands.unregister_system(spawn_item);
        commands.unregister_system(on_select);
    }
}

pub(crate) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        (
            setup_picker,
            process_fuzzy_pickers,
            on_text_edit_submit,
            handle_picker_item_hover,
            scroll_to_picker_item,
        ),
    )
    .add_observer(on_fuzzy_picker_select)
    .add_observer(on_picker_item_activated);
}
