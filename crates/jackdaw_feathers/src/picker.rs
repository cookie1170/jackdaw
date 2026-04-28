use bevy::ecs::lifecycle::HookContext;
use bevy::ecs::relationship::RelatedSpawner;
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
pub use jackdaw_fuzzy::{Match, Matchable, MatchedStr};
use lucide_icons::Icon;

use crate::button::{ButtonClickEvent, IconButtonProps, icon_button};
use crate::cursor::HoverCursor;
use crate::icons::{EditorFont, IconFont};
use crate::scroll::scrollbar;
use crate::separator::{SeparatorProps, separator};
use crate::text_edit::{TextEditProps, TextEditValue, text_edit};
use crate::tokens;

pub trait Pickable: Matchable + Send + Sync + 'static {}

impl<T: Matchable + Send + Sync + 'static> Pickable for T {}

#[derive(Component)]
#[component(on_replace)]
pub struct Picker {
    matcher: FuzzyMatcher<String>,
    spawn_item: SystemId<In<SpawnItemInput>, Result>,
    on_select: SystemId<In<SelectInput>, Result>,
    on_dismiss: SystemId<In<PickerEntities>, Result>,
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

pub struct PickerProps<T: Pickable> {
    items: Vec<T>,
    title: Option<String>,
    dismissible: bool,
    register_spawn_item: Option<
        Box<dyn FnOnce(&mut Commands) -> SystemId<In<SpawnItemInput>, Result> + Send + Sync>,
    >,
    register_on_select:
        Option<Box<dyn FnOnce(&mut Commands) -> SystemId<In<SelectInput>, Result> + Send + Sync>>,
    register_on_dismiss: Option<
        Box<dyn FnOnce(&mut Commands) -> SystemId<In<PickerEntities>, Result> + Send + Sync>,
    >,
}

#[derive(Component)]
struct PickerConfig {
    matcher: Option<FuzzyMatcher<String>>,
    title: Option<String>,
    dismissible: bool,
    register_spawn_item: Option<
        Box<dyn FnOnce(&mut Commands) -> SystemId<In<SpawnItemInput>, Result> + Send + Sync>,
    >,
    register_on_select:
        Option<Box<dyn FnOnce(&mut Commands) -> SystemId<In<SelectInput>, Result> + Send + Sync>>,
    register_on_dismiss: Option<
        Box<dyn FnOnce(&mut Commands) -> SystemId<In<PickerEntities>, Result> + Send + Sync>,
    >,

    initialized: bool,
}

fn setup_picker(
    pickers: Query<(Entity, &mut PickerConfig), Added<PickerConfig>>,
    font: Res<EditorFont>,
    icon_font: Res<IconFont>,
    mut commands: Commands,
) {
    for (entity, mut config) in pickers {
        if config.initialized {
            continue;
        };
        config.initialized = true;

        let spawn_item = (config.register_spawn_item.take().unwrap())(&mut commands);
        let on_select = (config.register_on_select.take().unwrap())(&mut commands);
        let on_dismiss = (config.register_on_dismiss.take().unwrap())(&mut commands);
        let picker = Picker {
            matcher: config.matcher.take().unwrap(),
            spawn_item,
            on_select,
            on_dismiss,
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
                ..default()
            })
            .id();

        let scrollbar = commands.spawn(scrollbar(list)).id();

        let list_container = commands
            .spawn(Node {
                width: percent(100),
                ..default()
            })
            .add_children(&[scrollbar, list])
            .id();

        let dismiss = if config.dismissible {
            Some((
                PickerDismissButton,
                icon_button(IconButtonProps::new(Icon::X), &icon_font.0),
            ))
        } else {
            None
        };

        let mut children = vec![];

        if let Some(title) = config.title.take() {
            let font = font.0.clone();

            let titlebar = commands
                .spawn((
                    Node {
                        padding: px(tokens::SPACING_XS).all(),
                        align_items: AlignItems::Center,
                        width: percent(100),
                        ..default()
                    },
                    Children::spawn(SpawnWith(|spawner: &mut RelatedSpawner<ChildOf>| {
                        spawner.spawn((
                            Node {
                                flex_grow: 1.0,
                                justify_content: JustifyContent::Center,
                                ..default()
                            },
                            children![(
                                Text(title),
                                TextFont::from(font).with_font_size(tokens::TEXT_SIZE_XL),
                            )],
                        ));

                        if let Some(dismiss) = dismiss {
                            spawner.spawn(dismiss);
                        }
                    })),
                ))
                .id();

            children.extend(&[titlebar, input]);
        } else if let Some(dismiss) = dismiss {
            // if we put the dismiss button in the title bar with no title, it looks ugly
            // because there's a lot of empty space so we put it after the input instead
            let dismiss = commands.spawn(dismiss).id();
            let input_container = commands
                .spawn(Node {
                    width: percent(100),
                    column_gap: px(tokens::SPACING_SM),
                    ..default()
                })
                .add_children(&[input, dismiss])
                .id();

            children.push(input_container);
        } else {
            children.push(input);
        }

        let separator = commands.spawn(separator(SeparatorProps::horizontal())).id();

        children.extend(&[separator, list_container]);

        let picker_entity = commands
            .spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    padding: px(tokens::SPACING_MD).all(),
                    border: px(tokens::SPACING_XS).all(),
                    border_radius: BorderRadius::all(px(tokens::BORDER_RADIUS_MD)),
                    row_gap: px(tokens::SPACING_MD),
                    width: px(600),
                    ..default()
                },
                BorderColor::all(tokens::BORDER_STRONG),
                BackgroundColor(tokens::PANEL_BG),
                TabGroup::modal(),
            ))
            .add_children(&children)
            .id();

        commands
            .entity(entity)
            .insert((
                Node {
                    height: percent(100),
                    width: percent(100),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                picker,
            ))
            .add_one_related::<PickerInputOf>(input)
            .add_one_related::<PickerListOf>(list)
            .add_child(picker_entity);
    }
}

#[derive(Component, Debug, Default, PartialEq, Clone)]
pub struct PickerItems<T: Pickable>(Box<[T]>);

impl<T: Pickable> PickerItems<T> {
    pub fn items(&self) -> &[T] {
        &self.0
    }

    pub fn at(&self, index: usize) -> Result<&T> {
        // return a `BevyError` so you can just `?` it
        self.0
            .get(index)
            .ok_or_else(|| BevyError::from(format!("No item at index {index}")))
    }
}

pub fn picker<T: Pickable>(props: PickerProps<T>) -> impl Bundle {
    let PickerProps {
        items,
        title,
        dismissible,
        register_spawn_item,
        register_on_select,
        register_on_dismiss,
    } = props;

    let str_items = items.iter().map(Matchable::get_text);
    let matcher = FuzzyMatcher::from_items(str_items);

    (
        PickerItems(items.into_boxed_slice()),
        PickerConfig {
            matcher: Some(matcher),
            title,
            dismissible,
            register_spawn_item,
            register_on_select,
            register_on_dismiss,
            initialized: false,
        },
        GlobalZIndex(1000),
    )
}

#[derive(Component, Debug, Default, PartialEq, Clone, Copy)]
pub struct PickerItem(pub usize);

#[must_use]
pub fn picker_item(index: usize) -> impl Bundle {
    (
        Node {
            width: percent(100),
            padding: px(tokens::SPACING_SM).all(),
            border_radius: BorderRadius::all(px(tokens::BORDER_RADIUS_SM)),
            ..default()
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

fn handle_picker_item_hover(
    picker_items: Query<(Entity, &Interaction, &mut BackgroundColor), With<PickerItem>>,
    focus: Res<InputFocus>,
) {
    for (entity, interaction, mut background) in picker_items {
        let mut interaction = *interaction;
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

#[derive(Component)]
struct PickerDismissButton;

fn on_dismiss_activated(
    trigger: On<ButtonClickEvent>,
    picker_dismiss_query: Query<(), With<PickerDismissButton>>,
    child_of: Query<&ChildOf>,
    picker_query: Query<(Entity, &Picker, &WithPickerInput, &WithPickerList)>,
    mut commands: Commands,
) {
    if picker_dismiss_query.get(trigger.entity).is_err() {
        return;
    };

    let Some((picker_entity, picker, input, list)) = std::iter::once(trigger.entity)
        .chain(child_of.iter_ancestors(trigger.entity))
        .find_map(|e| picker_query.get(e).ok())
    else {
        return;
    };

    let entities = PickerEntities {
        picker: picker_entity,
        input: input.0,
        list: list.0,
    };

    let on_dismiss = picker.on_dismiss;

    commands.queue(move |world: &mut World| {
        if let Err(e) = world.run_system_with(on_dismiss, entities) {
            error!("Error when dismissing picker {picker_entity}: {e}");
        }
    });
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

pub fn match_text(segments: Box<[MatchedStr]>) -> impl Bundle {
    let mut spans = Vec::with_capacity(segments.len());

    for segment in segments {
        let color = if segment.is_match {
            tokens::TEXT_ACCENT
        } else {
            tokens::TEXT_PRIMARY
        };
        spans.push((TextSpan(segment.text), ThemedText, TextColor(color)));
    }

    (Text::default(), Children::spawn(spans), MatchText)
}

fn process_pickers(
    pickers: Query<(Entity, &mut Picker, &WithPickerInput, &WithPickerList)>,
    text_edits: Query<&TextEditValue, Changed<TextEditValue>>,
    mut commands: Commands,
) {
    for (picker_entity, mut picker, input_entity, list) in pickers {
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
                    picker: picker_entity,
                    input: input_entity.0,
                    list: list.0,
                },
            };

            commands.queue(move |world: &mut World| {
                if let Err(e) = world.run_system_with(spawn_item, input) {
                    error!("Error when spawning item for picker {picker_entity}: {e}");
                }
            });
        }
    }
}

fn on_picker_select(
    trigger: On<PickerSelect>,
    pickers: Query<(&Picker, &WithPickerInput, &WithPickerList)>,
    mut commands: Commands,
) {
    let Ok((picker, input, list)) = pickers.get(trigger.entity) else {
        return;
    };

    let picker_entity = trigger.entity;

    let input = SelectInput {
        index: trigger.index,
        entities: PickerEntities {
            picker: picker_entity,
            input: input.0,
            list: list.0,
        },
    };

    let on_select = picker.on_select;
    commands.queue(move |world: &mut World| {
        if let Err(e) = world.run_system_with(on_select, input) {
            error!("Error when selecting item on picker {picker_entity}: {e}");
        }
    });
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

impl<T: Pickable> PickerProps<T> {
    pub fn new<S1, M1, S2, M2>(spawn_item: S1, on_select: S2) -> Self
    where
        S1: IntoSystem<In<SpawnItemInput>, Result, M1>,
        S2: IntoSystem<In<SelectInput>, Result, M2>,
    {
        let spawn_item = IntoSystem::into_system(spawn_item);
        let on_select = IntoSystem::into_system(on_select);
        Self {
            items: vec![],
            dismissible: true,
            title: None,
            register_spawn_item: Some(Box::new(move |commands| {
                commands.register_system(spawn_item)
            })),
            register_on_select: Some(Box::new(move |commands| {
                commands.register_system(on_select)
            })),
            register_on_dismiss: Some(Box::new(move |commands| {
                commands.register_system(|entities: In<PickerEntities>, mut commands: Commands| {
                    commands.entity(entities.picker).try_despawn();
                    Ok(())
                })
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

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn with_dismissible(mut self, value: bool) -> Self {
        self.dismissible = value;
        self
    }

    pub fn on_dismiss<S, M>(mut self, on_dismiss: S) -> Self
    where
        S: IntoSystem<In<PickerEntities>, Result, M>,
    {
        let on_dismiss = IntoSystem::into_system(on_dismiss);
        self.register_on_dismiss = Some(Box::new(move |commands| {
            commands.register_system(on_dismiss)
        }));

        self
    }
}

impl Picker {
    fn on_replace(mut world: DeferredWorld, ctx: HookContext) {
        let entity = world.entity(ctx.entity);
        let picker = entity.get::<Self>().unwrap();
        let (spawn_item, on_select, on_dismiss) =
            (picker.spawn_item, picker.on_select, picker.on_dismiss);
        let mut commands = world.commands();

        // Clean up after ourselves!
        commands.unregister_system(spawn_item);
        commands.unregister_system(on_select);
        commands.unregister_system(on_dismiss);
    }
}

pub(crate) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        (
            setup_picker,
            process_pickers,
            on_text_edit_submit,
            handle_picker_item_hover,
            scroll_to_picker_item,
        ),
    )
    .add_observer(on_picker_select)
    .add_observer(on_picker_item_activated)
    .add_observer(on_dismiss_activated);
}
