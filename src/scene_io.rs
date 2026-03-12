use std::any::TypeId;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fmt::{self, Formatter};
use std::path::{Path, PathBuf};
use std::result::Result;

use bevy::asset::{ReflectAsset, ReflectHandle, UntypedAssetId};
use bevy::reflect::serde::{
    ReflectDeserializerProcessor, ReflectSerializerProcessor,
};
use bevy::reflect::{TypeRegistration, TypeRegistry};
use bevy::{
    asset::AssetPath,
    ecs::reflect::AppTypeRegistry,
    prelude::*,
    reflect::serde::{TypedReflectDeserializer, TypedReflectSerializer},
    tasks::{AsyncComputeTaskPool, IoTaskPool, Task, futures_lite::future},
    window::{PrimaryWindow, RawHandleWrapper},
};
use jackdaw_jsn::format::{JsnAssets, JsnEntity, JsnHeader, JsnMetadata, JsnScene};
use rfd::{AsyncFileDialog, FileHandle};
use serde::de::{DeserializeSeed, Visitor};
use serde::{Deserializer, Serializer};

use crate::brush::BrushMaterialPalette;
use crate::{EditorEntity, EditorHidden, NonSerializable};
use jackdaw_jsn::Brush;

/// Component type path prefixes that should never be saved (runtime-only / internal).
const SKIP_COMPONENT_PREFIXES: &[&str] = &[
    "bevy_render::",
    "bevy_picking::",
    "bevy_window::",
    "bevy_ecs::observer::",
    "bevy_camera::primitives::",
    "bevy_camera::visibility::",
];

/// Specific component type paths that should never be saved.
const SKIP_COMPONENT_PATHS: &[&str] = &[
    "bevy_transform::components::transform::TransformTreeChanged",
    "bevy_light::cascade::Cascades",
];

pub fn should_skip_component(type_path: &str) -> bool {
    if type_path.starts_with("jackdaw::") {
        return true;
    }
    for prefix in SKIP_COMPONENT_PREFIXES {
        if type_path.starts_with(prefix) {
            return true;
        }
    }
    SKIP_COMPONENT_PATHS.contains(&type_path)
}

pub struct SceneIoPlugin;

impl Plugin for SceneIoPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SceneFilePath>().add_systems(
            Update,
            (handle_scene_io_keys, poll_scene_dialog).run_if(in_state(crate::AppState::Editor)),
        );
    }
}

#[derive(Resource)]
enum SceneDialogTask {
    Save(Task<Option<FileHandle>>),
    Load(Task<Option<FileHandle>>),
}

/// Stores the currently active scene file path and metadata.
#[derive(Resource, Default)]
pub struct SceneFilePath {
    pub path: Option<String>,
    pub metadata: JsnMetadata,
    pub last_directory: Option<PathBuf>,
}

fn get_window_handle(world: &mut World) -> Option<RawHandleWrapper> {
    world
        .query_filtered::<&RawHandleWrapper, With<PrimaryWindow>>()
        .single(world)
        .ok()
        .cloned()
}

fn spawn_save_dialog(world: &mut World) {
    let raw_handle = get_window_handle(world);
    let last_dir = world.resource::<SceneFilePath>().last_directory.clone();

    let mut dialog = AsyncFileDialog::new()
        .add_filter("JSN Scene", &["jsn"])
        .set_file_name("scene.jsn");

    if let Some(dir) = &last_dir {
        dialog = dialog.set_directory(dir);
    }
    if let Some(ref rh) = raw_handle {
        // SAFETY: called on the main thread during an exclusive system
        let handle = unsafe { rh.get_handle() };
        dialog = dialog.set_parent(&handle);
    }

    let task = AsyncComputeTaskPool::get().spawn(async move { dialog.save_file().await });
    world.insert_resource(SceneDialogTask::Save(task));
}

fn spawn_open_dialog(world: &mut World) {
    let raw_handle = get_window_handle(world);
    let last_dir = world.resource::<SceneFilePath>().last_directory.clone();

    let mut dialog = AsyncFileDialog::new()
        .add_filter("JSN Scene", &["jsn"])
        .add_filter("Legacy Scene", &["scene.json"]);

    if let Some(dir) = &last_dir {
        dialog = dialog.set_directory(dir);
    }
    if let Some(ref rh) = raw_handle {
        // SAFETY: called on the main thread during an exclusive system
        let handle = unsafe { rh.get_handle() };
        dialog = dialog.set_parent(&handle);
    }

    let task = AsyncComputeTaskPool::get().spawn(async move { dialog.pick_file().await });
    world.insert_resource(SceneDialogTask::Load(task));
}

pub fn save_scene(world: &mut World) {
    // If no path is set yet, delegate to Save As
    let has_path = world.resource::<SceneFilePath>().path.is_some();
    if !has_path {
        save_scene_as(world);
        return;
    }

    save_scene_inner(world);
}

pub fn save_scene_as(world: &mut World) {
    if world.contains_resource::<SceneDialogTask>() {
        return; // Dialog already open
    }
    spawn_save_dialog(world);
}

fn save_scene_inner(world: &mut World) {
    let scene_file_path = world.resource::<SceneFilePath>();
    let parent_path: Cow<'_, Path> = match scene_file_path
        .path
        .as_ref()
        .and_then(|p| Path::new(p).parent())
    {
        Some(parent_path) => Cow::Owned(parent_path.to_path_buf()),
        None => Cow::Owned(env::current_dir().expect("Couldn't access the current directory")),
    };

    // Pre-compute entity lists while we have &mut World
    let editor_set = collect_editor_entities(world);
    let scene_entities = collect_scene_entities_from_set(world, &editor_set);

    // Resolve default brush face materials to the palette material so they serialize as inline assets
    let default_faces = resolve_default_brush_materials(world, &scene_entities);

    let registry = world.resource::<AppTypeRegistry>().clone();
    let registry_guard = registry.read();

    // --- Phase 1: Collect inline assets from all scene entity components ---
    let (inline_assets, inline_asset_data) =
        collect_inline_assets(world, &registry_guard, &parent_path, &scene_entities);

    // --- Phase 2: Build entity list and serialize ---
    let entities = build_scene_snapshot(world, &registry_guard, &parent_path, &inline_assets, &scene_entities);

    // --- Phase 3: Serialize inline asset data into JsnAssets ---
    let assets = JsnAssets(inline_asset_data);

    drop(registry_guard);

    // Restore Handle::default() on faces that were temporarily resolved
    restore_default_brush_materials(world, &default_faces);

    // Build metadata
    let now = chrono_now();
    let scene_path_res = world.resource::<SceneFilePath>();
    let mut metadata = scene_path_res.metadata.clone();
    metadata.modified = now.clone();
    if metadata.created.is_empty() {
        metadata.created = now;
    }
    if metadata.name.is_empty() {
        metadata.name = "Untitled".to_string();
    }

    let jsn = JsnScene {
        jsn: JsnHeader::default(),
        metadata: metadata.clone(),
        assets,
        editor: None,
        scene: entities,
    };

    let json = match serde_json::to_string_pretty(&jsn) {
        Ok(json) => json,
        Err(err) => {
            warn!("Failed to serialize JSN: {err}");
            return;
        }
    };

    let path = {
        let scene_path = world.resource::<SceneFilePath>();
        scene_path
            .path
            .clone()
            .expect("save_scene_inner called without a path set")
    };

    // Save metadata back
    let mut scene_path = world.resource_mut::<SceneFilePath>();
    scene_path.metadata = metadata;

    // Write to disk on the IO task pool
    let path_clone = path.clone();
    IoTaskPool::get()
        .spawn(async move {
            match std::fs::write(&path_clone, &json) {
                Ok(()) => info!("Scene saved to {path_clone}"),
                Err(err) => warn!("Failed to write scene file: {err}"),
            }
        })
        .detach();
}

pub fn load_scene(world: &mut World) {
    if world.contains_resource::<SceneDialogTask>() {
        return; // Dialog already open
    }
    spawn_open_dialog(world);
}

// ─────────────────────────────────── Serializer Processor ───────────────────────────────────

struct JsnSerializerProcessor<'a> {
    parent_path: Cow<'a, Path>,
    /// Maps runtime asset IDs (no path) to inline `#Name` references.
    inline_assets: &'a HashMap<UntypedAssetId, String>,
    /// Maps scene entities to their index in the entity array.
    entity_to_index: &'a HashMap<Entity, usize>,
}

impl<'a> ReflectSerializerProcessor for JsnSerializerProcessor<'a> {
    fn try_serialize<S>(
        &self,
        value: &dyn PartialReflect,
        registry: &TypeRegistry,
        serializer: S,
    ) -> Result<Result<S::Ok, S>, S::Error>
    where
        S: Serializer,
    {
        let Some(value) = value.try_as_reflect() else {
            return Ok(Err(serializer));
        };
        let type_id = value.reflect_type_info().type_id();

        // Non-finite floats: JSON has no infinity/NaN, serialize as descriptive strings
        if type_id == TypeId::of::<f32>() {
            if let Some(&v) = value.as_any().downcast_ref::<f32>() {
                if !v.is_finite() {
                    let s = if v == f32::INFINITY {
                        "inf"
                    } else if v == f32::NEG_INFINITY {
                        "-inf"
                    } else {
                        "NaN"
                    };
                    return Ok(Ok(serializer.serialize_str(s)?));
                }
            }
        }
        if type_id == TypeId::of::<f64>() {
            if let Some(&v) = value.as_any().downcast_ref::<f64>() {
                if !v.is_finite() {
                    let s = if v == f64::INFINITY {
                        "inf"
                    } else if v == f64::NEG_INFINITY {
                        "-inf"
                    } else {
                        "NaN"
                    };
                    return Ok(Ok(serializer.serialize_str(s)?));
                }
            }
        }

        // Handle<T> → path string or inline #Name
        if let Some(reflect_handle) = registry.get_type_data::<ReflectHandle>(type_id) {
            let untyped_handle = reflect_handle
                .downcast_handle_untyped(value.as_any())
                .expect("This must have been a handle");

            // Check collected asset references first (both inline and external)
            if let Some(inline_name) = self.inline_assets.get(&untyped_handle.id()) {
                return Ok(Ok(serializer.serialize_str(inline_name)?));
            }

            if let Some(path) = untyped_handle.path() {
                // Uncollected external asset — serialize as relative path (backward compat)
                let rel = pathdiff::diff_paths(path.path(), &self.parent_path)
                    .unwrap_or_else(|| path.path().to_owned());
                let mut path_str = rel.to_string_lossy().into_owned();
                if let Some(label) = path.label() {
                    path_str.push('#');
                    path_str.push_str(label);
                }
                return Ok(Ok(serializer.serialize_str(&path_str)?));
            }

            // Unknown handle (no path, not inline) — serialize as null
            return Ok(Ok(serializer.serialize_unit()?));
        }

        // Entity → scene-local index
        if type_id == TypeId::of::<Entity>() {
            if let Some(entity) = value.as_any().downcast_ref::<Entity>() {
                if let Some(&idx) = self.entity_to_index.get(entity) {
                    return Ok(Ok(serializer.serialize_u64(idx as u64)?));
                }
            }
            return Ok(Ok(serializer.serialize_unit()?));
        }

        Ok(Err(serializer))
    }
}

// ─────────────────────────────────── Deserializer Processor ───────────────────────────────────

pub(crate) struct JsnDeserializerProcessor<'a> {
    pub(crate) asset_server: &'a AssetServer,
    pub(crate) parent_path: &'a Path,
    /// Maps inline `#Name` references to loaded handles.
    pub(crate) local_assets: &'a HashMap<String, UntypedHandle>,
    /// Maps scene-local indices to spawned entities.
    pub(crate) entity_map: &'a [Entity],
}

impl<'a> ReflectDeserializerProcessor for JsnDeserializerProcessor<'a> {
    fn try_deserialize<'de, D>(
        &mut self,
        registration: &TypeRegistration,
        _registry: &TypeRegistry,
        deserializer: D,
    ) -> Result<Result<Box<dyn PartialReflect>, D>, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Non-finite floats: deserialize from string ("inf", "-inf", "NaN") or number
        if registration.type_id() == TypeId::of::<f32>() {
            let val = deserializer
                .deserialize_any(F32Visitor)
                .map_err(|e| <D::Error as serde::de::Error>::custom(e))?;
            return Ok(Ok(Box::new(val).into_partial_reflect()));
        }
        if registration.type_id() == TypeId::of::<f64>() {
            let val = deserializer
                .deserialize_any(F64Visitor)
                .map_err(|e| <D::Error as serde::de::Error>::custom(e))?;
            return Ok(Ok(Box::new(val).into_partial_reflect()));
        }

        // Handle<T> — deserialize from path string or #Name
        if registration.data::<ReflectHandle>().is_some() {
            let type_info = registration.type_info();

            let relative_path = match deserializer.deserialize_any(&*self) {
                Ok(path) => path,
                Err(error) => {
                    error!(
                        "Failed to deserialize `{}`: {:?}",
                        type_info.type_path(),
                        error
                    );
                    return Err(error);
                }
            };

            // Null sentinel (from old files with "material": null) → default handle
            if relative_path.is_empty() {
                if let Some(reflect_default) = registration.data::<ReflectDefault>() {
                    return Ok(Ok(reflect_default.default().into_partial_reflect()));
                }
            }

            // Check for inline asset reference
            if let Some(handle) = self.local_assets.get(&relative_path) {
                return Ok(Ok(Box::new(handle.clone()).into_partial_reflect()));
            }

            // External asset path
            let stem_pos = relative_path.find('#').unwrap_or(relative_path.len());
            let stem = self.relative_path_to_asset_path(&relative_path[0..stem_pos]);
            let mut asset_path = stem.to_string_lossy().into_owned();
            asset_path.push_str(&relative_path[stem_pos..]);

            let handle = self.asset_server.load_untyped(asset_path);
            return Ok(Ok(Box::new(handle).into_partial_reflect()));
        }

        // Entity — deserialize from scene-local index
        if registration.type_id() == TypeId::of::<Entity>() {
            let idx_str = match deserializer.deserialize_u64(&*self) {
                Ok(s) => s,
                Err(_) => {
                    // Not a valid index, return placeholder
                    return Ok(Ok(
                        Box::new(Entity::PLACEHOLDER).into_partial_reflect()
                    ));
                }
            };
            let idx: usize = idx_str.parse().unwrap_or(usize::MAX);
            let entity = self
                .entity_map
                .get(idx)
                .copied()
                .unwrap_or(Entity::PLACEHOLDER);
            return Ok(Ok(Box::new(entity).into_partial_reflect()));
        }

        Ok(Err(deserializer))
    }
}

impl<'a> Visitor<'_> for &'a JsnDeserializerProcessor<'a> {
    type Value = String;

    fn expecting(&self, formatter: &mut Formatter) -> fmt::Result {
        write!(formatter, "a string, integer, or null")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(String::new())
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(v.to_owned())
    }

    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(v.to_string())
    }
}

// ─────────────────────────────── Float Visitors (inf/NaN round-trip) ────────────────────────────

struct F32Visitor;

impl Visitor<'_> for F32Visitor {
    type Value = f32;

    fn expecting(&self, formatter: &mut Formatter) -> fmt::Result {
        write!(formatter, "a number or float string (inf, -inf, NaN)")
    }

    fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Self::Value, E> {
        Ok(v as f32)
    }

    fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
        Ok(v as f32)
    }

    fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
        Ok(v as f32)
    }

    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
        match v {
            "inf" | "Infinity" => Ok(f32::INFINITY),
            "-inf" | "-Infinity" => Ok(f32::NEG_INFINITY),
            "NaN" | "nan" => Ok(f32::NAN),
            _ => Err(E::custom(format!("unexpected float string: {v}"))),
        }
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
        Ok(0.0) // backward compat: old files with null
    }
}

struct F64Visitor;

impl Visitor<'_> for F64Visitor {
    type Value = f64;

    fn expecting(&self, formatter: &mut Formatter) -> fmt::Result {
        write!(formatter, "a number or float string (inf, -inf, NaN)")
    }

    fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Self::Value, E> {
        Ok(v)
    }

    fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
        Ok(v as f64)
    }

    fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
        Ok(v as f64)
    }

    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
        match v {
            "inf" | "Infinity" => Ok(f64::INFINITY),
            "-inf" | "-Infinity" => Ok(f64::NEG_INFINITY),
            "NaN" | "nan" => Ok(f64::NAN),
            _ => Err(E::custom(format!("unexpected float string: {v}"))),
        }
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
        Ok(0.0) // backward compat: old files with null
    }
}

impl<'a> JsnDeserializerProcessor<'a> {
    fn relative_path_to_asset_path(&self, asset_path: &str) -> PathBuf {
        let mut asset_path = Path::new(asset_path).to_owned();
        if asset_path.is_relative() {
            asset_path = self.parent_path.join(asset_path);
        }
        asset_path
    }
}

// ─────────────────────────────────── Inline Asset Collection ───────────────────────────────────

/// Walk all scene entity components, find `Handle<T>` fields that have no asset path
/// (runtime-created), serialize them into the generic assets table, and return a map
/// of asset ID → inline name for the serializer processor.
fn collect_inline_assets(
    world: &World,
    registry: &TypeRegistry,
    parent_path: &Path,
    scene_entities: &[Entity],
) -> (
    HashMap<UntypedAssetId, String>,
    HashMap<String, HashMap<String, serde_json::Value>>,
) {
    let mut id_to_name: HashMap<UntypedAssetId, String> = HashMap::new();
    let mut asset_data: HashMap<String, HashMap<String, serde_json::Value>> = HashMap::new();
    let mut counters: HashMap<String, usize> = HashMap::new();

    // Scan all scene entities' components for Handle<T> values,
    // collect the ones without paths, serialize the underlying asset data.
    let skip_ids: HashSet<TypeId> = HashSet::from([
        TypeId::of::<Name>(),
        TypeId::of::<Transform>(),
        TypeId::of::<GlobalTransform>(),
        TypeId::of::<Visibility>(),
        TypeId::of::<InheritedVisibility>(),
        TypeId::of::<ViewVisibility>(),
        TypeId::of::<ChildOf>(),
        TypeId::of::<Children>(),
    ]);

    for &entity in scene_entities {
        let entity_ref = world.entity(entity);

        for registration in registry.iter() {
            if skip_ids.contains(&registration.type_id()) {
                continue;
            }
            let type_path = registration.type_info().type_path_table().path();
            if should_skip_component(type_path) {
                continue;
            }
            let Some(reflect_component) = registration.data::<ReflectComponent>() else {
                continue;
            };
            let Some(component) = reflect_component.reflect(entity_ref) else {
                continue;
            };

            // Walk the reflected value looking for Handle<T> fields
            collect_handles_from_reflect(
                component.as_partial_reflect(),
                registry,
                world,
                parent_path,
                &mut id_to_name,
                &mut asset_data,
                &mut counters,
            );
        }
    }

    (id_to_name, asset_data)
}

/// Recursively walk a reflected value looking for Handle<T> fields that are runtime-created.
fn collect_handles_from_reflect(
    value: &dyn PartialReflect,
    registry: &TypeRegistry,
    world: &World,
    parent_path: &Path,
    id_to_name: &mut HashMap<UntypedAssetId, String>,
    asset_data: &mut HashMap<String, HashMap<String, serde_json::Value>>,
    counters: &mut HashMap<String, usize>,
) {
    let Some(value) = value.try_as_reflect() else {
        return;
    };
    let type_id = value.reflect_type_info().type_id();

    // Check if this is a Handle<T>
    if let Some(reflect_handle) = registry.get_type_data::<ReflectHandle>(type_id) {
        let untyped_handle = reflect_handle
            .downcast_handle_untyped(value.as_any())
            .expect("This must have been a handle");

        // Already collected — skip
        if id_to_name.contains_key(&untyped_handle.id()) {
            return;
        }

        // External file-backed resource — store as a path string entry
        if let Some(asset_path) = untyped_handle.path() {
            let asset_type_id = reflect_handle.asset_type_id();
            let Some(asset_registration) = registry.get(asset_type_id) else {
                return;
            };
            let asset_type_path = asset_registration
                .type_info()
                .type_path_table()
                .path()
                .to_string();

            let counter = counters.entry(asset_type_path.clone()).or_insert(0);
            let short_name = asset_type_path
                .rsplit("::")
                .next()
                .unwrap_or(&asset_type_path);
            let inline_name = format!("#{short_name}{counter}");
            *counter += 1;

            let rel = pathdiff::diff_paths(asset_path.path(), parent_path)
                .unwrap_or_else(|| asset_path.path().to_owned());
            let mut path_str = rel.to_string_lossy().into_owned();
            if let Some(label) = asset_path.label() {
                path_str.push('#');
                path_str.push_str(label);
            }

            id_to_name.insert(untyped_handle.id(), inline_name.clone());
            asset_data
                .entry(asset_type_path)
                .or_default()
                .insert(inline_name, serde_json::Value::String(path_str));
            return;
        }

        // Skip default/UUID handles (not backed by a live asset)
        if matches!(untyped_handle, UntypedHandle::Uuid { .. }) {
            return;
        }

        let asset_type_id = reflect_handle.asset_type_id();
        let Some(asset_registration) = registry.get(asset_type_id) else {
            return;
        };
        let Some(reflect_asset) = asset_registration.data::<ReflectAsset>() else {
            return;
        };

        let asset_type_path = asset_registration
            .type_info()
            .type_path_table()
            .path()
            .to_string();

        // Get the asset data and serialize it
        let Some(asset_reflect) = reflect_asset.get(world, untyped_handle.id()) else {
            return;
        };

        // Recurse into the asset to collect nested handles (e.g. textures inside materials)
        // before serializing, so they get #Name entries and the serializer emits refs not paths.
        collect_handles_from_reflect(
            asset_reflect.as_partial_reflect(),
            registry,
            world,
            parent_path,
            id_to_name,
            asset_data,
            counters,
        );

        // Generate a name like "Material0", "Material1"
        let counter = counters.entry(asset_type_path.clone()).or_insert(0);
        let short_name = asset_type_path
            .rsplit("::")
            .next()
            .unwrap_or(&asset_type_path);
        let inline_name = format!("#{short_name}{counter}");
        *counter += 1;

        // Serialize the asset using the processor (for nested handles like textures inside materials)
        let ser_processor = JsnSerializerProcessor {
            parent_path: Cow::Borrowed(parent_path),
            inline_assets: id_to_name, // partial map, but handles already collected will be there
            entity_to_index: &HashMap::new(),
        };
        let serializer =
            TypedReflectSerializer::with_processor(asset_reflect, registry, &ser_processor);
        if let Ok(json_value) = serde_json::to_value(&serializer) {
            id_to_name.insert(untyped_handle.id(), inline_name.clone());
            asset_data
                .entry(asset_type_path)
                .or_default()
                .insert(inline_name, json_value);
        }

        return;
    }

    // Recurse into struct/tuple/list/map fields
    match value.reflect_ref() {
        bevy::reflect::ReflectRef::Struct(s) => {
            for i in 0..s.field_len() {
                if let Some(field) = s.field_at(i) {
                    collect_handles_from_reflect(
                        field,
                        registry,
                        world,
                        parent_path,
                        id_to_name,
                        asset_data,
                        counters,
                    );
                }
            }
        }
        bevy::reflect::ReflectRef::TupleStruct(ts) => {
            for i in 0..ts.field_len() {
                if let Some(field) = ts.field(i) {
                    collect_handles_from_reflect(
                        field,
                        registry,
                        world,
                        parent_path,
                        id_to_name,
                        asset_data,
                        counters,
                    );
                }
            }
        }
        bevy::reflect::ReflectRef::Tuple(t) => {
            for i in 0..t.field_len() {
                if let Some(field) = t.field(i) {
                    collect_handles_from_reflect(
                        field,
                        registry,
                        world,
                        parent_path,
                        id_to_name,
                        asset_data,
                        counters,
                    );
                }
            }
        }
        bevy::reflect::ReflectRef::List(l) => {
            for i in 0..l.len() {
                if let Some(item) = l.get(i) {
                    collect_handles_from_reflect(
                        item,
                        registry,
                        world,
                        parent_path,
                        id_to_name,
                        asset_data,
                        counters,
                    );
                }
            }
        }
        bevy::reflect::ReflectRef::Array(a) => {
            for i in 0..a.len() {
                if let Some(item) = a.get(i) {
                    collect_handles_from_reflect(
                        item,
                        registry,
                        world,
                        parent_path,
                        id_to_name,
                        asset_data,
                        counters,
                    );
                }
            }
        }
        bevy::reflect::ReflectRef::Map(m) => {
            for (_k, v) in m.iter() {
                collect_handles_from_reflect(
                    v,
                    registry,
                    world,
                    parent_path,
                    id_to_name,
                    asset_data,
                    counters,
                );
            }
        }
        bevy::reflect::ReflectRef::Set(s) => {
            for item in s.iter() {
                collect_handles_from_reflect(
                    item,
                    registry,
                    world,
                    parent_path,
                    id_to_name,
                    asset_data,
                    counters,
                );
            }
        }
        bevy::reflect::ReflectRef::Enum(e) => {
            for i in 0..e.field_len() {
                if let Some(field) = e.field_at(i) {
                    collect_handles_from_reflect(
                        field,
                        registry,
                        world,
                        parent_path,
                        id_to_name,
                        asset_data,
                        counters,
                    );
                }
            }
        }
        bevy::reflect::ReflectRef::Opaque(_) => {}
    }
}

// ─────────────────────────────────── Save (Snapshot) ───────────────────────────────────

/// Build a `Vec<JsnEntity>` from scene entities using reflection.
/// Uses the serializer processor to handle `Handle<T>` and `Entity` fields.
fn build_scene_snapshot(
    world: &World,
    registry: &TypeRegistry,
    parent_path: &Path,
    inline_assets: &HashMap<UntypedAssetId, String>,
    entities: &[Entity],
) -> Vec<JsnEntity> {

    // Build entity → index map for parent and entity-field references
    let entity_to_index: HashMap<Entity, usize> =
        entities.iter().enumerate().map(|(i, &e)| (e, i)).collect();

    let ser_processor = JsnSerializerProcessor {
        parent_path: Cow::Borrowed(parent_path),
        inline_assets,
        entity_to_index: &entity_to_index,
    };

    // Component types handled as explicit fields — skip in the generic loop
    let skip_ids: HashSet<TypeId> = HashSet::from([
        TypeId::of::<Name>(),
        TypeId::of::<Transform>(),
        TypeId::of::<GlobalTransform>(),
        TypeId::of::<Visibility>(),
        TypeId::of::<InheritedVisibility>(),
        TypeId::of::<ViewVisibility>(),
        TypeId::of::<ChildOf>(),
        TypeId::of::<Children>(),
    ]);

    entities
        .iter()
        .map(|&entity| {
            let entity_ref = world.entity(entity);

            // Core fields
            let name = entity_ref.get::<Name>().map(|n| n.to_string());
            let transform = entity_ref.get::<Transform>().map(|t| (*t).into());
            let visibility = entity_ref
                .get::<Visibility>()
                .map(|v| (*v).into())
                .unwrap_or_default();
            let parent = entity_ref
                .get::<ChildOf>()
                .and_then(|c| entity_to_index.get(&c.parent()).copied());

            // Extensible components via reflection — now with processor
            let mut components = HashMap::new();

            for registration in registry.iter() {
                if skip_ids.contains(&registration.type_id()) {
                    continue;
                }

                let type_path = registration.type_info().type_path_table().path();

                if should_skip_component(type_path) {
                    continue;
                }

                let Some(reflect_component) = registration.data::<ReflectComponent>() else {
                    continue;
                };
                let Some(component) = reflect_component.reflect(entity_ref) else {
                    continue;
                };

                // Serialize with processor — handles Handle<T> → path and Entity → index
                let serializer =
                    TypedReflectSerializer::with_processor(component, registry, &ser_processor);
                if let Ok(value) = serde_json::to_value(&serializer) {
                    components.insert(type_path.to_string(), value);
                }
            }

            JsnEntity {
                name,
                transform,
                visibility,
                parent,
                components,
            }
        })
        .collect()
}

// ─────────────────────────────────── Load ───────────────────────────────────

fn finish_load_scene(world: &mut World, chosen: &std::path::Path) {
    let path = chosen.to_string_lossy().to_string();
    let last_dir = chosen.parent().map(|p| p.to_path_buf());

    // Update last directory
    world.resource_mut::<SceneFilePath>().last_directory = last_dir;

    let json = match std::fs::read_to_string(&path) {
        Ok(json) => json,
        Err(err) => {
            warn!("Failed to read scene file '{path}': {err}");
            return;
        }
    };

    if path.ends_with(".scene.json") {
        // Legacy format: raw DynamicScene JSON
        let registry = world.resource::<AppTypeRegistry>().clone();
        let registry = registry.read();

        use bevy::scene::serde::SceneDeserializer;
        let scene_deserializer = SceneDeserializer {
            type_registry: &registry,
        };
        let mut json_de = serde_json::Deserializer::from_str(&json);
        let scene = match scene_deserializer.deserialize(&mut json_de) {
            Ok(scene) => scene,
            Err(err) => {
                warn!("Failed to deserialize legacy scene: {err}");
                return;
            }
        };

        drop(registry);
        clear_scene_entities(world);
        match scene.write_to_world(world, &mut Default::default()) {
            Ok(_) => info!("Scene loaded from {path} (legacy format)"),
            Err(err) => warn!("Failed to write scene to world: {err}"),
        }
    } else {
        // JSN v2 format
        let jsn: JsnScene = match serde_json::from_str(&json) {
            Ok(jsn) => jsn,
            Err(err) => {
                warn!("Failed to parse JSN file: {err}");
                return;
            }
        };

        // Check format version
        if jsn.jsn.format_version[0] < 2 {
            warn!(
                "JSN format version {:?} is not supported. Please re-save with the latest editor.",
                jsn.jsn.format_version
            );
            return;
        }

        clear_scene_entities(world);

        let parent_path = Path::new(&path).parent().unwrap_or(Path::new("."));

        // Deserialize inline assets before entities
        let local_assets = load_inline_assets(world, &jsn.assets, parent_path);

        // Load entities with processor
        load_scene_from_jsn(world, &jsn.scene, parent_path, &local_assets);

        info!("Scene loaded from {path}");

        // Restore metadata
        let mut scene_path = world.resource_mut::<SceneFilePath>();
        scene_path.metadata = jsn.metadata;
    }

    world.resource_mut::<SceneFilePath>().path = Some(path);
}

/// Deserialize inline assets from the generic assets table.
/// Returns a map of `#Name` → `UntypedHandle` for the deserializer processor.
pub fn load_inline_assets(
    world: &mut World,
    assets: &JsnAssets,
    parent_path: &Path,
) -> HashMap<String, UntypedHandle> {
    let mut local_assets: HashMap<String, UntypedHandle> = HashMap::new();

    let registry = world.resource::<AppTypeRegistry>().clone();
    let registry_guard = registry.read();
    let asset_server = world.resource::<AssetServer>().clone();

    // First pass: load all string-value entries (external file refs like textures).
    // These must be loaded before inline assets that may reference them.
    for (type_path, named_entries) in &assets.0 {
        for (name, json_value) in named_entries {
            let serde_json::Value::String(rel_path) = json_value else {
                continue;
            };

            let abs_path = if Path::new(rel_path).is_relative() {
                parent_path.join(rel_path)
            } else {
                PathBuf::from(rel_path)
            };
            let path_str = abs_path.to_string_lossy().into_owned();

            let handle = if type_path == "bevy_image::image::Image" {
                asset_server.load::<Image>(&path_str).untyped()
            } else {
                warn!(
                    "External asset entry '{name}' has unknown type '{type_path}' — loading untyped"
                );
                asset_server.load::<bevy::asset::LoadedUntypedAsset>(&path_str).untyped()
            };
            local_assets.insert(name.clone(), handle);
        }
    }

    // Second pass: deserialize all object-value entries (inline assets like materials)
    for (type_path, named_entries) in &assets.0 {
        let Some(registration) = registry_guard.get_with_type_path(type_path) else {
            warn!("Unknown asset type '{type_path}' in inline assets — skipping");
            continue;
        };
        let Some(reflect_asset) = registration.data::<ReflectAsset>() else {
            warn!("Type '{type_path}' has no ReflectAsset — skipping");
            continue;
        };

        for (name, json_value) in named_entries {
            // String entries already handled in first pass
            if json_value.is_string() {
                continue;
            }

            // Deserialize with processor to resolve nested handles (e.g. textures in materials)
            let mut deser_processor = JsnDeserializerProcessor {
                asset_server: &asset_server,
                parent_path,
                local_assets: &local_assets,
                entity_map: &[],
            };

            let deserializer =
                TypedReflectDeserializer::with_processor(registration, &registry_guard, &mut deser_processor);
            let Ok(reflected) = deserializer.deserialize(json_value) else {
                warn!("Failed to deserialize inline asset '{name}' of type '{type_path}'");
                continue;
            };

            // Add into the asset store and get a handle
            let handle = reflect_asset.add(world, reflected.as_ref());
            local_assets.insert(name.clone(), handle);
        }
    }

    local_assets
}

/// Spawn entities from a `Vec<JsnEntity>` into the world using reflection.
pub fn load_scene_from_jsn(
    world: &mut World,
    entities: &[JsnEntity],
    parent_path: &Path,
    local_assets: &HashMap<String, UntypedHandle>,
) {
    let registry = world.resource::<AppTypeRegistry>().clone();
    let asset_server = world.resource::<AssetServer>().clone();

    // First pass: spawn entities with core fields
    let mut spawned: Vec<Entity> = Vec::new();
    for (i, jsn) in entities.iter().enumerate() {
        let mut entity = world.spawn_empty();
        entity.insert(Name::new(
            jsn.name
                .clone()
                .unwrap_or_else(|| format!("Entity {}", i)),
        ));
        if let Some(t) = &jsn.transform {
            entity.insert(Transform::from(t.clone()));
        }
        let vis: Visibility = jsn.visibility.clone().into();
        entity.insert(vis);
        spawned.push(entity.id());
    }

    // Second pass: set parents (ChildOf)
    for (i, jsn) in entities.iter().enumerate() {
        if let Some(parent_idx) = jsn.parent {
            if let Some(&parent_entity) = spawned.get(parent_idx) {
                world.entity_mut(spawned[i]).insert(ChildOf(parent_entity));
            }
        }
    }

    // Third pass: deserialize extensible components via reflection with processor
    let registry_guard = registry.read();
    for (i, jsn) in entities.iter().enumerate() {
        for (type_path, value) in &jsn.components {
            let Some(registration) = registry_guard.get_with_type_path(type_path) else {
                warn!("Unknown type '{type_path}' — skipping");
                continue;
            };
            let Some(reflect_component) = registration.data::<ReflectComponent>() else {
                warn!("Type '{type_path}' has no ReflectComponent — skipping");
                continue;
            };

            let mut deser_processor = JsnDeserializerProcessor {
                asset_server: &asset_server,
                parent_path,
                local_assets,
                entity_map: &spawned,
            };
            let deserializer = TypedReflectDeserializer::with_processor(
                registration,
                &registry_guard,
                &mut deser_processor,
            );
            let Ok(reflected) = deserializer.deserialize(value) else {
                warn!("Failed to deserialize '{type_path}' — skipping");
                continue;
            };

            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                reflect_component.insert(
                    &mut world.entity_mut(spawned[i]),
                    reflected.as_ref(),
                    &registry_guard,
                );
            }));
            if result.is_err() {
                warn!("Panic while inserting component '{type_path}' — skipping");
            }
        }
    }
    drop(registry_guard);

    // Post-load: re-trigger GLTF loading for GltfSource entities
    let gltf_entities: Vec<(Entity, String, usize)> = spawned
        .iter()
        .filter_map(|&e| {
            world
                .get::<jackdaw_jsn::GltfSource>(e)
                .map(|gs| (e, gs.path.clone(), gs.scene_index))
        })
        .collect();
    for (entity, gltf_path, scene_index) in gltf_entities {
        let asset_server = world.resource::<AssetServer>();
        let asset_path: AssetPath<'static> = gltf_path.into();
        let scene = asset_server.load(GltfAssetLabel::Scene(scene_index).from_asset(asset_path));
        world.entity_mut(entity).insert(SceneRoot(scene));
    }
}

pub fn new_scene(world: &mut World) {
    clear_scene_entities(world);
    let mut scene_path = world.resource_mut::<SceneFilePath>();
    scene_path.path = None;
    scene_path.metadata = JsnMetadata::default();
    info!("New scene created");
}

// ─────────────────────────────────── Helpers ───────────────────────────────────

/// Collect scene entities (named non-editor entities and all their descendants).
/// Requires `&mut World` for `query_filtered`.
fn collect_scene_entities_from_set(world: &mut World, editor_set: &HashSet<Entity>) -> Vec<Entity> {
    let roots: Vec<Entity> = world
        .query_filtered::<Entity, With<Name>>()
        .iter(world)
        .filter(|e| !editor_set.contains(e))
        .collect();

    // Expand to include all descendants
    let mut scene_set = HashSet::new();
    let mut stack = roots;
    while let Some(entity) = stack.pop() {
        if !scene_set.insert(entity) {
            continue;
        }
        if let Some(children) = world.get::<Children>(entity) {
            for child in children.iter() {
                if world.get::<EditorHidden>(child).is_none()
                    && world.get::<NonSerializable>(child).is_none()
                {
                    stack.push(child);
                }
            }
        }
    }

    scene_set.into_iter().collect()
}

/// Collect the set of all editor entities (those with `EditorEntity` and all their descendants).
fn collect_editor_entities(world: &mut World) -> HashSet<Entity> {
    let roots: Vec<Entity> = world
        .query_filtered::<Entity, With<EditorEntity>>()
        .iter(world)
        .collect();

    let mut editor_set = HashSet::new();
    let mut stack = roots;
    while let Some(entity) = stack.pop() {
        if !editor_set.insert(entity) {
            continue;
        }
        if let Some(children) = world.get::<Children>(entity) {
            stack.extend(children.iter());
        }
    }
    editor_set
}

/// Remove scene entities from the world (named non-editor entities + their descendants).
fn clear_scene_entities(world: &mut World) {
    // Clear selection first to prevent on_entity_deselected observer from
    // firing on stale/despawned tree row entities.
    world.resource_mut::<crate::selection::Selection>().entities.clear();

    // Clear hierarchy tree rows and TreeIndex before despawning scene entities.
    crate::hierarchy::clear_all_tree_rows(world);

    // Clear undo/redo stacks — they hold entity references that become stale.
    let mut history = world.resource_mut::<jackdaw_commands::CommandHistory>();
    history.undo_stack.clear();
    history.redo_stack.clear();

    let editor_set = collect_editor_entities(world);

    // Collect named non-editor entities as roots
    let roots: Vec<Entity> = world
        .query_filtered::<Entity, With<Name>>()
        .iter(world)
        .filter(|e| !editor_set.contains(e))
        .collect();

    // Expand to include all descendants
    let mut scene_set = HashSet::new();
    let mut stack = roots;
    while let Some(entity) = stack.pop() {
        if !scene_set.insert(entity) {
            continue;
        }
        if let Some(children) = world.get::<Children>(entity) {
            stack.extend(children.iter());
        }
    }

    for entity in scene_set {
        if let Ok(entity_mut) = world.get_entity_mut(entity) {
            entity_mut.despawn();
        }
    }
}

/// ISO 8601 timestamp (simplified — no chrono dependency).
fn chrono_now() -> String {
    let since_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = since_epoch.as_secs();
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;
    let (year, month, day) = days_to_date(days);
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

fn days_to_date(days: u64) -> (u64, u64, u64) {
    let mut y = 1970;
    let mut remaining = days;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }
    let month_days = if is_leap(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut m = 0;
    for (i, &md) in month_days.iter().enumerate() {
        if remaining < md {
            m = i;
            break;
        }
        remaining -= md;
    }
    (y, m as u64 + 1, remaining + 1)
}

fn is_leap(y: u64) -> bool {
    (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400)
}

/// Temporarily replace `Handle::default()` brush face materials with the palette material
/// so they serialize as inline assets rather than null.
/// Returns a list of (entity, face indices) that were changed.
fn resolve_default_brush_materials(
    world: &mut World,
    scene_entities: &[Entity],
) -> Vec<(Entity, Vec<usize>)> {
    let palette_material = world
        .get_resource::<BrushMaterialPalette>()
        .and_then(|p| p.materials.first().cloned());
    let Some(palette_material) = palette_material else {
        return Vec::new();
    };

    let mut changed = Vec::new();

    for &entity in scene_entities {
        let Some(brush) = world.get::<Brush>(entity) else {
            continue;
        };

        let default_faces: Vec<usize> = brush
            .faces
            .iter()
            .enumerate()
            .filter(|(_, f)| f.material == Handle::default())
            .map(|(i, _)| i)
            .collect();

        if default_faces.is_empty() {
            continue;
        }

        // Use bypass_change_detection to avoid triggering mesh regen
        let mut brush_mut = world.get_mut::<Brush>(entity).unwrap();
        let brush_inner = brush_mut.bypass_change_detection();
        for &fi in &default_faces {
            brush_inner.faces[fi].material = palette_material.clone();
        }

        changed.push((entity, default_faces));
    }

    changed
}

/// Restore `Handle::default()` on brush faces that were temporarily resolved.
fn restore_default_brush_materials(world: &mut World, changed: &[(Entity, Vec<usize>)]) {
    for (entity, face_indices) in changed {
        let Some(mut brush_mut) = world.get_mut::<Brush>(*entity) else {
            continue;
        };
        let brush_inner = brush_mut.bypass_change_detection();
        for &fi in face_indices {
            brush_inner.faces[fi].material = Handle::default();
        }
    }
}

fn poll_scene_dialog(world: &mut World) {
    let Some(mut task) = world.remove_resource::<SceneDialogTask>() else {
        return;
    };

    match &mut task {
        SceneDialogTask::Save(t) => {
            let Some(result) = future::block_on(future::poll_once(t)) else {
                world.insert_resource(task); // Not ready, put it back
                return;
            };
            if let Some(file) = result {
                let path = file.path().to_path_buf();
                let path_str = path.to_string_lossy().to_string();
                let last_dir = path.parent().map(|p| p.to_path_buf());

                let mut scene_path = world.resource_mut::<SceneFilePath>();
                scene_path.path = Some(path_str);
                scene_path.last_directory = last_dir;

                save_scene_inner(world);
            }
        }
        SceneDialogTask::Load(t) => {
            let Some(result) = future::block_on(future::poll_once(t)) else {
                world.insert_resource(task);
                return;
            };
            if let Some(file) = result {
                finish_load_scene(world, file.path());
            }
        }
    }
}

fn handle_scene_io_keys(world: &mut World) {
    let keyboard = world.resource::<ButtonInput<KeyCode>>();
    let ctrl = keyboard.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    let shift = keyboard.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let s_pressed = keyboard.just_pressed(KeyCode::KeyS);
    let o_pressed = keyboard.just_pressed(KeyCode::KeyO);
    let n_pressed = keyboard.just_pressed(KeyCode::KeyN);

    if ctrl && shift && s_pressed {
        save_scene_as(world);
    } else if ctrl && s_pressed {
        save_scene(world);
    } else if ctrl && o_pressed {
        load_scene(world);
    } else if ctrl && shift && n_pressed {
        new_scene(world);
    }
}
