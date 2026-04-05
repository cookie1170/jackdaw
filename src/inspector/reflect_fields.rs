use crate::commands::{CommandGroup, CommandHistory, EditorCommand, SetJsnField};
use crate::selection::Selection;

use bevy::{
    ecs::reflect::{AppTypeRegistry, ReflectComponent},
    feathers::theme::ThemedText,
    input_focus::InputFocus,
    prelude::*,
    reflect::ReflectRef,
    ui_widgets::observe,
};
use jackdaw_feathers::{
    checkbox::{CheckboxCommitEvent, CheckboxProps, CheckboxState, checkbox},
    color_picker::{ColorPickerCommitEvent, ColorPickerProps, color_picker},
    combobox::{ComboBoxChangeEvent, combobox_with_selected},
    list_view,
    text_edit::{
        self, TextEditCommitEvent, TextEditConfig, TextEditDragging, TextEditProps, TextEditValue,
        TextEditVariant, TextEditWrapper, TextInputQueue, set_text_input_value,
    },
    tokens,
};

use crate::colors;

use super::{FieldBinding, MAX_REFLECT_DEPTH};

pub(crate) fn spawn_reflected_fields(
    commands: &mut Commands,
    parent: Entity,
    reflected: &dyn Reflect,
    depth: usize,
    base_path: String,
    source_entity: Entity,
    type_path: &str,
    entity_names: &Query<&Name>,
    type_registry: &AppTypeRegistry,
    editor_font: &Handle<Font>,
    icon_font: &Handle<Font>,
) {
    match reflected.reflect_ref() {
        ReflectRef::Struct(s) => {
            for i in 0..s.field_len() {
                let Some(name) = s.name_at(i) else {
                    continue;
                };
                let Some(value) = s.field_at(i) else {
                    continue;
                };
                let child_path = if base_path.is_empty() {
                    name.to_string()
                } else {
                    format!("{base_path}.{name}")
                };
                spawn_field_row(
                    commands,
                    parent,
                    name,
                    value,
                    depth,
                    child_path,
                    source_entity,
                    type_path,
                    entity_names,
                    type_registry,
                    editor_font,
                    icon_font,
                );
            }
        }
        ReflectRef::TupleStruct(ts) => {
            for i in 0..ts.field_len() {
                let Some(value) = ts.field(i) else {
                    continue;
                };
                let child_path = if base_path.is_empty() {
                    format!(".{i}")
                } else {
                    format!("{base_path}.{i}")
                };
                spawn_field_row(
                    commands,
                    parent,
                    &format!("{i}"),
                    value,
                    depth,
                    child_path,
                    source_entity,
                    type_path,
                    entity_names,
                    type_registry,
                    editor_font,
                    icon_font,
                );
            }
        }
        ReflectRef::Enum(e) => {
            spawn_enum_field(
                commands,
                parent,
                e,
                depth,
                base_path,
                source_entity,
                type_path,
                entity_names,
                type_registry,
                editor_font,
                icon_font,
            );
        }
        ReflectRef::List(list) => {
            spawn_list_expansion(
                commands,
                parent,
                list.len(),
                |i| list.get(i),
                depth,
                &base_path,
                source_entity,
                type_path,
                entity_names,
            );
        }
        ReflectRef::Array(array) => {
            spawn_list_expansion(
                commands,
                parent,
                array.len(),
                |i| array.get(i),
                depth,
                &base_path,
                source_entity,
                type_path,
                entity_names,
            );
        }
        ReflectRef::Map(map) => {
            spawn_text_row(
                commands,
                parent,
                &format!("{{ {} entries }}", map.len()),
                depth,
            );
            if !map.is_empty() {
                let lv = commands
                    .spawn((list_view::list_view(), ChildOf(parent)))
                    .id();
                for (i, (key, val)) in map.iter().enumerate() {
                    let item_entity = commands.spawn((list_view::list_item(i), ChildOf(lv))).id();
                    let key_label = format_partial_reflect_value(key);
                    let child_path = if base_path.is_empty() {
                        format!("[{key_label}]")
                    } else {
                        format!("{base_path}[{key_label}]")
                    };
                    spawn_field_row(
                        commands,
                        item_entity,
                        &key_label,
                        val,
                        depth + 1,
                        child_path,
                        source_entity,
                        type_path,
                        entity_names,
                        type_registry,
                        editor_font,
                        icon_font,
                    );
                }
            }
        }
        ReflectRef::Set(set) => {
            spawn_text_row(
                commands,
                parent,
                &format!("{{ {} items }}", set.len()),
                depth,
            );
            if !set.is_empty() {
                let lv = commands
                    .spawn((list_view::list_view(), ChildOf(parent)))
                    .id();
                for (i, item) in set.iter().enumerate() {
                    let item_entity = commands.spawn((list_view::list_item(i), ChildOf(lv))).id();
                    spawn_text_row(
                        commands,
                        item_entity,
                        &format_partial_reflect_value(item),
                        depth + 1,
                    );
                }
            }
        }
        ReflectRef::Tuple(tuple) => {
            for i in 0..tuple.field_len() {
                let Some(value) = tuple.field(i) else {
                    continue;
                };
                let child_path = if base_path.is_empty() {
                    format!(".{i}")
                } else {
                    format!("{base_path}.{i}")
                };
                spawn_field_row(
                    commands,
                    parent,
                    &format!("{i}"),
                    value,
                    depth,
                    child_path,
                    source_entity,
                    type_path,
                    entity_names,
                    type_registry,
                    editor_font,
                    icon_font,
                );
            }
        }
        ReflectRef::Opaque(_) => {
            let label = reflected
                .get_represented_type_info()
                .map(|info| {
                    let path = info.type_path_table().short_path();
                    format!("<{path}>")
                })
                .unwrap_or_else(|| "(opaque)".to_string());
            spawn_text_row(commands, parent, &label, depth);
        }
    }
}

fn is_editable_primitive(value: &dyn PartialReflect) -> bool {
    value.try_downcast_ref::<f32>().is_some()
        || value.try_downcast_ref::<f64>().is_some()
        || value.try_downcast_ref::<i32>().is_some()
        || value.try_downcast_ref::<u32>().is_some()
        || value.try_downcast_ref::<usize>().is_some()
        || value.try_downcast_ref::<i8>().is_some()
        || value.try_downcast_ref::<i16>().is_some()
        || value.try_downcast_ref::<i64>().is_some()
        || value.try_downcast_ref::<u8>().is_some()
        || value.try_downcast_ref::<u16>().is_some()
        || value.try_downcast_ref::<u64>().is_some()
        || value.try_downcast_ref::<bool>().is_some()
        || value.try_downcast_ref::<String>().is_some()
}

fn spawn_field_row(
    commands: &mut Commands,
    parent: Entity,
    name: &str,
    value: &dyn PartialReflect,
    depth: usize,
    field_path: String,
    source_entity: Entity,
    type_path: &str,
    entity_names: &Query<&Name>,
    type_registry: &AppTypeRegistry,
    editor_font: &Handle<Font>,
    icon_font: &Handle<Font>,
) {
    // Entity reference -> clickable link (before any other check)
    if let Some(&entity_val) = value.try_downcast_ref::<Entity>() {
        let left_padding = depth as f32 * tokens::SPACING_MD;
        let row = commands
            .spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(tokens::SPACING_XS),
                    padding: UiRect::left(px(left_padding)),
                    ..Default::default()
                },
                ChildOf(parent),
            ))
            .id();
        commands.spawn((
            Text::new(format!("{name}:")),
            TextFont {
                font_size: tokens::FONT_SM,
                ..Default::default()
            },
            Node {
                min_width: px(20.0),
                flex_shrink: 0.0,
                ..Default::default()
            },
            TextColor(tokens::TYPE_ENTITY),
            ChildOf(row),
        ));
        let label = entity_names
            .get(entity_val)
            .map(|n| format!("{} ({entity_val})", n.as_str()))
            .unwrap_or_else(|_| format!("{entity_val}"));
        spawn_entity_link(commands, row, entity_val, &label);
        return;
    }

    // List/Array -> expand with ListView
    if let ReflectRef::List(list) = value.reflect_ref() {
        spawn_text_row(
            commands,
            parent,
            &format!("{name}: [{} items]", list.len()),
            depth,
        );
        if !list.is_empty() {
            let lv = commands
                .spawn((list_view::list_view(), ChildOf(parent)))
                .id();
            for i in 0..list.len() {
                if let Some(item) = list.get(i) {
                    let item_entity = commands.spawn((list_view::list_item(i), ChildOf(lv))).id();
                    let child_path = if field_path.is_empty() {
                        format!("[{i}]")
                    } else {
                        format!("{field_path}[{i}]")
                    };
                    spawn_list_item_value(
                        commands,
                        item_entity,
                        item,
                        depth + 1,
                        child_path,
                        source_entity,
                        type_path,
                        entity_names,
                    );
                }
            }
        }
        return;
    }
    if let ReflectRef::Array(array) = value.reflect_ref() {
        spawn_text_row(
            commands,
            parent,
            &format!("{name}: [{} items]", array.len()),
            depth,
        );
        if !array.is_empty() {
            let lv = commands
                .spawn((list_view::list_view(), ChildOf(parent)))
                .id();
            for i in 0..array.len() {
                if let Some(item) = array.get(i) {
                    let item_entity = commands.spawn((list_view::list_item(i), ChildOf(lv))).id();
                    let child_path = if field_path.is_empty() {
                        format!("[{i}]")
                    } else {
                        format!("{field_path}[{i}]")
                    };
                    spawn_list_item_value(
                        commands,
                        item_entity,
                        item,
                        depth + 1,
                        child_path,
                        source_entity,
                        type_path,
                        entity_names,
                    );
                }
            }
        }
        return;
    }
    if let ReflectRef::Map(map) = value.reflect_ref() {
        spawn_text_row(
            commands,
            parent,
            &format!("{name}: {{ {} entries }}", map.len()),
            depth,
        );
        if !map.is_empty() {
            let lv = commands
                .spawn((list_view::list_view(), ChildOf(parent)))
                .id();
            for (i, (key, val)) in map.iter().enumerate() {
                let item_entity = commands.spawn((list_view::list_item(i), ChildOf(lv))).id();
                let key_label = format_partial_reflect_value(key);
                let child_path = if field_path.is_empty() {
                    format!("[{key_label}]")
                } else {
                    format!("{field_path}[{key_label}]")
                };
                spawn_field_row(
                    commands,
                    item_entity,
                    &key_label,
                    val,
                    depth + 1,
                    child_path,
                    source_entity,
                    type_path,
                    entity_names,
                    type_registry,
                    editor_font,
                    icon_font,
                );
            }
        }
        return;
    }
    if let ReflectRef::Set(set) = value.reflect_ref() {
        spawn_text_row(
            commands,
            parent,
            &format!("{name}: {{ {} items }}", set.len()),
            depth,
        );
        if !set.is_empty() {
            let lv = commands
                .spawn((list_view::list_view(), ChildOf(parent)))
                .id();
            for (i, item) in set.iter().enumerate() {
                let item_entity = commands.spawn((list_view::list_item(i), ChildOf(lv))).id();
                spawn_text_row(
                    commands,
                    item_entity,
                    &format_partial_reflect_value(item),
                    depth + 1,
                );
            }
        }
        return;
    }

    // Vec3 compact row with colored XYZ labels
    if let Some(vec3) = value.try_downcast_ref::<Vec3>() {
        spawn_vec3_row(
            commands,
            parent,
            name,
            vec3,
            field_path,
            source_entity,
            type_path,
            depth,
        );
        return;
    }

    // Vec2 compact row
    if let Some(vec2) = value.try_downcast_ref::<Vec2>() {
        spawn_vec2_row(
            commands,
            parent,
            name,
            vec2,
            field_path,
            source_entity,
            type_path,
            depth,
        );
        return;
    }

    // Color field with picker
    if let Some(color) = value.try_downcast_ref::<Color>() {
        spawn_color_field(
            commands,
            parent,
            name,
            *color,
            field_path,
            source_entity,
            type_path,
            depth,
        );
        return;
    }

    // Bool toggle
    if let Some(&bool_val) = value.try_downcast_ref::<bool>() {
        spawn_bool_toggle(
            commands,
            parent,
            name,
            bool_val,
            field_path,
            source_entity,
            type_path,
            depth,
            editor_font,
            icon_font,
        );
        return;
    }

    // Numeric fields -> drag input
    if let Some(&v) = value.try_downcast_ref::<f32>() {
        spawn_numeric_field(
            commands,
            parent,
            name,
            v as f64,
            field_path,
            source_entity,
            type_path,
            depth,
        );
        return;
    }
    if let Some(&v) = value.try_downcast_ref::<f64>() {
        spawn_numeric_field(
            commands,
            parent,
            name,
            v,
            field_path,
            source_entity,
            type_path,
            depth,
        );
        return;
    }

    // Integer fields -> numeric input with drag-to-scrub
    if let Some(&v) = value.try_downcast_ref::<i32>() {
        spawn_numeric_field(
            commands,
            parent,
            name,
            v as f64,
            field_path,
            source_entity,
            type_path,
            depth,
        );
        return;
    }
    if let Some(&v) = value.try_downcast_ref::<u32>() {
        spawn_numeric_field(
            commands,
            parent,
            name,
            v as f64,
            field_path,
            source_entity,
            type_path,
            depth,
        );
        return;
    }
    if let Some(&v) = value.try_downcast_ref::<usize>() {
        spawn_numeric_field(
            commands,
            parent,
            name,
            v as f64,
            field_path,
            source_entity,
            type_path,
            depth,
        );
        return;
    }
    if let Some(&v) = value.try_downcast_ref::<i8>() {
        spawn_numeric_field(
            commands,
            parent,
            name,
            v as f64,
            field_path,
            source_entity,
            type_path,
            depth,
        );
        return;
    }
    if let Some(&v) = value.try_downcast_ref::<i16>() {
        spawn_numeric_field(
            commands,
            parent,
            name,
            v as f64,
            field_path,
            source_entity,
            type_path,
            depth,
        );
        return;
    }
    if let Some(&v) = value.try_downcast_ref::<i64>() {
        spawn_numeric_field(
            commands,
            parent,
            name,
            v as f64,
            field_path,
            source_entity,
            type_path,
            depth,
        );
        return;
    }
    if let Some(&v) = value.try_downcast_ref::<u8>() {
        spawn_numeric_field(
            commands,
            parent,
            name,
            v as f64,
            field_path,
            source_entity,
            type_path,
            depth,
        );
        return;
    }
    if let Some(&v) = value.try_downcast_ref::<u16>() {
        spawn_numeric_field(
            commands,
            parent,
            name,
            v as f64,
            field_path,
            source_entity,
            type_path,
            depth,
        );
        return;
    }
    if let Some(&v) = value.try_downcast_ref::<u64>() {
        spawn_numeric_field(
            commands,
            parent,
            name,
            v as f64,
            field_path,
            source_entity,
            type_path,
            depth,
        );
        return;
    }

    // Enum fields -> ComboBox
    if let ReflectRef::Enum(e) = value.reflect_ref() {
        if value.try_as_reflect().is_some() {
            spawn_enum_field(
                commands,
                parent,
                e,
                depth,
                field_path,
                source_entity,
                type_path,
                entity_names,
                type_registry,
                editor_font,
                icon_font,
            );
            return;
        }
        // Fallback: just show variant name
        let text = format!("{name}: {}", e.variant_name());
        spawn_text_row(commands, parent, &text, depth);
        return;
    }

    let is_compound = matches!(
        value.reflect_ref(),
        ReflectRef::Struct(_) | ReflectRef::TupleStruct(_) | ReflectRef::Tuple(_)
    );

    // Check for opaque types that shouldn't be recursed into
    if is_compound && is_opaque_type(value) {
        let text = format!("{name}: {}", format_partial_reflect_value(value));
        spawn_text_row(commands, parent, &text, depth);
        return;
    }

    if depth >= MAX_REFLECT_DEPTH || !is_compound {
        if is_editable_primitive(value) {
            spawn_editable_field(
                commands,
                parent,
                name,
                &format_partial_reflect_value(value),
                field_path,
                source_entity,
                type_path,
                depth,
            );
        } else {
            let text = format!("{name}: {}", format_partial_reflect_value(value));
            spawn_text_row(commands, parent, &text, depth);
        }
    } else {
        // Sub-header + recurse
        spawn_text_row(commands, parent, name, depth);

        let container = commands
            .spawn((Node {
                flex_direction: FlexDirection::Column,
                padding: UiRect::left(px(tokens::SPACING_LG)),
                ..Default::default()
            },))
            .insert(ChildOf(parent))
            .id();

        match value.reflect_ref() {
            ReflectRef::Struct(s) => {
                for i in 0..s.field_len() {
                    let Some(field_name) = s.name_at(i) else {
                        continue;
                    };
                    let Some(field_value) = s.field_at(i) else {
                        continue;
                    };
                    let child_path = format!("{field_path}.{field_name}");
                    spawn_field_row(
                        commands,
                        container,
                        field_name,
                        field_value,
                        depth + 1,
                        child_path,
                        source_entity,
                        type_path,
                        entity_names,
                        type_registry,
                        editor_font,
                        icon_font,
                    );
                }
            }
            ReflectRef::TupleStruct(ts) => {
                for i in 0..ts.field_len() {
                    let Some(field_value) = ts.field(i) else {
                        continue;
                    };
                    let child_path = format!("{field_path}.{i}");
                    spawn_field_row(
                        commands,
                        container,
                        &format!("{i}"),
                        field_value,
                        depth + 1,
                        child_path,
                        source_entity,
                        type_path,
                        entity_names,
                        type_registry,
                        editor_font,
                        icon_font,
                    );
                }
            }
            ReflectRef::Tuple(tuple) => {
                for i in 0..tuple.field_len() {
                    let Some(field_value) = tuple.field(i) else {
                        continue;
                    };
                    let child_path = format!("{field_path}.{i}");
                    spawn_field_row(
                        commands,
                        container,
                        &format!("{i}"),
                        field_value,
                        depth + 1,
                        child_path,
                        source_entity,
                        type_path,
                        entity_names,
                        type_registry,
                        editor_font,
                        icon_font,
                    );
                }
            }
            _ => {}
        }
    }
}

fn spawn_vec3_row(
    commands: &mut Commands,
    parent: Entity,
    name: &str,
    vec3: &Vec3,
    field_path: String,
    source_entity: Entity,
    type_path: &str,
    depth: usize,
) {
    let left_padding = depth as f32 * tokens::SPACING_MD;
    let row = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(tokens::SPACING_XS),
                padding: UiRect::left(px(left_padding)),
                width: Val::Percent(100.0),
                ..Default::default()
            },
            ChildOf(parent),
        ))
        .id();

    // Label
    commands.spawn((
        Text::new(format!("{name}:")),
        TextFont {
            font_size: tokens::FONT_SM,
            ..Default::default()
        },
        Node {
            min_width: px(20.0),
            flex_shrink: 0.0,
            ..Default::default()
        },
        ThemedText,
        ChildOf(row),
    ));

    spawn_axis_input(
        commands,
        row,
        "X",
        vec3.x as f64,
        colors::INSPECTOR_AXIS_X,
        format!("{field_path}.x"),
        source_entity,
        type_path,
    );
    spawn_axis_input(
        commands,
        row,
        "Y",
        vec3.y as f64,
        colors::INSPECTOR_AXIS_Y,
        format!("{field_path}.y"),
        source_entity,
        type_path,
    );
    spawn_axis_input(
        commands,
        row,
        "Z",
        vec3.z as f64,
        colors::INSPECTOR_AXIS_Z,
        format!("{field_path}.z"),
        source_entity,
        type_path,
    );
}

fn spawn_vec2_row(
    commands: &mut Commands,
    parent: Entity,
    name: &str,
    vec2: &Vec2,
    field_path: String,
    source_entity: Entity,
    type_path: &str,
    depth: usize,
) {
    let left_padding = depth as f32 * tokens::SPACING_MD;
    let row = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(tokens::SPACING_XS),
                padding: UiRect::left(px(left_padding)),
                width: Val::Percent(100.0),
                ..Default::default()
            },
            ChildOf(parent),
        ))
        .id();

    commands.spawn((
        Text::new(format!("{name}:")),
        TextFont {
            font_size: tokens::FONT_SM,
            ..Default::default()
        },
        Node {
            min_width: px(20.0),
            flex_shrink: 0.0,
            ..Default::default()
        },
        ThemedText,
        ChildOf(row),
    ));

    spawn_axis_input(
        commands,
        row,
        "X",
        vec2.x as f64,
        colors::INSPECTOR_AXIS_X,
        format!("{field_path}.x"),
        source_entity,
        type_path,
    );
    spawn_axis_input(
        commands,
        row,
        "Y",
        vec2.y as f64,
        colors::INSPECTOR_AXIS_Y,
        format!("{field_path}.y"),
        source_entity,
        type_path,
    );
}

fn spawn_axis_input(
    commands: &mut Commands,
    parent: Entity,
    label: &str,
    value: f64,
    label_color: Color,
    field_path: String,
    source_entity: Entity,
    type_path: &str,
) {
    // Axis label
    commands.spawn((
        Text::new(label),
        TextFont {
            font_size: tokens::FONT_SM,
            ..Default::default()
        },
        TextColor(label_color),
        Node {
            flex_shrink: 0.0,
            ..Default::default()
        },
        ChildOf(parent),
    ));

    // Numeric input
    commands.spawn((
        text_edit::text_edit(
            TextEditProps::default()
                .numeric_f32()
                .grow()
                .with_default_value(value.to_string()),
        ),
        FieldBinding {
            source_entity,
            type_path: type_path.to_string(),
            field_path,
        },
        ChildOf(parent),
    ));
}

fn spawn_bool_toggle(
    commands: &mut Commands,
    parent: Entity,
    name: &str,
    value: bool,
    field_path: String,
    source_entity: Entity,
    type_path: &str,
    depth: usize,
    editor_font: &Handle<Font>,
    icon_font: &Handle<Font>,
) {
    let left_padding = depth as f32 * tokens::SPACING_MD;
    let row = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(tokens::SPACING_XS),
                padding: UiRect::left(px(left_padding)),
                ..Default::default()
            },
            ChildOf(parent),
        ))
        .id();

    commands.spawn((
        Text::new(format!("{name}:")),
        TextFont {
            font: editor_font.clone(),
            font_size: tokens::FONT_SM,
            ..Default::default()
        },
        Node {
            min_width: px(20.0),
            flex_shrink: 0.0,
            ..Default::default()
        },
        TextColor(tokens::TYPE_BOOL),
        ChildOf(row),
    ));

    commands.spawn((
        checkbox(
            CheckboxProps::new("").checked(value),
            editor_font,
            icon_font,
        ),
        FieldBinding {
            source_entity,
            type_path: type_path.to_string(),
            field_path,
        },
        ChildOf(row),
    ));
}

fn spawn_color_field(
    commands: &mut Commands,
    parent: Entity,
    name: &str,
    color: Color,
    field_path: String,
    source_entity: Entity,
    type_path: &str,
    depth: usize,
) {
    let left_padding = depth as f32 * tokens::SPACING_MD;
    let row = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(tokens::SPACING_XS),
                padding: UiRect::left(px(left_padding)),
                ..Default::default()
            },
            ChildOf(parent),
        ))
        .id();

    commands.spawn((
        Text::new(format!("{name}:")),
        TextFont {
            font_size: tokens::FONT_SM,
            ..Default::default()
        },
        Node {
            min_width: px(20.0),
            flex_shrink: 0.0,
            ..Default::default()
        },
        ThemedText,
        ChildOf(row),
    ));

    let srgba = color.to_srgba();
    let rgba = [srgba.red, srgba.green, srgba.blue, srgba.alpha];

    let path = field_path.clone();
    let tp = type_path.to_string();
    commands
        .spawn((
            color_picker(ColorPickerProps::new().with_color(rgba)),
            FieldBinding {
                source_entity,
                type_path: type_path.to_string(),
                field_path,
            },
            ChildOf(row),
        ))
        .observe(
            move |event: On<ColorPickerCommitEvent>, mut commands: Commands| {
                let color = event.color;
                let path = path.clone();
                let tp = tp.clone();
                commands.queue(move |world: &mut World| {
                    apply_color_with_undo(world, source_entity, &tp, &path, color);
                });
            },
        );
}

/// Apply a color change with undo support (propagates to all selected entities).
fn apply_color_with_undo(
    world: &mut World,
    _entity: Entity,
    type_path: &str,
    field_path: &str,
    new_rgba: [f32; 4],
) {
    let registry = world.resource::<AppTypeRegistry>().clone();

    let selection = world.resource::<Selection>();
    let targets: Vec<Entity> = selection.entities.clone();

    let new_json = serde_json::to_value(new_rgba).unwrap_or_default();

    let reg = registry.read();
    let mut sub_commands: Vec<Box<dyn EditorCommand>> = Vec::new();

    for &target in &targets {
        let old_json = world
            .resource::<jackdaw_jsn::SceneJsnAst>()
            .get_component_field(target, type_path, field_path, &reg)
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        sub_commands.push(Box::new(SetJsnField {
            entity: target,
            type_path: type_path.to_string(),
            field_path: field_path.to_string(),
            old_value: old_json,
            new_value: new_json.clone(),
        }));
    }
    drop(reg);

    if sub_commands.is_empty() {
        return;
    }

    let mut cmd: Box<dyn EditorCommand> = if sub_commands.len() == 1 {
        sub_commands.pop().unwrap()
    } else {
        Box::new(CommandGroup {
            label: "Set color on multiple entities".to_string(),
            commands: sub_commands,
        })
    };
    cmd.execute(world);
    let mut history = world.resource_mut::<CommandHistory>();
    history.undo_stack.push(cmd);
    history.redo_stack.clear();
}

fn spawn_numeric_field(
    commands: &mut Commands,
    parent: Entity,
    label: &str,
    value: f64,
    field_path: String,
    source_entity: Entity,
    type_path: &str,
    depth: usize,
) {
    let left_padding = depth as f32 * tokens::SPACING_MD;
    let row = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(tokens::SPACING_XS),
                padding: UiRect::left(px(left_padding)),
                ..Default::default()
            },
            ChildOf(parent),
        ))
        .id();

    commands.spawn((
        Text::new(format!("{label}:")),
        TextFont {
            font_size: tokens::FONT_SM,
            ..Default::default()
        },
        Node {
            min_width: px(20.0),
            flex_shrink: 0.0,
            ..Default::default()
        },
        TextColor(tokens::TYPE_NUMERIC),
        ChildOf(row),
    ));

    commands.spawn((
        text_edit::text_edit(
            TextEditProps::default()
                .numeric_f32()
                .grow()
                .with_default_value(value.to_string()),
        ),
        FieldBinding {
            source_entity,
            type_path: type_path.to_string(),
            field_path,
        },
        ChildOf(row),
    ));
}

fn spawn_editable_field(
    commands: &mut Commands,
    parent: Entity,
    label: &str,
    current_value: &str,
    field_path: String,
    source_entity: Entity,
    type_path: &str,
    depth: usize,
) {
    let left_padding = depth as f32 * tokens::SPACING_MD;

    let row = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(tokens::SPACING_XS),
                padding: UiRect::left(px(left_padding)),
                ..Default::default()
            },
            ChildOf(parent),
        ))
        .id();

    commands.spawn((
        Text::new(format!("{label}:")),
        TextFont {
            font_size: tokens::FONT_SM,
            ..Default::default()
        },
        Node {
            min_width: px(20.0),
            flex_shrink: 0.0,
            ..Default::default()
        },
        ThemedText,
        ChildOf(row),
    ));

    commands.spawn((
        text_edit::text_edit(
            TextEditProps::default()
                .grow()
                .with_default_value(current_value)
                .allow_empty(),
        ),
        FieldBinding {
            source_entity,
            type_path: type_path.to_string(),
            field_path,
        },
        ChildOf(row),
    ));
}

/// Apply a field value change with undo support -- snapshots old value, creates command.
/// Propagates the edit to all selected entities that have the same component.
fn apply_field_value_with_undo(
    world: &mut World,
    _entity: Entity,
    type_path: &str,
    field_path: &str,
    new_value_str: &str,
) {
    let registry = world.resource::<AppTypeRegistry>().clone();

    // Collect all selected entities
    let selection = world.resource::<Selection>();
    let targets: Vec<Entity> = selection.entities.clone();

    let new_json = parse_to_json_value(new_value_str);

    let reg = registry.read();
    let mut sub_commands: Vec<Box<dyn EditorCommand>> = Vec::new();

    for &target in &targets {
        let old_json = world
            .resource::<jackdaw_jsn::SceneJsnAst>()
            .get_component_field(target, type_path, field_path, &reg)
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        sub_commands.push(Box::new(SetJsnField {
            entity: target,
            type_path: type_path.to_string(),
            field_path: field_path.to_string(),
            old_value: old_json,
            new_value: new_json.clone(),
        }));
    }
    drop(reg);

    if sub_commands.is_empty() {
        return;
    }

    let mut cmd: Box<dyn EditorCommand> = if sub_commands.len() == 1 {
        sub_commands.pop().unwrap()
    } else {
        Box::new(CommandGroup {
            label: "Set field on multiple entities".to_string(),
            commands: sub_commands,
        })
    };
    cmd.execute(world);
    let mut history = world.resource_mut::<CommandHistory>();
    history.undo_stack.push(cmd);
    history.redo_stack.clear();
}

/// Parse a text field string to the most appropriate JSON value.
fn parse_to_json_value(s: &str) -> serde_json::Value {
    if let Ok(v) = s.parse::<f64>() {
        serde_json::json!(v)
    } else if let Ok(v) = s.parse::<bool>() {
        serde_json::json!(v)
    } else {
        serde_json::Value::String(s.to_string())
    }
}

/// Parse a string value into a reflected value, returning true on success.
fn spawn_entity_link(commands: &mut Commands, parent: Entity, target: Entity, label: &str) {
    commands.spawn((
        Text::new(label),
        TextFont {
            font_size: tokens::FONT_SM,
            ..Default::default()
        },
        TextColor(tokens::TEXT_ACCENT),
        ChildOf(parent),
        observe(
            move |_: On<Pointer<Click>>,
                  mut commands: Commands,
                  mut selection: ResMut<Selection>| {
                selection.select_single(&mut commands, target);
            },
        ),
        observe(
            move |hover: On<Pointer<Over>>, mut q: Query<&mut TextColor>| {
                if let Ok(mut c) = q.get_mut(hover.event_target()) {
                    c.0 = tokens::TEXT_ACCENT_HOVER;
                }
            },
        ),
        observe(move |out: On<Pointer<Out>>, mut q: Query<&mut TextColor>| {
            if let Ok(mut c) = q.get_mut(out.event_target()) {
                c.0 = tokens::TEXT_ACCENT;
            }
        }),
    ));
}

fn spawn_list_expansion<'a>(
    commands: &mut Commands,
    parent: Entity,
    len: usize,
    get_item: impl Fn(usize) -> Option<&'a dyn PartialReflect>,
    depth: usize,
    base_path: &str,
    source_entity: Entity,
    type_path: &str,
    entity_names: &Query<&Name>,
) {
    spawn_text_row(commands, parent, &format!("[{len} items]"), depth);
    if len == 0 {
        return;
    }
    let lv = commands
        .spawn((list_view::list_view(), ChildOf(parent)))
        .id();
    for i in 0..len {
        if let Some(item) = get_item(i) {
            let item_entity = commands.spawn((list_view::list_item(i), ChildOf(lv))).id();
            let child_path = if base_path.is_empty() {
                format!("[{i}]")
            } else {
                format!("{base_path}[{i}]")
            };
            spawn_list_item_value(
                commands,
                item_entity,
                item,
                depth + 1,
                child_path,
                source_entity,
                type_path,
                entity_names,
            );
        }
    }
}

fn spawn_list_item_value(
    commands: &mut Commands,
    parent: Entity,
    value: &dyn PartialReflect,
    depth: usize,
    field_path: String,
    source_entity: Entity,
    type_path: &str,
    entity_names: &Query<&Name>,
) {
    // Entity -> clickable link
    if let Some(&entity_val) = value.try_downcast_ref::<Entity>() {
        let label = entity_names
            .get(entity_val)
            .map(|n| format!("{} ({entity_val})", n.as_str()))
            .unwrap_or_else(|_| format!("{entity_val}"));
        spawn_entity_link(commands, parent, entity_val, &label);
        return;
    }
    // Editable primitive -> inline text input
    if is_editable_primitive(value) {
        spawn_inline_editable(
            commands,
            parent,
            &format_partial_reflect_value(value),
            field_path,
            source_entity,
            type_path,
        );
        return;
    }
    // Compound -> recurse (list items don't have type_registry context, show as text)
    if let Some(reflected) = value.try_as_reflect() {
        // For list items we don't have type_registry context, so show text
        let text = format_reflect_value(reflected);
        spawn_text_row(commands, parent, &text, depth);
        return;
    }
    // Fallback -> plain text
    spawn_text_row(
        commands,
        parent,
        &format_partial_reflect_value(value),
        depth,
    );
}

fn spawn_inline_editable(
    commands: &mut Commands,
    parent: Entity,
    current_value: &str,
    field_path: String,
    source_entity: Entity,
    type_path: &str,
) {
    commands.spawn((
        text_edit::text_edit(
            TextEditProps::default()
                .grow()
                .with_default_value(current_value)
                .allow_empty(),
        ),
        FieldBinding {
            source_entity,
            type_path: type_path.to_string(),
            field_path,
        },
        ChildOf(parent),
    ));
}

fn spawn_text_row(commands: &mut Commands, parent: Entity, text: &str, depth: usize) {
    let left_padding = depth as f32 * tokens::SPACING_MD;
    commands.spawn((
        Node {
            padding: UiRect::left(px(left_padding)),
            ..Default::default()
        },
        Text::new(text),
        TextFont {
            font_size: tokens::FONT_SM,
            ..Default::default()
        },
        ThemedText,
        ChildOf(parent),
    ));
}

fn format_reflect_value(value: &dyn Reflect) -> String {
    format_partial_reflect_value(value.as_partial_reflect())
}

fn format_partial_reflect_value(value: &dyn PartialReflect) -> String {
    if let Some(v) = value.try_downcast_ref::<f32>() {
        return format!("{v:.3}");
    }
    if let Some(v) = value.try_downcast_ref::<f64>() {
        return format!("{v:.3}");
    }
    if let Some(v) = value.try_downcast_ref::<bool>() {
        return format!("{v}");
    }
    if let Some(v) = value.try_downcast_ref::<String>() {
        return format!("\"{v}\"");
    }
    if let Some(v) = value.try_downcast_ref::<i32>() {
        return format!("{v}");
    }
    if let Some(v) = value.try_downcast_ref::<u32>() {
        return format!("{v}");
    }
    if let Some(v) = value.try_downcast_ref::<usize>() {
        return format!("{v}");
    }
    if let Some(v) = value.try_downcast_ref::<i8>() {
        return format!("{v}");
    }
    if let Some(v) = value.try_downcast_ref::<i16>() {
        return format!("{v}");
    }
    if let Some(v) = value.try_downcast_ref::<i64>() {
        return format!("{v}");
    }
    if let Some(v) = value.try_downcast_ref::<u8>() {
        return format!("{v}");
    }
    if let Some(v) = value.try_downcast_ref::<u16>() {
        return format!("{v}");
    }
    if let Some(v) = value.try_downcast_ref::<u64>() {
        return format!("{v}");
    }

    // Handle<T> and other opaque types -> clean format
    if is_opaque_type(value) {
        return "<opaque>".to_string();
    }

    // Fallback: show type name if available, otherwise Debug
    if let Some(info) = value.get_represented_type_info() {
        return format!("<{}>", info.type_path_table().short_path());
    }
    format!("{value:?}")
}

/// Handle TextEditCommitEvent for inspector field bindings (numeric and string fields).
pub(crate) fn on_text_edit_commit(
    event: On<TextEditCommitEvent>,
    bindings: Query<(&FieldBinding, Option<&TextEditVariant>)>,
    child_of_query: Query<&ChildOf>,
    mut commands: Commands,
    remote_proxies: Query<(), With<crate::remote::entity_browser::RemoteEntityProxy>>,
) {
    // Walk up from the committed entity to find a FieldBinding
    let mut current = event.entity;
    let mut found = None;
    for _ in 0..4 {
        let Ok(child_of) = child_of_query.get(current) else {
            break;
        };
        if let Ok((binding, variant)) = bindings.get(child_of.parent()) {
            found = Some((
                binding.source_entity,
                binding.type_path.clone(),
                binding.field_path.clone(),
                variant.copied(),
            ));
            break;
        }
        current = child_of.parent();
    }

    let Some((source_entity, tp, path, variant)) = found else {
        return;
    };

    // Skip edits targeting remote proxy entities (read-only inspector)
    if remote_proxies.contains(source_entity) {
        return;
    }

    // For numeric fields, use the text as-is (already formatted)
    // For string fields, use text directly
    let value_str = if variant.is_some_and(|v| v.is_numeric()) {
        // Parse and re-format to ensure consistent value
        let val: f64 = event.text.parse().unwrap_or(0.0);
        format!("{val}")
    } else {
        event.text.clone()
    };

    commands.queue(move |world: &mut World| {
        apply_field_value_with_undo(world, source_entity, &tp, &path, &value_str);
    });
}

pub(crate) fn on_checkbox_commit(
    event: On<CheckboxCommitEvent>,
    bindings: Query<&FieldBinding>,
    mut commands: Commands,
    remote_proxies: Query<(), With<crate::remote::entity_browser::RemoteEntityProxy>>,
) {
    let Ok(binding) = bindings.get(event.entity) else {
        return;
    };
    let source = binding.source_entity;

    // Skip edits targeting remote proxy entities (read-only inspector)
    if remote_proxies.contains(source) {
        return;
    }
    let tp = binding.type_path.clone();
    let path = binding.field_path.clone();
    let val = format!("{}", event.checked);
    commands.queue(move |world: &mut World| {
        apply_field_value_with_undo(world, source, &tp, &path, &val);
    });
}

/// Refreshes inspector field values using reflection -- handles all component types generically.
/// Uses exclusive world access to avoid query conflicts.
pub(crate) fn refresh_inspector_fields(world: &mut World) {
    let selection = world.resource::<Selection>();
    let Some(primary) = selection.primary() else {
        return;
    };

    let type_registry = world.resource::<AppTypeRegistry>().clone();
    let registry = type_registry.read();

    // Collect numeric binding info: outer entity + current TextEditValue
    let mut numeric_lookups: Vec<(Entity, String, String, String)> = Vec::new();
    let mut query = world.query::<(Entity, &FieldBinding, &TextEditValue, &TextEditConfig)>();
    for (entity, binding, value, config) in query.iter(world) {
        if binding.source_entity == primary && config.variant.is_numeric() {
            numeric_lookups.push((
                entity,
                binding.type_path.clone(),
                binding.field_path.clone(),
                value.0.clone(),
            ));
        }
    }

    // Collect checkbox binding info and current state
    let mut bool_lookups: Vec<(Entity, String, String, bool)> = Vec::new();
    let mut checkbox_query = world.query::<(Entity, &FieldBinding, &CheckboxState)>();
    for (entity, binding, state) in checkbox_query.iter(world) {
        if binding.source_entity == primary {
            bool_lookups.push((
                entity,
                binding.type_path.clone(),
                binding.field_path.clone(),
                state.checked,
            ));
        }
    }

    if numeric_lookups.is_empty() && bool_lookups.is_empty() {
        return;
    }

    // Read reflected values and compute updates
    // For numeric fields: we need to find inner EditorTextEdit entity and set its value
    let mut numeric_updates: Vec<(Entity, f64)> = Vec::new();
    let mut bool_updates: Vec<(Entity, bool)> = Vec::new();
    let Ok(entity_ref) = world.get_entity(primary) else {
        return;
    };

    for (ui_entity, comp_type_path, field_path, current_text) in &numeric_lookups {
        let Some(registration) = registry.get_with_type_path(comp_type_path) else {
            continue;
        };
        let Some(reflect_component) = registration.data::<ReflectComponent>() else {
            continue;
        };
        let Some(reflected) = reflect_component.reflect(entity_ref) else {
            continue;
        };
        let Ok(field) = reflected.reflect_path(field_path.as_str()) else {
            continue;
        };
        let value = reflect_field_to_f64(field);
        let Some(value) = value else {
            continue;
        };

        let current_val: f64 = current_text.parse().unwrap_or(0.0);
        if (current_val - value).abs() > 0.005 {
            numeric_updates.push((*ui_entity, value));
        }
    }

    for (ui_entity, comp_type_path, field_path, current_checked) in &bool_lookups {
        let Some(registration) = registry.get_with_type_path(comp_type_path) else {
            continue;
        };
        let Some(reflect_component) = registration.data::<ReflectComponent>() else {
            continue;
        };
        let Some(reflected) = reflect_component.reflect(entity_ref) else {
            continue;
        };
        let Ok(field) = reflected.reflect_path(field_path.as_str()) else {
            continue;
        };
        if let Some(&val) = field.try_downcast_ref::<bool>() {
            if val != *current_checked {
                bool_updates.push((*ui_entity, val));
            }
        }
    }

    drop(registry);

    // Apply numeric updates: find inner EditorTextEdit entity and use set_text_input_value
    let input_focus = world.resource::<InputFocus>().0;
    for (outer_entity, value) in numeric_updates {
        // Walk: outer (TextEditConfig) → children → wrapper (TextEditWrapper) → inner entity
        let Some((wrapper_entity, inner_entity)) = find_text_edit_entities(world, outer_entity)
        else {
            continue;
        };

        // Skip if the field is being drag-adjusted or the user is typing in it
        if world.get::<TextEditDragging>(wrapper_entity).is_some() {
            continue;
        }
        if input_focus == Some(inner_entity) {
            continue;
        }

        if let Some(variant) = world.get::<TextEditVariant>(inner_entity).copied() {
            let formatted = text_edit::format_numeric_value(value, variant);
            if let Some(mut queue) = world.get_mut::<TextInputQueue>(inner_entity) {
                set_text_input_value(&mut queue, formatted);
            }
        }
    }

    // Apply bool updates (sync_checkbox_icon handles the visual update)
    for (entity, value) in bool_updates {
        if let Some(mut state) = world.get_mut::<CheckboxState>(entity) {
            state.checked = value;
        }
    }
}

/// Walk from an outer text_edit entity to find the wrapper and inner EditorTextEdit entities.
/// Returns (wrapper_entity, inner_entity).
fn find_text_edit_entities(world: &World, outer_entity: Entity) -> Option<(Entity, Entity)> {
    let children = world.get::<Children>(outer_entity)?;
    for child in children.iter() {
        if let Some(wrapper) = world.get::<TextEditWrapper>(child) {
            return Some((child, wrapper.0));
        }
        if let Some(grandchildren) = world.get::<Children>(child) {
            for gc in grandchildren.iter() {
                if let Some(wrapper) = world.get::<TextEditWrapper>(gc) {
                    return Some((gc, wrapper.0));
                }
            }
        }
    }
    None
}

fn reflect_field_to_f64(field: &dyn PartialReflect) -> Option<f64> {
    if let Some(&v) = field.try_downcast_ref::<f32>() {
        Some(v as f64)
    } else if let Some(&v) = field.try_downcast_ref::<f64>() {
        Some(v)
    } else if let Some(&v) = field.try_downcast_ref::<i32>() {
        Some(v as f64)
    } else if let Some(&v) = field.try_downcast_ref::<u32>() {
        Some(v as f64)
    } else if let Some(&v) = field.try_downcast_ref::<usize>() {
        Some(v as f64)
    } else if let Some(&v) = field.try_downcast_ref::<i8>() {
        Some(v as f64)
    } else if let Some(&v) = field.try_downcast_ref::<i16>() {
        Some(v as f64)
    } else if let Some(&v) = field.try_downcast_ref::<i64>() {
        Some(v as f64)
    } else if let Some(&v) = field.try_downcast_ref::<u8>() {
        Some(v as f64)
    } else if let Some(&v) = field.try_downcast_ref::<u16>() {
        Some(v as f64)
    } else if let Some(&v) = field.try_downcast_ref::<u64>() {
        Some(v as f64)
    } else {
        None
    }
}

/// Check if a type is opaque and shouldn't be recursed into for inspection.
fn is_opaque_type(value: &dyn PartialReflect) -> bool {
    let Some(type_info) = value.get_represented_type_info() else {
        return false;
    };
    let type_path = type_info.type_path();
    type_path.starts_with("bevy_asset::handle::Handle")
        || type_path.starts_with("bevy_asset::id::AssetId")
        || type_path.contains("Cow<")
}

/// Spawn a ComboBox for enum fields, supporting unit-only enums with undo.
fn spawn_enum_field(
    commands: &mut Commands,
    parent: Entity,
    enum_ref: &dyn bevy::reflect::Enum,
    depth: usize,
    field_path: String,
    source_entity: Entity,
    type_path: &str,
    entity_names: &Query<&Name>,
    type_registry: &AppTypeRegistry,
    editor_font: &Handle<Font>,
    icon_font: &Handle<Font>,
) {
    let current_variant = enum_ref.variant_name().to_string();

    // Try to get variant names from type info
    let Some(type_info) = enum_ref.get_represented_type_info() else {
        spawn_text_row(
            commands,
            parent,
            &format!("variant: {current_variant}"),
            depth,
        );
        return;
    };
    let bevy::reflect::TypeInfo::Enum(enum_info) = type_info else {
        spawn_text_row(
            commands,
            parent,
            &format!("variant: {current_variant}"),
            depth,
        );
        return;
    };

    let variant_names: Vec<String> = enum_info
        .variant_names()
        .iter()
        .map(|n| n.to_string())
        .collect();

    if variant_names.is_empty() {
        spawn_text_row(
            commands,
            parent,
            &format!("variant: {current_variant}"),
            depth,
        );
        return;
    }

    let selected_index = variant_names
        .iter()
        .position(|n| n == &current_variant)
        .unwrap_or(0);

    // Check if all variants are unit variants
    let all_unit = (0..enum_info.variant_len()).all(|i| {
        enum_info
            .variant_at(i)
            .map(|v| matches!(v, bevy::reflect::VariantInfo::Unit(_)))
            .unwrap_or(false)
    });

    let left_padding = depth as f32 * tokens::SPACING_MD;

    if all_unit {
        // Simple ComboBox for unit-only enums
        let row = commands
            .spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(tokens::SPACING_XS),
                    padding: UiRect::left(px(left_padding)),
                    width: Val::Percent(100.0),
                    ..Default::default()
                },
                ChildOf(parent),
            ))
            .id();

        let path = field_path.clone();
        let tp = type_path.to_string();
        commands
            .spawn((
                combobox_with_selected(variant_names, selected_index),
                FieldBinding {
                    source_entity,
                    type_path: type_path.to_string(),
                    field_path,
                },
                ChildOf(row),
            ))
            .observe(
                move |event: On<ComboBoxChangeEvent>, mut commands: Commands| {
                    let variant_name = event.label.clone();
                    let path = path.clone();
                    let tp = tp.clone();
                    commands.queue(move |world: &mut World| {
                        apply_enum_variant_with_undo(
                            world,
                            source_entity,
                            &tp,
                            &path,
                            &variant_name,
                        );
                    });
                },
            );
    } else {
        // Show ComboBox for variant selection + recurse into fields for data-carrying variants
        let container = commands
            .spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::left(px(left_padding)),
                    row_gap: px(tokens::SPACING_XS),
                    width: Val::Percent(100.0),
                    ..Default::default()
                },
                ChildOf(parent),
            ))
            .id();

        let path = field_path.clone();
        let tp = type_path.to_string();
        commands
            .spawn((
                combobox_with_selected(variant_names, selected_index),
                FieldBinding {
                    source_entity,
                    type_path: type_path.to_string(),
                    field_path: field_path.clone(),
                },
                ChildOf(container),
            ))
            .observe(
                move |event: On<ComboBoxChangeEvent>, mut commands: Commands| {
                    let variant_name = event.label.clone();
                    let path = path.clone();
                    let tp = tp.clone();
                    commands.queue(move |world: &mut World| {
                        apply_enum_variant_with_undo(
                            world,
                            source_entity,
                            &tp,
                            &path,
                            &variant_name,
                        );
                    });
                },
            );

        // Recurse into current variant's fields (if it has any)
        let variant_field_count = enum_ref.field_len();
        if variant_field_count > 0 {
            for i in 0..variant_field_count {
                let Some(field_value) = enum_ref.field_at(i) else {
                    continue;
                };
                let field_name = enum_ref
                    .name_at(i)
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| format!("{i}"));
                let child_path = if field_path.is_empty() {
                    field_name.clone()
                } else {
                    format!("{field_path}.{field_name}")
                };
                spawn_field_row(
                    commands,
                    container,
                    &field_name,
                    field_value,
                    depth + 1,
                    child_path,
                    source_entity,
                    type_path,
                    entity_names,
                    type_registry,
                    editor_font,
                    icon_font,
                );
            }
        }
    }
}

/// Apply an enum variant change with undo support.
fn apply_enum_variant_with_undo(
    world: &mut World,
    _entity: Entity,
    type_path: &str,
    field_path: &str,
    variant_name: &str,
) {
    let registry = world.resource::<AppTypeRegistry>().clone();

    let selection = world.resource::<Selection>();
    let targets: Vec<Entity> = selection.entities.clone();

    let new_json = serde_json::Value::String(variant_name.to_string());

    let reg = registry.read();
    let mut sub_commands: Vec<Box<dyn EditorCommand>> = Vec::new();

    for &target in &targets {
        let old_json = world
            .resource::<jackdaw_jsn::SceneJsnAst>()
            .get_component_field(target, type_path, field_path, &reg)
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        sub_commands.push(Box::new(SetJsnField {
            entity: target,
            type_path: type_path.to_string(),
            field_path: field_path.to_string(),
            old_value: old_json,
            new_value: new_json.clone(),
        }));
    }
    drop(reg);

    if sub_commands.is_empty() {
        return;
    }

    let mut cmd: Box<dyn EditorCommand> = if sub_commands.len() == 1 {
        sub_commands.pop().unwrap()
    } else {
        Box::new(CommandGroup {
            label: "Set enum on multiple entities".to_string(),
            commands: sub_commands,
        })
    };
    cmd.execute(world);
    let mut history = world.resource_mut::<CommandHistory>();
    history.undo_stack.push(cmd);
    history.redo_stack.clear();
}
