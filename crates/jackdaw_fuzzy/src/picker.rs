use std::marker::PhantomData;

use bevy::ecs::lifecycle::HookContext;
use bevy::ecs::system::SystemId;
use bevy::ecs::world::DeferredWorld;
use bevy::prelude::*;
use jackdaw_feathers::text_edit::TextEditValue;

use crate::matcher::{FuzzyItem, FuzzyMatcher, MatchedStr};

pub trait FuzzyPickerItem: FuzzyItem + Send + Sync + 'static {}

impl<T: FuzzyItem + Send + Sync + 'static> FuzzyPickerItem for T {}

type OnSelect<T> = SystemId<(InRef<'static, T>, In<Entity>)>;

#[derive(Component)]
#[component(on_insert)]
#[component(on_replace)]
pub struct FuzzyPicker<T: FuzzyPickerItem> {
    pub matcher: FuzzyMatcher<T>,
    pub input: Entity,
    pub list: Entity,
    spawn_item: SystemId<In<SpawnItemInput>>,
    on_discard: SystemId<In<Entity>>,
    on_select: OnSelect<T>,
    // Functions to register a system in the insert hook
    register_spawn_item:
        Option<Box<dyn FnOnce(&mut Commands) -> SystemId<In<SpawnItemInput>> + Send + Sync>>,
    register_on_discard:
        Option<Box<dyn FnOnce(&mut Commands) -> SystemId<In<Entity>> + Send + Sync>>,
    register_on_select: Option<Box<dyn FnOnce(&mut Commands) -> OnSelect<T> + Send + Sync>>,
}

impl<T: FuzzyPickerItem> FuzzyPicker<T> {
    pub fn new<S1, M1, S2, M2>(input: Entity, list: Entity, spawn_item: S1, on_select: S2) -> Self
    where
        S1: IntoSystem<In<SpawnItemInput>, (), M1>,
        S2: IntoSystem<(InRef<'static, T>, In<Entity>), (), M2>,
    {
        let spawn_item = IntoSystem::into_system(spawn_item);
        let on_select = IntoSystem::into_system(on_select);
        let matcher = FuzzyMatcher::new();
        Self {
            matcher,
            input,
            list,
            spawn_item: SystemId::from_entity(Entity::PLACEHOLDER),
            on_discard: SystemId::from_entity(Entity::PLACEHOLDER),
            on_select: SystemId::from_entity(Entity::PLACEHOLDER),
            register_spawn_item: Some(Box::new(move |commands| {
                commands.register_system(spawn_item)
            })),
            register_on_discard: Some(Box::new(move |commands| {
                commands.register_system(|entity: In<Entity>, mut cmd: Commands| {
                    cmd.entity(*entity).despawn();
                })
            })),
            register_on_select: Some(Box::new(move |commands| {
                commands.register_system(on_select)
            })),
        }
    }

    pub fn on_discard<S, M>(mut self, on_discard: S) -> Self
    where
        S: IntoSystem<In<Entity>, (), M> + Send + Sync + 'static,
    {
        self.register_on_discard = Some(Box::new(move |commands| {
            commands.register_system(on_discard)
        }));

        self
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
        // Utterly unhinged, but lets me store a system inside a component, by @janhohenheim
        let Some(register_spawn_item) = world
            .entity_mut(ctx.entity)
            .get_mut::<Self>()
            .and_then(|mut s| s.register_spawn_item.take())
        else {
            return;
        };

        let Some(register_on_discard) = world
            .entity_mut(ctx.entity)
            .get_mut::<Self>()
            .and_then(|mut s| s.register_on_discard.take())
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
        let on_discard = register_on_discard(&mut commands);
        let on_select = register_on_select(&mut commands);

        let mut entity = world.entity_mut(ctx.entity);
        let mut picker = entity.get_mut::<Self>().unwrap();
        picker.spawn_item = spawn_item;
        picker.on_discard = on_discard;
        picker.on_select = on_select;
    }

    fn on_replace(mut world: DeferredWorld, ctx: HookContext) {
        let entity = world.entity(ctx.entity);
        let picker = entity.get::<Self>().unwrap();
        let (spawn_item, on_discard, on_select) =
            (picker.spawn_item, picker.on_discard, picker.on_select);
        let mut commands = world.commands();
        // Clean up after ourselves!
        commands.unregister_system(spawn_item);
        commands.unregister_system(on_discard);
        commands.unregister_system(on_select);
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct SpawnItemInput {
    pub index: usize,
    pub segments: Box<[MatchedStr]>,
    pub score: u32,
    pub picker: Entity,
    pub input: Entity,
    pub list: Entity,
}

fn process_fuzzy_pickers<T: FuzzyPickerItem>(
    pickers: Query<(Entity, &mut FuzzyPicker<T>)>,
    text_edits: Query<&TextEditValue, Changed<TextEditValue>>,
    mut commands: Commands,
) {
    for (entity, mut picker) in pickers {
        let Ok(input) = text_edits.get(picker.input) else {
            continue;
        };

        picker.matcher.update_pattern(&input.0);

        let input = picker.input;
        let list = picker.list;
        let spawn_item = picker.spawn_item;
        let matches: Vec<_> = picker.matcher.matches().collect();

        commands.entity(list).despawn_children();

        for matched in matches {
            let input = SpawnItemInput {
                index: matched.index,
                segments: matched.segments,
                score: matched.score,
                picker: entity,
                input,
                list,
            };

            commands.run_system_with(spawn_item, input);
        }
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
        app.add_systems(Update, process_fuzzy_pickers::<T>);
    }
}
