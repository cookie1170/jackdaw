//! Sync helpers: ECS → BSN AST.
//!
//! After modifying an ECS component, call `sync_to_ast` to update the
//! corresponding BSN AST node. This is a one-line addition to existing
//! editor commands.

use std::any::TypeId;

use bevy::ecs::reflect::{AppTypeRegistry, ReflectComponent};
use bevy::prelude::*;

use crate::{AstNodeRef, SceneBsnAst, component_to_bsn_patch};

/// After modifying an ECS component, sync its current value to the BSN AST.
///
/// This reads the component via reflection, converts it to a BSN patch, and
/// updates the AST node. Should be called after every ECS component mutation
/// that needs to persist to the scene file.
pub fn sync_to_ast(world: &mut World, entity: Entity, component_type_id: TypeId) {
    // Get the AST node reference
    let Some(ast_ref) = world.get::<AstNodeRef>(entity) else {
        bevy::log::warn!("sync_to_ast: entity {entity:?} has no AstNodeRef");
        return;
    };
    let patches_entity = ast_ref.patches_entity;

    // Read the component via reflection
    let registry = world.resource::<AppTypeRegistry>().clone();
    let registry = registry.read();

    let Some(registration) = registry.get(component_type_id) else {
        bevy::log::warn!("sync_to_ast: no registration for type");
        return;
    };
    let Some(reflect_component) = registration.data::<ReflectComponent>() else {
        bevy::log::warn!("sync_to_ast: no ReflectComponent");
        return;
    };
    let Some(reflected) = reflect_component.reflect(world.entity(entity)) else {
        bevy::log::warn!("sync_to_ast: component not found on entity {entity:?}");
        return;
    };

    // Convert to BSN patch
    let patch = component_to_bsn_patch(reflected, &registry);

    let type_path = reflected
        .get_represented_type_info()
        .map(|info| info.type_path().to_string())
        .unwrap_or_default();

    // Log the patch type for debugging enum issues
    let patch_desc = match &patch {
        crate::BsnPatch::Type(tp) => format!("Type({})", tp),
        crate::BsnPatch::Struct(d) => format!("Struct({}, {} fields)", d.type_path, d.fields.0.len()),
        crate::BsnPatch::TupleStruct(d) => format!("TupleStruct({})", d.type_path),
        _ => "other".to_string(),
    };
    bevy::log::info!("sync_to_ast: entity {entity:?}, type_path={type_path}, patch={patch_desc}");

    drop(registry);

    // Update the AST
    let mut ast = world.resource_mut::<SceneBsnAst>();

    // Find or create the patch for this type
    if let Some(existing) = ast.find_patch_by_type_path(patches_entity, &type_path) {
        bevy::log::info!("  replacing existing patch");
        ast.set_patch(existing, patch);
    } else {
        bevy::log::info!("  creating new patch (no existing found)");
        let patch_entity = ast.world.spawn(patch).id();
        if let Some(patches) = ast.get_patches_mut(patches_entity) {
            patches.0.push(patch_entity);
        }
    }
}

/// Ensure an entity has an AST node. Creates one on the fly if missing,
/// reflecting all current components into BSN patches. Returns the
/// `patches_entity` in the AST world.
pub fn ensure_ast_node(world: &mut World, entity: Entity) -> Entity {
    if let Some(ast_ref) = world.get::<AstNodeRef>(entity) {
        return ast_ref.patches_entity;
    }
    create_entity_in_ast(world, entity, None);
    world
        .get::<AstNodeRef>(entity)
        .expect("create_entity_in_ast should have inserted AstNodeRef")
        .patches_entity
}

/// After adding a component to an ECS entity, add a corresponding BSN patch.
pub fn add_component_to_ast(world: &mut World, entity: Entity, component_type_id: TypeId) {
    // Same as sync_to_ast — it creates the patch if it doesn't exist
    sync_to_ast(world, entity, component_type_id);
}

/// After removing a component from an ECS entity, remove its BSN patch.
pub fn remove_component_from_ast(world: &mut World, entity: Entity, type_path: &str) {
    let Some(ast_ref) = world.get::<AstNodeRef>(entity) else {
        return;
    };
    let patches_entity = ast_ref.patches_entity;

    let mut ast = world.resource_mut::<SceneBsnAst>();
    let Some(existing) = ast.find_patch_by_type_path(patches_entity, type_path) else {
        return;
    };

    // Remove from patches list
    if let Some(patches) = ast.get_patches_mut(patches_entity) {
        patches.0.retain(|&e| e != existing);
    }

    // Despawn the patch entity from the AST world
    if let Ok(entity_mut) = ast.world.get_entity_mut(existing) {
        entity_mut.despawn();
    }
}

/// Create an AST node for a new ECS entity and link them.
///
/// Inserts the node into the parent's `Children` patch (or roots if no parent).
pub fn create_entity_in_ast(world: &mut World, entity: Entity, parent: Option<Entity>) {
    let name = world.get::<Name>(entity).map(|n| n.to_string());

    let mut initial_patches = Vec::new();
    if let Some(name) = name {
        initial_patches.push(crate::BsnPatch::Name(name));
    }

    let mut ast = world.resource_mut::<SceneBsnAst>();
    let ast_entity = ast.create_entity_node(initial_patches);

    let parent_ast = parent.and_then(|p| ast.ast_for(p));
    if let Some(parent_ast) = parent_ast {
        ast.add_child_to_ast(parent_ast, ast_entity);
    } else {
        ast.add_to_roots(ast_entity);
    }
    ast.link(entity, ast_entity);
    drop(ast);

    world
        .entity_mut(entity)
        .insert(crate::AstNodeRef { patches_entity: ast_entity });
}

/// Remove an ECS entity's AST node and unlink it.
///
/// Removes the node from its parent's `Children` patch (or roots) and despawns
/// the AST entities recursively.
pub fn delete_entity_from_ast(world: &mut World, entity: Entity) {
    let Some(ast_ref) = world.get::<crate::AstNodeRef>(entity) else {
        return;
    };
    let node_ast = ast_ref.patches_entity;

    let mut ast = world.resource_mut::<SceneBsnAst>();

    // Find parent and remove from it.
    let parent_ast = find_ast_parent(&ast, node_ast);
    if let Some(parent_ast) = parent_ast {
        ast.remove_child_from_ast(parent_ast, node_ast);
    } else {
        ast.remove_from_roots(node_ast);
    }

    // Recursively despawn AST nodes.
    despawn_ast_recursive(&mut ast, node_ast);

    // Unlink ECS → AST.
    ast.unlink(entity);
}

/// Recursively despawn an AST node and all its child AST nodes.
fn despawn_ast_recursive(ast: &mut SceneBsnAst, node: Entity) {
    // Collect children first.
    let children: Vec<Entity> = if let Some(patches) = ast.get_patches(node) {
        let mut children = Vec::new();
        for &pe in &patches.0 {
            if let Some(crate::BsnPatch::Children(child_list)) = ast.get_patch(pe) {
                children.extend(child_list.iter().copied());
            }
        }
        children
    } else {
        Vec::new()
    };

    for child in children {
        despawn_ast_recursive(ast, child);
    }

    // Despawn patch entities, then the node itself.
    if let Some(patches) = ast.get_patches(node) {
        let patch_ids: Vec<Entity> = patches.0.clone();
        for pe in patch_ids {
            if let Ok(em) = ast.world.get_entity_mut(pe) {
                em.despawn();
            }
        }
    }
    if let Ok(em) = ast.world.get_entity_mut(node) {
        em.despawn();
    }
}

/// After reparenting an ECS entity, move its AST node to the new parent's
/// Children block.
pub fn sync_hierarchy_to_ast(world: &mut World, entity: Entity, new_parent: Option<Entity>) {
    let Some(ast_ref) = world.get::<AstNodeRef>(entity) else {
        return;
    };
    let node_ast = ast_ref.patches_entity;

    let parent_ast = new_parent.and_then(|p| {
        world
            .get::<AstNodeRef>(p)
            .map(|r| r.patches_entity)
    });

    // Determine old parent AST
    // We need to find which AST node currently contains this node as a child.
    // For now, search roots and all Children patches.
    let mut ast = world.resource_mut::<SceneBsnAst>();

    let old_parent_ast = find_ast_parent(&ast, node_ast);

    ast.move_to_parent(node_ast, old_parent_ast, parent_ast);
}

/// Find which AST entity is the parent of `child_ast` (contains it in a
/// Children patch). Returns None if child is a root.
fn find_ast_parent(ast: &SceneBsnAst, child_ast: Entity) -> Option<Entity> {
    // Check roots
    if ast.roots.contains(&child_ast) {
        return None;
    }

    // Search all patches entities for a Children patch containing child_ast
    for &root in &ast.roots {
        if let Some(parent) = find_parent_recursive(ast, root, child_ast) {
            return Some(parent);
        }
    }

    None
}

fn find_parent_recursive(
    ast: &SceneBsnAst,
    current: Entity,
    target: Entity,
) -> Option<Entity> {
    let Some(patches) = ast.get_patches(current) else {
        return None;
    };

    for &patch_entity in &patches.0 {
        if let Some(patch) = ast.get_patch(patch_entity) {
            if let crate::BsnPatch::Children(children) = patch {
                // Check if target is a direct child
                if children.contains(&target) {
                    return Some(current);
                }
                // Recurse into children
                for &child in children {
                    if let Some(parent) = find_parent_recursive(ast, child, target) {
                        return Some(parent);
                    }
                }
            }
        }
    }

    None
}
