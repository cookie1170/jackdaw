use std::any::TypeId;

use bevy::{
    ecs::{
        component::ComponentId,
        reflect::{AppTypeRegistry, ReflectComponent},
    },
    prelude::*,
};
use serde::de::DeserializeSeed;

// Re-export the core command framework from the jackdaw_commands crate
pub use jackdaw_commands::{CommandGroup, CommandHistory, EditorCommand};

use crate::EditorEntity;
use crate::selection::{Selected, Selection};

pub struct CommandHistoryPlugin;

impl Plugin for CommandHistoryPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(CommandHistory::default()).add_systems(
            Update,
            handle_undo_redo_keys.in_set(crate::EditorInteraction),
        );
    }
}

pub struct SetComponentField {
    pub entity: Entity,
    pub component_type_id: TypeId,
    pub field_path: String,
    pub old_value: Box<dyn PartialReflect>,
    pub new_value: Box<dyn PartialReflect>,
}

impl EditorCommand for SetComponentField {
    fn execute(&mut self, world: &mut World) {
        apply_reflected_value(
            world,
            self.entity,
            self.component_type_id,
            &self.field_path,
            &*self.new_value,
        );
    }

    fn undo(&mut self, world: &mut World) {
        apply_reflected_value(
            world,
            self.entity,
            self.component_type_id,
            &self.field_path,
            &*self.old_value,
        );
    }

    fn description(&self) -> &str {
        "Set component field"
    }
}

fn apply_reflected_value(
    world: &mut World,
    entity: Entity,
    component_type_id: TypeId,
    field_path: &str,
    value: &dyn PartialReflect,
) {
    let registry = world.resource::<AppTypeRegistry>().clone();
    let registry = registry.read();

    let Some(registration) = registry.get(component_type_id) else {
        return;
    };
    let Some(reflect_component) = registration.data::<ReflectComponent>() else {
        return;
    };

    let Some(reflected) = reflect_component.reflect_mut(world.entity_mut(entity)) else {
        return;
    };

    if field_path.is_empty() {
        // Apply to the entire component (e.g. a top-level enum component)
        reflected.into_inner().apply(value);
    } else {
        let Ok(field) = reflected.into_inner().reflect_path_mut(field_path) else {
            return;
        };
        field.apply(value);
    }
}

pub struct SetTransform {
    pub entity: Entity,
    pub old_transform: Transform,
    pub new_transform: Transform,
}

impl EditorCommand for SetTransform {
    fn execute(&mut self, world: &mut World) {
        if let Some(mut transform) = world.get_mut::<Transform>(self.entity) {
            *transform = self.new_transform;
        }
        sync_component_to_ast::<Transform>(
            world,
            self.entity,
            "bevy_transform::components::transform::Transform",
            &self.new_transform,
        );
    }

    fn undo(&mut self, world: &mut World) {
        if let Some(mut transform) = world.get_mut::<Transform>(self.entity) {
            *transform = self.old_transform;
        }
        sync_component_to_ast::<Transform>(
            world,
            self.entity,
            "bevy_transform::components::transform::Transform",
            &self.old_transform,
        );
    }

    fn description(&self) -> &str {
        "Set transform"
    }
}

pub struct ReparentEntity {
    pub entity: Entity,
    pub old_parent: Option<Entity>,
    pub new_parent: Option<Entity>,
}

impl EditorCommand for ReparentEntity {
    fn execute(&mut self, world: &mut World) {
        set_parent(world, self.entity, self.new_parent);
    }

    fn undo(&mut self, world: &mut World) {
        set_parent(world, self.entity, self.old_parent);
    }

    fn description(&self) -> &str {
        "Reparent entity"
    }
}

fn set_parent(world: &mut World, entity: Entity, parent: Option<Entity>) {
    match parent {
        Some(p) => {
            world.entity_mut(entity).insert(ChildOf(p));
        }
        None => {
            world.entity_mut(entity).remove::<ChildOf>();
        }
    }
    // Update AST parent
    let mut ast = world.resource_mut::<jackdaw_jsn::SceneJsnAst>();
    let parent_idx = parent.and_then(|p| ast.ecs_to_jsn.get(&p).copied());
    if let Some(node) = ast.node_for_entity_mut(entity) {
        node.parent = parent_idx;
    }
}

pub struct AddComponent {
    pub entity: Entity,
    pub type_id: TypeId,
    pub component_id: ComponentId,
    pub type_path: String,
}

impl EditorCommand for AddComponent {
    fn execute(&mut self, world: &mut World) {
        let registry = world.resource::<AppTypeRegistry>().clone();
        let registry = registry.read();

        let Some(registration) = registry.get(self.type_id) else {
            return;
        };

        // Create default value
        let Some(reflect_default) = registration.data::<ReflectDefault>() else {
            warn!("No ReflectDefault for component — cannot add");
            return;
        };
        let default_value = reflect_default.default();
        let Some(reflect_component) = registration.data::<ReflectComponent>() else {
            return;
        };

        reflect_component.insert(
            &mut world.entity_mut(self.entity),
            default_value.as_partial_reflect(),
            &registry,
        );

        // Sync to AST
        let serializer =
            bevy::reflect::serde::TypedReflectSerializer::new(default_value.as_ref(), &registry);
        if let Ok(json_value) = serde_json::to_value(&serializer) {
            drop(registry);
            world
                .resource_mut::<jackdaw_jsn::SceneJsnAst>()
                .set_component(self.entity, &self.type_path, json_value);
        }
    }

    fn undo(&mut self, world: &mut World) {
        if let Ok(mut entity) = world.get_entity_mut(self.entity) {
            entity.remove_by_id(self.component_id);
        }
        // Remove from AST
        if let Some(node) = world
            .resource_mut::<jackdaw_jsn::SceneJsnAst>()
            .node_for_entity_mut(self.entity)
        {
            node.components.remove(&self.type_path);
        }
    }

    fn description(&self) -> &str {
        "Add component"
    }
}

pub struct RemoveComponent {
    pub entity: Entity,
    pub type_id: TypeId,
    pub component_id: ComponentId,
    pub type_path: String,
    /// Snapshot of the component's value before removal, for undo.
    pub snapshot: Box<dyn PartialReflect>,
    /// AST snapshot for undo.
    pub ast_snapshot: Option<serde_json::Value>,
}

impl EditorCommand for RemoveComponent {
    fn execute(&mut self, world: &mut World) {
        // Snapshot from AST before removal
        self.ast_snapshot = world
            .resource::<jackdaw_jsn::SceneJsnAst>()
            .get_component(self.entity, &self.type_path)
            .cloned();
        if let Ok(mut entity) = world.get_entity_mut(self.entity) {
            entity.remove_by_id(self.component_id);
        }
        // Remove from AST
        if let Some(node) = world
            .resource_mut::<jackdaw_jsn::SceneJsnAst>()
            .node_for_entity_mut(self.entity)
        {
            node.components.remove(&self.type_path);
        }
    }

    fn undo(&mut self, world: &mut World) {
        let registry = world.resource::<AppTypeRegistry>().clone();
        let registry = registry.read();

        let Some(registration) = registry.get(self.type_id) else {
            return;
        };
        let Some(reflect_component) = registration.data::<ReflectComponent>() else {
            return;
        };

        reflect_component.insert(
            &mut world.entity_mut(self.entity),
            &*self.snapshot,
            &registry,
        );
        drop(registry);

        // Restore AST snapshot
        if let Some(json_value) = self.ast_snapshot.take() {
            world
                .resource_mut::<jackdaw_jsn::SceneJsnAst>()
                .set_component(self.entity, &self.type_path, json_value);
        }
    }

    fn description(&self) -> &str {
        "Remove component"
    }
}

pub struct SpawnEntity {
    /// The entity that was spawned (set after first execute).
    pub spawned: Option<Entity>,
    /// Builder function that spawns the entity and returns its Entity id.
    pub spawn_fn: Box<dyn Fn(&mut World) -> Entity + Send + Sync>,
    pub label: String,
}

impl EditorCommand for SpawnEntity {
    fn execute(&mut self, world: &mut World) {
        let _entity = (self.spawn_fn)(world);
    }

    fn undo(&mut self, _world: &mut World) {
        // TODO: Track spawned entity for despawn on undo
    }

    fn description(&self) -> &str {
        &self.label
    }
}

pub struct DespawnEntity {
    pub entity: Entity,
    pub scene_snapshot: DynamicScene,
    pub parent: Option<Entity>,
    pub label: String,
}

impl DespawnEntity {
    pub fn from_world(world: &World, entity: Entity) -> Self {
        let parent = world.get::<ChildOf>(entity).map(|c| c.0);
        let scene = snapshot_entity(world, entity);
        Self {
            entity,
            scene_snapshot: scene,
            parent,
            label: format!("Despawn entity {entity}"),
        }
    }
}

impl EditorCommand for DespawnEntity {
    fn execute(&mut self, world: &mut World) {
        deselect_entities(world, &[self.entity]);
        world
            .resource_mut::<jackdaw_jsn::SceneJsnAst>()
            .remove_node(self.entity);
        if let Ok(entity_mut) = world.get_entity_mut(self.entity) {
            entity_mut.despawn();
        }
    }

    fn undo(&mut self, world: &mut World) {
        // Re-build the scene from scratch and write it back
        let scene = snapshot_rebuild(&self.scene_snapshot);
        let mut entity_map = bevy::ecs::entity::hash_map::EntityHashMap::default();
        let _ = scene.write_to_world(world, &mut entity_map);
        if let Some(&new_id) = entity_map.get(&self.entity) {
            self.entity = new_id;
        }
        crate::scene_io::register_entity_in_ast(world, self.entity);
    }

    fn description(&self) -> &str {
        &self.label
    }
}

/// Create a `DynamicSceneBuilder` that excludes computed components which become
/// stale when restored (Children references dead mesh entities, visibility flags
/// block rendering).
pub(crate) fn filtered_scene_builder(world: &World) -> DynamicSceneBuilder<'_> {
    DynamicSceneBuilder::from_world(world)
        .deny_component::<Children>()
        .deny_component::<GlobalTransform>()
        .deny_component::<InheritedVisibility>()
        .deny_component::<ViewVisibility>()
}

/// Deselect the given entities: remove the `Selected` component and purge them
/// from the `Selection` resource.  Must be called **before** despawning so that
/// observers can clean up tree-row UI while the entities still exist.
pub(crate) fn deselect_entities(world: &mut World, entities: &[Entity]) {
    for &entity in entities {
        if let Ok(mut ec) = world.get_entity_mut(entity) {
            ec.remove::<Selected>();
        }
    }
    let mut selection = world.resource_mut::<Selection>();
    selection.entities.retain(|e| !entities.contains(e));
}

/// Create a DynamicScene snapshot of a single entity and all its descendants.
pub(crate) fn snapshot_entity(world: &World, entity: Entity) -> DynamicScene {
    let mut entities = Vec::new();
    collect_entity_ids(world, entity, &mut entities);
    filtered_scene_builder(world)
        .extract_entities(entities.into_iter())
        .build()
}

pub(crate) fn collect_entity_ids(world: &World, entity: Entity, out: &mut Vec<Entity>) {
    out.push(entity);
    if let Some(children) = world.get::<Children>(entity) {
        for child in children.iter() {
            if world.get::<EditorEntity>(child).is_none() {
                collect_entity_ids(world, child, out);
            }
        }
    }
}

/// Rebuild a DynamicScene by copying its entity data (since DynamicScene doesn't impl Clone).
pub(crate) fn snapshot_rebuild(scene: &DynamicScene) -> DynamicScene {
    DynamicScene {
        resources: scene.resources.iter().map(|r| r.to_dynamic()).collect(),
        entities: scene
            .entities
            .iter()
            .map(|e| bevy::scene::DynamicEntity {
                entity: e.entity,
                components: e.components.iter().map(|c| c.to_dynamic()).collect(),
            })
            .collect(),
    }
}

fn handle_undo_redo_keys(world: &mut World) {
    let keyboard = world.resource::<ButtonInput<KeyCode>>();
    let keybinds = world.resource::<crate::keybinds::KeybindRegistry>();
    let undo = keybinds.just_pressed(crate::keybinds::EditorAction::Undo, keyboard);
    let redo = keybinds.just_pressed(crate::keybinds::EditorAction::Redo, keyboard);

    if !undo && !redo {
        return;
    }

    let mut history = world.resource_mut::<CommandHistory>();
    let command = if redo {
        history.redo_stack.pop()
    } else {
        history.undo_stack.pop()
    };

    if let Some(mut command) = command {
        if redo {
            command.execute(world);
            world
                .resource_mut::<CommandHistory>()
                .undo_stack
                .push(command);
        } else {
            command.undo(world);
            world
                .resource_mut::<CommandHistory>()
                .redo_stack
                .push(command);
        }
    }
}

// ─────────────────────────────────── JSN-First Commands ───────────────────────────────────

pub struct SetJsnField {
    pub entity: Entity,
    pub type_path: String,
    pub field_path: String,
    pub old_value: serde_json::Value,
    pub new_value: serde_json::Value,
}

impl EditorCommand for SetJsnField {
    fn execute(&mut self, world: &mut World) {
        {
            let registry = world.resource::<AppTypeRegistry>().clone();
            let registry = registry.read();
            world
                .resource_mut::<jackdaw_jsn::SceneJsnAst>()
                .set_component_field(
                    self.entity,
                    &self.type_path,
                    &self.field_path,
                    self.new_value.clone(),
                    &registry,
                );
        }
        apply_jsn_field_to_ecs(
            world,
            self.entity,
            &self.type_path,
            &self.field_path,
            &self.new_value,
        );
    }

    fn undo(&mut self, world: &mut World) {
        {
            let registry = world.resource::<AppTypeRegistry>().clone();
            let registry = registry.read();
            world
                .resource_mut::<jackdaw_jsn::SceneJsnAst>()
                .set_component_field(
                    self.entity,
                    &self.type_path,
                    &self.field_path,
                    self.old_value.clone(),
                    &registry,
                );
        }
        apply_jsn_field_to_ecs(
            world,
            self.entity,
            &self.type_path,
            &self.field_path,
            &self.old_value,
        );
    }

    fn description(&self) -> &str {
        "Set component field"
    }
}

/// Apply a JSON value to an ECS component — either full component replacement
/// (empty field_path) or field-level update.
fn apply_jsn_field_to_ecs(
    world: &mut World,
    entity: Entity,
    type_path: &str,
    field_path: &str,
    value: &serde_json::Value,
) {
    let registry = world.resource::<AppTypeRegistry>().clone();
    let registry = registry.read();

    let Some(registration) = registry.get_with_type_path(type_path) else {
        return;
    };
    let Some(reflect_component) = registration.data::<ReflectComponent>() else {
        return;
    };

    if field_path.is_empty() {
        // Full component replacement via TypedReflectDeserializer
        let deserializer =
            bevy::reflect::serde::TypedReflectDeserializer::new(registration, &registry);
        if let Ok(reflected) = deserializer.deserialize(value) {
            reflect_component.apply(world.entity_mut(entity), reflected.as_ref());
        }
    } else {
        // Field-level update via reflect_path_mut
        let Some(reflected) = reflect_component.reflect_mut(world.entity_mut(entity)) else {
            return;
        };
        if let Ok(field) = reflected.into_inner().reflect_path_mut(field_path) {
            apply_json_to_reflect(field, value);
        }
    }
}

/// Convert a serde_json::Value into the matching reflect primitive and apply it.
fn apply_json_to_reflect(field: &mut dyn bevy::reflect::PartialReflect, value: &serde_json::Value) {
    match value {
        serde_json::Value::Number(n) => {
            if let Some(f) = field.try_downcast_mut::<f32>() {
                *f = n.as_f64().unwrap_or_default() as f32;
            } else if let Some(f) = field.try_downcast_mut::<f64>() {
                *f = n.as_f64().unwrap_or_default();
            } else if let Some(i) = field.try_downcast_mut::<i32>() {
                *i = n.as_i64().unwrap_or_default() as i32;
            } else if let Some(i) = field.try_downcast_mut::<u32>() {
                *i = n.as_u64().unwrap_or_default() as u32;
            } else if let Some(i) = field.try_downcast_mut::<usize>() {
                *i = n.as_u64().unwrap_or_default() as usize;
            } else if let Some(i) = field.try_downcast_mut::<i8>() {
                *i = n.as_i64().unwrap_or_default() as i8;
            } else if let Some(i) = field.try_downcast_mut::<i16>() {
                *i = n.as_i64().unwrap_or_default() as i16;
            } else if let Some(i) = field.try_downcast_mut::<i64>() {
                *i = n.as_i64().unwrap_or_default();
            } else if let Some(i) = field.try_downcast_mut::<u8>() {
                *i = n.as_u64().unwrap_or_default() as u8;
            } else if let Some(i) = field.try_downcast_mut::<u16>() {
                *i = n.as_u64().unwrap_or_default() as u16;
            } else if let Some(i) = field.try_downcast_mut::<u64>() {
                *i = n.as_u64().unwrap_or_default();
            }
        }
        serde_json::Value::Bool(b) => {
            if let Some(f) = field.try_downcast_mut::<bool>() {
                *f = *b;
            }
        }
        serde_json::Value::String(s) => {
            if let Some(f) = field.try_downcast_mut::<String>() {
                *f = s.clone();
            }
        }
        _ => {}
    }
}

/// Serialize a component to JSON and store it in the AST.
pub fn sync_component_to_ast<T: bevy::reflect::Reflect>(
    world: &mut World,
    entity: Entity,
    type_path: &str,
    value: &T,
) {
    let registry = world.resource::<AppTypeRegistry>().clone();
    let registry = registry.read();
    let processor = crate::scene_io::AstSerializerProcessor;
    let serializer =
        bevy::reflect::serde::TypedReflectSerializer::with_processor(value, &registry, &processor);
    if let Ok(json_value) = serde_json::to_value(&serializer) {
        drop(registry);
        world
            .resource_mut::<jackdaw_jsn::SceneJsnAst>()
            .set_component(entity, type_path, json_value);
    }
}
