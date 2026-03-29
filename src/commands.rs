use std::any::TypeId;

use bevy::{
    ecs::{
        component::ComponentId,
        reflect::{AppTypeRegistry, ReflectComponent},
    },
    prelude::*,
};

// Re-export the core command framework from the jackdaw_commands crate
pub use jackdaw_commands::{CommandGroup, CommandHistory, EditorCommand};

use jackdaw_bsn::{
    BsnValue, SceneBsnAst,
    sync_to_ast, sync_hierarchy_to_ast, add_component_to_ast, remove_component_from_ast,
    set_bsn_field,
};

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
        sync_to_ast(world, self.entity, self.component_type_id);
    }

    fn undo(&mut self, world: &mut World) {
        apply_reflected_value(
            world,
            self.entity,
            self.component_type_id,
            &self.field_path,
            &*self.old_value,
        );
        sync_to_ast(world, self.entity, self.component_type_id);
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

/// AST-first field edit: mutates the BSN AST and marks the entity dirty so
/// that [`apply_dirty_ast_patches`] propagates the change to ECS.
pub struct SetBsnField {
    pub entity: Entity,
    pub type_path: String,
    pub field_path: String,
    pub old_value: BsnValue,
    pub new_value: BsnValue,
}

impl EditorCommand for SetBsnField {
    fn execute(&mut self, world: &mut World) {
        set_bsn_field_on_entity(world, self.entity, &self.type_path, &self.field_path, self.new_value.clone());
    }

    fn undo(&mut self, world: &mut World) {
        set_bsn_field_on_entity(world, self.entity, &self.type_path, &self.field_path, self.old_value.clone());
    }

    fn description(&self) -> &str {
        "Set BSN field"
    }
}

fn set_bsn_field_on_entity(
    world: &mut World,
    entity: Entity,
    type_path: &str,
    field_path: &str,
    value: BsnValue,
) {
    let patches_entity = jackdaw_bsn::ensure_ast_node(world, entity);

    let registry = world.resource::<AppTypeRegistry>().clone();
    let reg = registry.read();

    // 1. Update the BSN AST (source of truth)
    let mut ast = world.resource_mut::<SceneBsnAst>();
    set_bsn_field(&mut ast, patches_entity, type_path, field_path, value.clone(), &reg);
    drop(ast);

    // 2. Apply via BSN template patch (Bevy's Scene system)
    let Some(registration) = reg.get_with_type_path(type_path) else {
        drop(reg);
        return;
    };
    let template_type_id = registration.type_id();
    let field_path_owned = field_path.to_string();
    drop(reg);

    let scene = bevy::scene2::dynamic_bsn::ErasedTemplatePatch {
        template_type_id,
        app_type_registry: registry.clone(),
        fun: move |reflect: &mut dyn bevy::reflect::PartialReflect, _context: &mut bevy::scene2::ResolveContext| {
            let reg = registry.read();
            let asset_server = None::<&bevy::asset::AssetServer>;
            if let Some(field_tid) = get_field_type_id(reflect, &field_path_owned) {
                if let Some(reflected) = jackdaw_bsn::bsn_value_to_reflect(
                    &value, field_tid, &reg, asset_server,
                ) {
                    if field_path_owned.is_empty() {
                        reflect.apply(&*reflected);
                    } else {
                        // Navigate to the field via struct reflection
                        apply_to_field_path(reflect, &field_path_owned, &*reflected);
                    }
                }
            }
        },
    };

    let asset_server = world.resource::<bevy::asset::AssetServer>().clone();
    let mut patch = bevy::scene2::ScenePatch::load(&asset_server, scene);
    if patch.resolve(&asset_server, &world.resource::<bevy::asset::Assets<bevy::scene2::ScenePatch>>()).is_ok() {
        let _ = patch.apply(&mut world.entity_mut(entity));
    }
}

fn get_field_type_id(reflect: &dyn bevy::reflect::PartialReflect, field_path: &str) -> Option<std::any::TypeId> {
    use bevy::reflect::ReflectRef;
    if field_path.is_empty() {
        return reflect.get_represented_type_info().map(|i| i.type_id());
    }
    // Navigate the field path manually through struct fields
    let parts: Vec<&str> = field_path.split('.').collect();
    let mut current: &dyn bevy::reflect::PartialReflect = reflect;
    for part in &parts {
        match current.reflect_ref() {
            ReflectRef::Struct(s) => {
                current = s.field(part)?;
            }
            _ => return None,
        }
    }
    current.get_represented_type_info().map(|i| i.type_id())
}

fn apply_to_field_path(
    reflect: &mut dyn bevy::reflect::PartialReflect,
    field_path: &str,
    value: &dyn bevy::reflect::PartialReflect,
) {
    use bevy::reflect::ReflectMut;
    let parts: Vec<&str> = field_path.split('.').collect();
    let mut current: &mut dyn bevy::reflect::PartialReflect = reflect;
    for (i, part) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            // Last segment — apply the value
            if let ReflectMut::Struct(s) = current.reflect_mut() {
                if let Some(field) = s.field_mut(part) {
                    field.apply(value);
                }
            }
        } else {
            // Intermediate segment — navigate into
            let ReflectMut::Struct(s) = current.reflect_mut() else { return };
            let Some(next) = s.field_mut(part) else { return };
            current = next;
        }
    }
}

/// AST-first name edit: updates the `BsnPatch::Name` directly (Name is a
/// special patch type, not a struct field).
pub struct SetBsnName {
    pub entity: Entity,
    pub old_name: String,
    pub new_name: String,
}

impl EditorCommand for SetBsnName {
    fn execute(&mut self, world: &mut World) {
        set_bsn_name_on_entity(world, self.entity, &self.new_name);
    }

    fn undo(&mut self, world: &mut World) {
        set_bsn_name_on_entity(world, self.entity, &self.old_name);
    }

    fn description(&self) -> &str {
        "Set entity name"
    }
}

fn set_bsn_name_on_entity(world: &mut World, entity: Entity, name: &str) {
    let patches_entity = jackdaw_bsn::ensure_ast_node(world, entity);

    // 1. Update BSN AST
    let mut ast = world.resource_mut::<SceneBsnAst>();
    if let Some(patches) = ast.get_patches(patches_entity) {
        let patch_ids: Vec<Entity> = patches.0.clone();
        for pe in patch_ids {
            if let Some(jackdaw_bsn::BsnPatch::Name(_)) = ast.get_patch(pe) {
                ast.set_patch(pe, jackdaw_bsn::BsnPatch::Name(name.to_string()));
                drop(ast);
                // 2. Also update ECS Name directly
                world.entity_mut(entity).insert(Name::new(name.to_string()));
                return;
            }
        }
    }
    let patch_entity = ast.world.spawn(jackdaw_bsn::BsnPatch::Name(name.to_string())).id();
    if let Some(patches) = ast.get_patches_mut(patches_entity) {
        patches.0.insert(0, patch_entity);
    }
    drop(ast);
    // 2. Also update ECS Name directly
    world.entity_mut(entity).insert(Name::new(name.to_string()));
}

pub struct SetTransform {
    pub entity: Entity,
    pub old_transform: Transform,
    pub new_transform: Transform,
}

impl EditorCommand for SetTransform {
    fn execute(&mut self, world: &mut World) {
        apply_component_bsn(world, self.entity, &self.new_transform);
    }

    fn undo(&mut self, world: &mut World) {
        apply_component_bsn(world, self.entity, &self.old_transform);
    }

    fn description(&self) -> &str {
        "Set transform"
    }
}

pub fn apply_component_bsn<C: Component + bevy::reflect::Reflect + Clone>(
    world: &mut World,
    entity: Entity,
    value: &C,
) {
    // 1. Update BSN AST (source of truth)
    let patches_entity = jackdaw_bsn::ensure_ast_node(world, entity);
    let registry = world.resource::<AppTypeRegistry>().clone();
    let reg = registry.read();
    let patch = jackdaw_bsn::component_to_bsn_patch(value.as_partial_reflect(), &reg);
    let type_path = reg.get(TypeId::of::<C>())
        .map(|r| r.type_info().type_path().to_string())
        .unwrap_or_default();
    let mut ast = world.resource_mut::<SceneBsnAst>();
    if let Some(existing) = ast.find_patch_by_type_path(patches_entity, &type_path) {
        ast.set_patch(existing, patch);
    } else {
        let pe = ast.world.spawn(patch).id();
        if let Some(patches) = ast.get_patches_mut(patches_entity) {
            patches.0.push(pe);
        }
    }
    drop(ast);
    drop(reg);

    // 2. Apply to ECS (insert overwrites the component)
    world.entity_mut(entity).insert(value.clone());
}

pub struct ReparentEntity {
    pub entity: Entity,
    pub old_parent: Option<Entity>,
    pub new_parent: Option<Entity>,
}

impl EditorCommand for ReparentEntity {
    fn execute(&mut self, world: &mut World) {
        set_parent(world, self.entity, self.new_parent);
        sync_hierarchy_to_ast(world, self.entity, self.new_parent);
    }

    fn undo(&mut self, world: &mut World) {
        set_parent(world, self.entity, self.old_parent);
        sync_hierarchy_to_ast(world, self.entity, self.old_parent);
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
}

pub struct AddComponent {
    pub entity: Entity,
    pub type_id: TypeId,
    pub component_id: ComponentId,
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
        drop(registry);
        add_component_to_ast(world, self.entity, self.type_id);
    }

    fn undo(&mut self, world: &mut World) {
        // Get type path before removing
        let type_path = {
            let registry = world.resource::<AppTypeRegistry>().clone();
            let registry = registry.read();
            registry
                .get(self.type_id)
                .map(|r| r.type_info().type_path().to_string())
        };
        if let Ok(mut entity) = world.get_entity_mut(self.entity) {
            entity.remove_by_id(self.component_id);
        }
        if let Some(type_path) = type_path {
            remove_component_from_ast(world, self.entity, &type_path);
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
    /// Snapshot of the component's value before removal, for undo.
    pub snapshot: Box<dyn PartialReflect>,
}

impl EditorCommand for RemoveComponent {
    fn execute(&mut self, world: &mut World) {
        // Get type path before removing
        let type_path = {
            let registry = world.resource::<AppTypeRegistry>().clone();
            let registry = registry.read();
            registry
                .get(self.type_id)
                .map(|r| r.type_info().type_path().to_string())
        };
        if let Ok(mut entity) = world.get_entity_mut(self.entity) {
            entity.remove_by_id(self.component_id);
        }
        if let Some(type_path) = type_path {
            remove_component_from_ast(world, self.entity, &type_path);
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
        add_component_to_ast(world, self.entity, self.type_id);
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
        jackdaw_bsn::delete_entity_from_ast(world, self.entity);
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
        crate::scene_io::link_entity_tree_to_ast(world, self.entity, self.parent);
    }

    fn description(&self) -> &str {
        &self.label
    }
}

/// Create a `DynamicSceneBuilder` that excludes computed components which become
/// stale when restored (Children references dead mesh entities, visibility flags
/// block rendering).
pub(crate) fn filtered_scene_builder<'w>(world: &'w World, type_registry: &'w bevy::reflect::TypeRegistry) -> DynamicSceneBuilder<'w> {
    DynamicSceneBuilder::from_world(world, type_registry)
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
    let registry = world.resource::<AppTypeRegistry>().clone();
    let registry = registry.read();
    let mut entities = Vec::new();
    collect_entity_ids(world, entity, &mut entities);
    filtered_scene_builder(world, &registry)
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
