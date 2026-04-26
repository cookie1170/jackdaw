use std::marker::PhantomData;

use bevy::ecs::lifecycle::HookContext;
use bevy::ecs::system::SystemId;
use bevy::ecs::world::DeferredWorld;
use bevy::prelude::*;
use bevy_ui_text_input::SubmitText;
use jackdaw_feathers::text_edit::TextEditValue;

use crate::matcher::{FuzzyItem, FuzzyMatcher, Match};

pub trait FuzzyPickerItem: FuzzyItem + Send + Sync + 'static {}

impl<T: FuzzyItem + Send + Sync + 'static> FuzzyPickerItem for T {}

#[derive(Component)]
#[component(on_insert)]
#[component(on_replace)]
pub struct FuzzyPicker<T: FuzzyPickerItem> {
    pub matcher: FuzzyMatcher<T>,
    spawn_item: SystemId<In<SpawnItemInput>>,
    on_select: SystemId<In<SelectInput>>,
    // Functions to register a system in the insert hook
    register_spawn_item:
        Option<Box<dyn FnOnce(&mut Commands) -> SystemId<In<SpawnItemInput>> + Send + Sync>>,
    register_on_select:
        Option<Box<dyn FnOnce(&mut Commands) -> SystemId<In<SelectInput>> + Send + Sync>>,
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

impl<T: FuzzyPickerItem> FuzzyPicker<T> {
    pub fn new<S1, M1, S2, M2>(spawn_item: S1, on_select: S2) -> Self
    where
        S1: IntoSystem<In<SpawnItemInput>, (), M1>,
        S2: IntoSystem<In<SelectInput>, (), M2>,
    {
        let spawn_item = IntoSystem::into_system(spawn_item);
        let on_select = IntoSystem::into_system(on_select);
        let matcher = FuzzyMatcher::new();
        Self {
            matcher,
            spawn_item: SystemId::from_entity(Entity::PLACEHOLDER),
            on_select: SystemId::from_entity(Entity::PLACEHOLDER),
            register_spawn_item: Some(Box::new(move |commands| {
                commands.register_system(spawn_item)
            })),
            register_on_select: Some(Box::new(move |commands| {
                commands.register_system(on_select)
            })),
        }
    }

    pub fn with_items(mut self, items: impl IntoIterator<Item = T>) -> Self {
        self.matcher.push_items(items);
        self
    }

    pub fn with_item(mut self, item: T) -> Self {
        self.matcher.push_item(item);
        self
    }

    /// Gets a reference to the list of items
    pub fn items(&self) -> &[T] {
        self.matcher.items()
    }

    fn on_insert(mut world: DeferredWorld, ctx: HookContext) {
        // Utterly unhinged hack to store a system inside a component, by @janhohenheim
        let Some(register_spawn_item) = world
            .entity_mut(ctx.entity)
            .get_mut::<Self>()
            .and_then(|mut s| s.register_spawn_item.take())
        else {
            return;
        };

        let Some(register_on_select) = world
            .entity_mut(ctx.entity)
            .get_mut::<Self>()
            .and_then(|mut s| s.register_on_select.take())
        else {
            return;
        };

        let mut commands = world.commands();
        let spawn_item = register_spawn_item(&mut commands);
        let on_select = register_on_select(&mut commands);

        let mut entity = world.entity_mut(ctx.entity);
        let mut picker = entity.get_mut::<Self>().unwrap();
        picker.spawn_item = spawn_item;
        picker.on_select = on_select;
    }

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
pub struct FuzzyPickerSelect {
    pub entity: Entity,
    pub index: usize,
}

fn process_fuzzy_pickers<T: FuzzyPickerItem>(
    pickers: Query<(
        Entity,
        &mut FuzzyPicker<T>,
        &WithPickerInput,
        &WithPickerList,
    )>,
    text_edits: Query<&TextEditValue, Changed<TextEditValue>>,
    mut commands: Commands,
) {
    for (entity, mut picker, input_entity, list) in pickers {
        let Ok(input) = text_edits.get(input_entity.0) else {
            continue;
        };

        picker.matcher.update_pattern(&input.0);

        let spawn_item = picker.spawn_item;
        let matches: Vec<_> = picker.matcher.matches().collect();

        commands.entity(list.0).despawn_children();

        for matched in matches {
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

fn on_fuzzy_picker_select<T: FuzzyPickerItem>(
    trigger: On<FuzzyPickerSelect>,
    pickers: Query<(&mut FuzzyPicker<T>, &WithPickerInput, &WithPickerList)>,
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

fn on_text_edit_submit<T: FuzzyPickerItem>(
    mut submit_messages: MessageReader<SubmitText>,
    inputs: Query<&PickerInputOf>,
    child_of: Query<&ChildOf>,
    mut pickers: Query<(Entity, &mut FuzzyPicker<T>)>,
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

        commands.trigger(FuzzyPickerSelect {
            entity: picker_entity,
            index: first.index,
        });
    }
}

pub trait RegisterFuzzyItemAppExt {
    fn register_fuzzy_item<T: FuzzyPickerItem>(&mut self) -> &mut Self;
}

impl RegisterFuzzyItemAppExt for App {
    fn register_fuzzy_item<T: FuzzyPickerItem>(&mut self) -> &mut Self {
        if !self.is_plugin_added::<PickerPlugin<T>>() {
            self.add_plugins(PickerPlugin::<T>::default());
        }

        self
    }
}

pub struct PickerPlugin<T: FuzzyPickerItem> {
    _phantom: PhantomData<T>,
}

impl<T: FuzzyPickerItem> Default for PickerPlugin<T> {
    fn default() -> Self {
        Self {
            _phantom: Default::default(),
        }
    }
}

impl<T: FuzzyPickerItem> Plugin for PickerPlugin<T> {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (process_fuzzy_pickers::<T>, on_text_edit_submit::<T>),
        )
        .add_observer(on_fuzzy_picker_select::<T>);
    }
}
