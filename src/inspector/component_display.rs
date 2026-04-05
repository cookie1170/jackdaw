use crate::EditorEntity;
use crate::colors;
use crate::custom_properties::CustomProperties;
use crate::selection::{Selected, Selection};
use std::any::TypeId;

use bevy::{
    ecs::{
        archetype::Archetype,
        component::{ComponentId, Components},
        reflect::{AppTypeRegistry, ReflectComponent},
    },
    prelude::*,
    reflect::serde::TypedReflectSerializer,
};
use jackdaw_feathers::{
    icons::{EditorFont, Icon, IconFont},
    tokens,
};
use jackdaw_widgets::collapsible::{
    CollapsibleBody, CollapsibleHeader, CollapsibleSection, ToggleCollapsible,
};

use jackdaw_feathers::text_edit::{self, TextEditProps, TextEditValue};
use std::collections::HashSet;

use bevy_monitors::prelude::{Addition, Monitor, NotifyAdded};

use super::{
    AddComponentButton, CollapseAllButton, ComponentDisplay, ComponentDisplayBody, ComponentName,
    ComponentPicker, Inspector, InspectorDirty, InspectorGroupSection, InspectorSearch,
    InspectorTarget, ReflectDisplayable, ReflectEditorMeta, brush_display, custom_props_display,
    extract_module_group, material_display, reflect_fields,
};

pub(crate) fn add_component_displays(
    _: On<Add, Selected>,
    mut commands: Commands,
    components: &Components,
    type_registry: Res<AppTypeRegistry>,
    selection: Res<Selection>,
    entity_query: Query<(&Archetype, EntityRef), (With<Selected>, Without<EditorEntity>)>,
    inspector: Single<Entity, With<Inspector>>,
    names: Query<&Name>,
    icon_font: Res<IconFont>,
    editor_font: Res<EditorFont>,
    materials: Res<Assets<StandardMaterial>>,
    ast: Res<jackdaw_jsn::SceneJsnAst>,
) {
    let Some(primary) = selection.primary() else {
        return;
    };
    let Ok((archetype, entity_ref)) = entity_query.get(primary) else {
        return;
    };

    let source_entity = entity_ref.entity();
    let sel_count = selection.entities.len();

    // Collect AST-tracked component type paths
    let jsn_type_paths: HashSet<String> = ast
        .node_for_entity(source_entity)
        .map(|node| node.components.keys().cloned().collect())
        .unwrap_or_default();

    build_inspector_displays(
        &mut commands,
        components,
        &type_registry,
        source_entity,
        archetype,
        entity_ref,
        *inspector,
        sel_count,
        &names,
        &icon_font,
        &editor_font,
        false,
        &materials,
        &jsn_type_paths,
    );

    // Set up monitoring: watch the selected entity for InspectorDirty
    commands.entity(*inspector).insert((
        InspectorTarget(primary),
        Monitor(primary),
        NotifyAdded::<InspectorDirty>::default(),
    ));
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_inspector_displays(
    commands: &mut Commands,
    components: &Components,
    type_registry: &Res<AppTypeRegistry>,
    source_entity: Entity,
    archetype: &Archetype,
    entity_ref: EntityRef,
    inspector_entity: Entity,
    selection_count: usize,
    names: &Query<&Name>,
    icon_font: &IconFont,
    editor_font: &EditorFont,
    read_only: bool,
    materials: &Assets<StandardMaterial>,
    jsn_type_paths: &HashSet<String>,
) {
    // Show multi-selection header when multiple entities are selected
    if selection_count > 1 {
        commands.spawn((
            ComponentDisplay,
            Node {
                padding: UiRect::axes(Val::Px(tokens::SPACING_MD), Val::Px(tokens::SPACING_SM)),
                width: Val::Percent(100.0),
                ..Default::default()
            },
            BackgroundColor(tokens::SELECTED_BG),
            ChildOf(inspector_entity),
            children![(
                Text::new(format!(
                    "{selection_count} entities selected — edits apply to all"
                )),
                TextFont {
                    font: editor_font.0.clone(),
                    font_size: tokens::FONT_SM,
                    ..Default::default()
                },
                TextColor(tokens::TEXT_PRIMARY),
            )],
        ));
    }

    // Search bar + collapse-all row
    let search_row = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                width: Val::Percent(100.0),
                column_gap: Val::Px(tokens::SPACING_SM),
                padding: UiRect::axes(Val::Px(tokens::SPACING_SM), Val::Px(tokens::SPACING_XS)),
                ..Default::default()
            },
            ComponentDisplay,
            ChildOf(inspector_entity),
        ))
        .id();

    commands.spawn((
        InspectorSearch,
        text_edit::text_edit(
            TextEditProps::default()
                .with_placeholder("Filter components...")
                .allow_empty()
                .grow(),
        ),
        ChildOf(search_row),
    ));

    // Collapse-all / expand-all button
    let collapse_btn = commands
        .spawn((
            CollapseAllButton,
            Node {
                padding: UiRect::all(Val::Px(tokens::SPACING_XS)),
                ..Default::default()
            },
            BackgroundColor(Color::NONE),
            ChildOf(search_row),
        ))
        .id();

    commands.spawn((
        Text::new(String::from(Icon::ChevronsUpDown.unicode())),
        TextFont {
            font: icon_font.0.clone(),
            font_size: tokens::FONT_MD,
            ..Default::default()
        },
        TextColor(tokens::TEXT_SECONDARY),
        ChildOf(collapse_btn),
    ));

    commands.entity(collapse_btn).observe(
        |_: On<Pointer<Click>>,
         mut sections: Query<(&mut CollapsibleSection, &Children), With<ComponentDisplay>>,
         mut nodes: Query<&mut Node, With<CollapsibleBody>>| {
            // If any section is expanded, collapse all; otherwise expand all
            let any_expanded = sections.iter().any(|(s, _)| !s.collapsed);
            let target_collapsed = any_expanded;

            for (mut section, children) in &mut sections {
                section.collapsed = target_collapsed;
                for child in children.iter() {
                    if let Ok(mut node) = nodes.get_mut(child) {
                        node.display = if target_collapsed {
                            Display::None
                        } else {
                            Display::Flex
                        };
                    }
                }
            }
        },
    );

    // Hover effect on collapse button
    commands.entity(collapse_btn).observe(
        |hover: On<Pointer<Over>>, mut bg: Query<&mut BackgroundColor>| {
            if let Ok(mut bg) = bg.get_mut(hover.event_target()) {
                bg.0 = tokens::HOVER_BG;
            }
        },
    );
    commands.entity(collapse_btn).observe(
        |out: On<Pointer<Out>>, mut bg: Query<&mut BackgroundColor>| {
            if let Ok(mut bg) = bg.get_mut(out.event_target()) {
                bg.0 = Color::NONE;
            }
        },
    );

    let registry = type_registry.read();

    // Check for prefab baseline (override tracking)
    let baseline = entity_ref.get::<jackdaw_jsn::JsnPrefabBaseline>().cloned();

    // (short_name, module_group, component_id)
    let mut custom_groups = std::collections::HashSet::new();
    let mut comp_list: Vec<(String, String, ComponentId)> = archetype
        .iter_components()
        .filter_map(|component_id| {
            let info = components.get_info(component_id)?;
            let type_id = info.type_id();

            // Try TypeRegistry first for proper names
            if let Some(type_id) = type_id
                && let Some(registration) = registry.get(type_id)
            {
                let table = registration.type_info().type_path_table();
                let full_path = table.path();
                if full_path.starts_with("jackdaw") && !full_path.starts_with("jackdaw_jsn") {
                    return None;
                }
                // AST filter: only show components tracked in the AST
                if !jsn_type_paths.is_empty() && !jsn_type_paths.contains(full_path) {
                    return None;
                }
                let short = table.short_path().to_string();
                let module_group = if let Some(meta) = registration.data::<ReflectEditorMeta>()
                    && !meta.category.is_empty()
                {
                    let cat = meta.category.to_string();
                    custom_groups.insert(cat.clone());
                    cat
                } else {
                    extract_module_group(table.module_path())
                };
                return Some((short, module_group, component_id));
            }

            // Fallback: use Components name
            let name = components.get_name(component_id)?;
            if name.starts_with("jackdaw") && !name.starts_with("jackdaw_jsn") {
                return None;
            }
            Some((
                name.shortname().to_string(),
                "Other".to_string(),
                component_id,
            ))
        })
        .collect();

    // Sort: custom-category groups first, then alphabetical within each tier
    comp_list.sort_by(|a, b| {
        let a_custom = custom_groups.contains(&a.1);
        let b_custom = custom_groups.contains(&b.1);
        b_custom
            .cmp(&a_custom)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))
    });

    // Group by module and spawn with group headers
    let mut current_group = String::new();
    let mut group_container = inspector_entity;

    for (name, module_group, component_id) in &comp_list {
        // Start a new group section if the module changed
        if *module_group != current_group {
            current_group = module_group.clone();
            let section = commands
                .spawn((
                    ComponentDisplay,
                    InspectorGroupSection,
                    CollapsibleSection { collapsed: false },
                    Node {
                        flex_direction: FlexDirection::Column,
                        width: Val::Percent(100.0),
                        ..Default::default()
                    },
                    ChildOf(inspector_entity),
                ))
                .id();

            let header = commands
                .spawn((
                    CollapsibleHeader,
                    Node {
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        width: Val::Percent(100.0),
                        padding: UiRect::axes(
                            Val::Px(tokens::SPACING_SM),
                            Val::Px(tokens::SPACING_SM),
                        ),
                        column_gap: Val::Px(tokens::SPACING_SM),
                        ..Default::default()
                    },
                    BackgroundColor(tokens::PANEL_BG),
                    ChildOf(section),
                ))
                .id();

            let section_for_toggle = section;
            commands.entity(header).observe(
                move |_: On<Pointer<Click>>, mut commands: Commands| {
                    commands.trigger(ToggleCollapsible {
                        entity: section_for_toggle,
                    });
                },
            );

            // Group icon
            let (group_icon, icon_color) = if custom_groups.contains(module_group) {
                (Icon::Tag, tokens::CATEGORY_ENTITY)
            } else {
                (Icon::Package, tokens::TEXT_SECONDARY)
            };

            commands.spawn((
                Text::new(String::from(group_icon.unicode())),
                TextFont {
                    font: icon_font.0.clone(),
                    font_size: tokens::FONT_MD,
                    ..Default::default()
                },
                TextColor(icon_color),
                ChildOf(header),
            ));

            // Group name
            commands.spawn((
                Text::new(module_group.clone()),
                TextFont {
                    font: editor_font.0.clone(),
                    font_size: tokens::FONT_MD,
                    weight: FontWeight::BOLD,
                    ..Default::default()
                },
                TextColor(tokens::TEXT_SECONDARY),
                ChildOf(header),
            ));

            group_container = commands
                .spawn((
                    CollapsibleBody,
                    Node {
                        flex_direction: FlexDirection::Column,
                        width: Val::Percent(100.0),
                        border: UiRect::left(Val::Px(1.0)),
                        margin: UiRect::left(Val::Px(tokens::SPACING_MD)),
                        ..Default::default()
                    },
                    BorderColor::all(tokens::BORDER_SUBTLE),
                    ChildOf(section),
                ))
                .id();
        }

        let component_id = *component_id;

        // Detect override: compare current component value vs baseline
        let is_overridden = baseline.as_ref().is_some_and(|bl| {
            let type_id = components
                .get_info(component_id)
                .and_then(|info| info.type_id());
            if let Some(type_id) = type_id
                && let Some(registration) = registry.get(type_id)
                && let Some(reflect_component) = registration.data::<ReflectComponent>()
                && let Some(component_ref) = reflect_component.reflect(entity_ref)
            {
                let type_path = registration.type_info().type_path_table().path();
                if let Some(baseline_val) = bl.components.get(type_path) {
                    let serializer = TypedReflectSerializer::new(component_ref, &registry);
                    if let Ok(current_val) = serde_json::to_value(&serializer) {
                        return current_val != *baseline_val;
                    }
                }
            }
            false
        });

        let (display_entity, body_entity) = spawn_component_display(
            commands,
            name,
            source_entity,
            Some(component_id),
            &icon_font.0,
            &editor_font.0,
            is_overridden,
        );
        commands
            .entity(display_entity)
            .insert(ChildOf(group_container));

        // Try Displayable first, then reflection, then fallback
        let type_id = components
            .get_info(component_id)
            .and_then(|info| info.type_id());

        if let Some(type_id) = type_id
            && let Some(registration) = registry.get(type_id)
            && let Some(reflect_component) = registration.data::<ReflectComponent>()
            && let Some(reflected) = reflect_component.reflect(entity_ref)
        {
            // Priority 1: Displayable trait override
            if let Some(reflect_displayable) = registration.data::<ReflectDisplayable>()
                && let Some(displayable) = reflect_displayable.get(reflected)
            {
                let mut body_commands = commands.entity(body_entity);
                displayable.display(&mut body_commands, source_entity);
                continue;
            }

            // Priority 2: MeshMaterial3d<StandardMaterial> — display material fields
            if type_id == TypeId::of::<MeshMaterial3d<StandardMaterial>>() {
                material_display::spawn_material_display_deferred(
                    commands,
                    body_entity,
                    source_entity,
                );
                continue;
            }

            // Priority 3: CustomProperties — specialized property editor
            if type_id == TypeId::of::<CustomProperties>() {
                if let Some(cp) = reflected.downcast_ref::<CustomProperties>() {
                    custom_props_display::spawn_custom_properties_display(
                        commands,
                        body_entity,
                        source_entity,
                        cp,
                        &editor_font.0,
                        &icon_font.0,
                    );
                }
                continue;
            }

            // Priority 3b: Brush — show face/vertex info
            if type_id == TypeId::of::<crate::brush::Brush>() {
                if let Some(brush) = reflected.downcast_ref::<crate::brush::Brush>() {
                    brush_display::spawn_brush_display(commands, body_entity, brush, materials);
                }
                continue;
            }

            // Priority 3c: Terrain — custom inspector sections
            if type_id == TypeId::of::<jackdaw_jsn::Terrain>() {
                crate::terrain::inspector::spawn_terrain_inspector_container(commands, body_entity);
                continue;
            }

            // Priority 3: Generic reflection display
            let full_path = registration.type_info().type_path_table().path();
            reflect_fields::spawn_reflected_fields(
                commands,
                body_entity,
                reflected,
                0,
                String::new(),
                source_entity,
                full_path,
                names,
                type_registry,
                &editor_font.0,
                &icon_font.0,
            );
            continue;
        }

        // Fallback: no reflection data
        commands.spawn((
            Text::new("(read-only)"),
            TextFont {
                font_size: tokens::FONT_SM,
                ..Default::default()
            },
            TextColor(tokens::TEXT_SECONDARY),
            ChildOf(body_entity),
        ));
    }

    if !read_only {
        commands.spawn((
            AddComponentButton,
            jackdaw_feathers::button::button(jackdaw_feathers::button::ButtonProps::new(
                "+ Add Component",
            )),
            ChildOf(inspector_entity),
        ));
    }
}

pub(crate) fn remove_component_displays(
    _: On<Remove, Selected>,
    mut commands: Commands,
    inspector: Single<(Entity, Option<&Children>), With<Inspector>>,
    displays: Query<
        Entity,
        Or<(
            With<ComponentDisplay>,
            With<AddComponentButton>,
            With<ComponentPicker>,
        )>,
    >,
) {
    let (entity, children) = inspector.into_inner();

    // Clean up monitoring components
    commands
        .entity(entity)
        .remove::<(InspectorTarget, Monitor, NotifyAdded<InspectorDirty>)>();

    let Some(children) = children else {
        return;
    };

    for child in displays.iter_many(children.collection()) {
        if let Ok(mut ec) = commands.get_entity(child) {
            ec.despawn();
        }
    }
}

/// Handles `Addition<InspectorDirty>` on the Inspector entity: despawn existing
/// displays and rebuild from the monitored source entity.
pub(crate) fn on_inspector_dirty(
    _: On<Addition<InspectorDirty>>,
    mut commands: Commands,
    components: &Components,
    type_registry: Res<AppTypeRegistry>,
    inspector: Single<(Entity, &InspectorTarget, Option<&Children>), With<Inspector>>,
    entity_query: Query<(&Archetype, EntityRef), Without<EditorEntity>>,
    selection: Res<Selection>,
    names: Query<&Name>,
    icon_font: Res<IconFont>,
    editor_font: Res<EditorFont>,
    displays: Query<
        Entity,
        Or<(
            With<ComponentDisplay>,
            With<AddComponentButton>,
            With<ComponentPicker>,
        )>,
    >,
    materials: Res<Assets<StandardMaterial>>,
    ast: Res<jackdaw_jsn::SceneJsnAst>,
) {
    let (inspector_entity, target, children) = inspector.into_inner();
    let source_entity = target.0;

    // Despawn existing display children
    if let Some(children) = children {
        for child in displays.iter_many(children.collection()) {
            if let Ok(mut ec) = commands.get_entity(child) {
                ec.despawn();
            }
        }
    }

    // Remove InspectorDirty from the source entity
    if let Ok(mut ec) = commands.get_entity(source_entity) {
        ec.remove::<InspectorDirty>();
    }

    // Rebuild
    let Ok((archetype, entity_ref)) = entity_query.get(source_entity) else {
        return;
    };
    let sel_count = selection.entities.len();

    let jsn_type_paths: HashSet<String> = ast
        .node_for_entity(source_entity)
        .map(|node| node.components.keys().cloned().collect())
        .unwrap_or_default();

    build_inspector_displays(
        &mut commands,
        components,
        &type_registry,
        source_entity,
        archetype,
        entity_ref,
        inspector_entity,
        sel_count,
        &names,
        &icon_font,
        &editor_font,
        false,
        &materials,
        &jsn_type_paths,
    );
}

pub(crate) fn spawn_component_display(
    commands: &mut Commands,
    name: &str,
    entity: Entity,
    component: Option<ComponentId>,
    icon_font: &Handle<Font>,
    editor_font: &Handle<Font>,
    is_overridden: bool,
) -> (Entity, Entity) {
    let font = icon_font.clone();
    let body_font = editor_font.clone();

    let body_entity = commands
        .spawn((
            ComponentDisplayBody,
            CollapsibleBody,
            Node {
                padding: UiRect::new(
                    Val::Px(tokens::SPACING_MD),
                    Val::Px(tokens::SPACING_SM),
                    Val::Px(tokens::SPACING_XS),
                    Val::Px(tokens::SPACING_XS),
                ),
                flex_direction: FlexDirection::Column,
                width: Val::Percent(100.0),
                ..Default::default()
            },
        ))
        .id();

    let section_entity = commands
        .spawn((
            ComponentDisplay,
            ComponentName(name.to_string()),
            CollapsibleSection { collapsed: false },
            Node {
                flex_direction: FlexDirection::Column,
                width: Val::Percent(100.0),
                ..Default::default()
            },
        ))
        .id();

    // Header
    let header = commands
        .spawn((
            CollapsibleHeader,
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                width: Val::Percent(100.0),
                padding: UiRect::axes(Val::Px(tokens::SPACING_SM), Val::Px(tokens::SPACING_XS)),
                column_gap: Val::Px(tokens::SPACING_SM),
                ..Default::default()
            },
            BackgroundColor(tokens::PANEL_HEADER_BG),
            ChildOf(section_entity),
        ))
        .id();

    // Toggle area (chevron + title) -- click to collapse/expand
    let toggle_area = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(tokens::SPACING_SM),
                flex_grow: 1.0,
                ..Default::default()
            },
            ChildOf(header),
        ))
        .id();

    // Chevron icon
    commands.spawn((
        Text::new(String::from(Icon::ChevronDown.unicode())),
        TextFont {
            font: font.clone(),
            font_size: tokens::FONT_SM,
            ..Default::default()
        },
        TextColor(tokens::TEXT_SECONDARY),
        ChildOf(toggle_area),
    ));

    // Component name (orange if overridden)
    let name_color = if is_overridden {
        colors::INSPECTOR_OVERRIDE
    } else {
        tokens::TEXT_DISPLAY_COLOR.into()
    };
    commands.spawn((
        Text::new(name.to_string()),
        TextFont {
            font: body_font,
            font_size: tokens::FONT_SM,
            weight: FontWeight::MEDIUM,
            ..Default::default()
        },
        TextColor(name_color),
        ChildOf(toggle_area),
    ));

    // Toggle on click (on toggle area, not on the X button)
    let section = section_entity;
    commands
        .entity(toggle_area)
        .observe(move |_: On<Pointer<Click>>, mut commands: Commands| {
            commands.trigger(ToggleCollapsible { entity: section });
        });

    if let Some(component) = component {
        // Revert button (only shown for overridden prefab components)
        if is_overridden {
            let source_entity = entity;
            commands.spawn((
                Text::new(String::from(Icon::RotateCcw.unicode())),
                TextFont {
                    font: font.clone(),
                    font_size: tokens::FONT_SM,
                    ..Default::default()
                },
                TextColor(colors::INSPECTOR_OVERRIDE),
                ChildOf(header),
                bevy::ui_widgets::observe(move |_: On<Pointer<Click>>, mut commands: Commands| {
                    commands.queue(move |world: &mut World| {
                        revert_component_to_baseline(world, source_entity, component);
                    });
                }),
            ));
        }

        // Remove component button (X icon)
        commands.spawn((
            Text::new(String::from(Icon::X.unicode())),
            TextFont {
                font,
                font_size: tokens::FONT_SM,
                ..Default::default()
            },
            TextColor(tokens::TEXT_SECONDARY),
            ChildOf(header),
            bevy::ui_widgets::observe(move |_: On<Pointer<Click>>, mut commands: Commands| {
                commands.entity(entity).remove_by_id(component);
            }),
        ));
    }

    // Hover effect on header
    commands.entity(header).observe(
        |hover: On<Pointer<Over>>, mut bg: Query<&mut BackgroundColor, With<CollapsibleHeader>>| {
            if let Ok(mut bg) = bg.get_mut(hover.event_target()) {
                bg.0 = tokens::HOVER_BG;
            }
        },
    );
    commands.entity(header).observe(
        |out: On<Pointer<Out>>, mut bg: Query<&mut BackgroundColor, With<CollapsibleHeader>>| {
            if let Ok(mut bg) = bg.get_mut(out.event_target()) {
                bg.0 = tokens::PANEL_HEADER_BG;
            }
        },
    );

    // Attach body to section
    commands.entity(body_entity).insert(ChildOf(section_entity));

    (section_entity, body_entity)
}

/// Filter inspector components based on the search input.
pub(crate) fn filter_inspector_components(
    search_query: Query<&TextEditValue, (With<InspectorSearch>, Changed<TextEditValue>)>,
    components: Query<(Entity, &ComponentName), With<ComponentDisplay>>,
    groups: Query<(Entity, &Children), With<InspectorGroupSection>>,
    mut node_query: Query<&mut Node>,
) {
    let Ok(search) = search_query.single() else {
        return;
    };
    let filter = search.0.trim().to_lowercase();

    // Track which component entities are visible
    let mut visible_components: HashSet<Entity> = HashSet::new();

    // Filter individual component displays by name
    for (entity, comp_name) in &components {
        let matches = filter.is_empty() || comp_name.0.to_lowercase().contains(&filter);

        if let Ok(mut node) = node_query.get_mut(entity) {
            node.display = if matches {
                Display::Flex
            } else {
                Display::None
            };
        }

        if matches {
            visible_components.insert(entity);
        }
    }

    // Hide group sections where all children are hidden
    for (group_entity, children) in &groups {
        let has_visible_child = children
            .iter()
            .any(|child| visible_components.contains(&child));

        if let Ok(mut node) = node_query.get_mut(group_entity) {
            node.display = if filter.is_empty() || has_visible_child {
                Display::Flex
            } else {
                Display::None
            };
        }
    }
}

/// Revert a single component on a prefab instance back to its baseline value.
fn revert_component_to_baseline(world: &mut World, entity: Entity, component_id: ComponentId) {
    use bevy::ecs::reflect::AppTypeRegistry;
    use bevy::reflect::serde::TypedReflectDeserializer;
    use serde::de::DeserializeSeed;

    let Some(baseline) = world.get::<jackdaw_jsn::JsnPrefabBaseline>(entity).cloned() else {
        return;
    };

    let Some(type_id) = world
        .components()
        .get_info(component_id)
        .and_then(|info| info.type_id())
    else {
        return;
    };

    let registry = world.resource::<AppTypeRegistry>().clone();
    let registry = registry.read();

    let Some(registration) = registry.get(type_id) else {
        return;
    };
    let type_path = registration.type_info().type_path_table().path();

    let Some(baseline_val) = baseline.components.get(type_path) else {
        return;
    };

    let Some(reflect_component) = registration.data::<ReflectComponent>() else {
        return;
    };

    let deserializer = TypedReflectDeserializer::new(registration, &registry);
    let Ok(reflected) = deserializer.deserialize(baseline_val) else {
        warn!("Failed to deserialize baseline for '{type_path}'");
        return;
    };

    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        reflect_component.apply(world.entity_mut(entity), reflected.as_ref());
    }));

    drop(registry);

    // Trigger inspector rebuild
    world.entity_mut(entity).insert(InspectorDirty);
}
