use bevy::{picking::hover::Hovered, prelude::*, window::PrimaryWindow};

use crate::{popover, tokens};

pub(super) fn plugin(app: &mut App) {
    app.world_mut().register_component::<Tooltip>();
    app.add_systems(Update, update_toolbar_tooltips)
        .init_resource::<ActiveTooltip>();
}

/// Stores tooltip text for toolbar buttons (used with `Hovered` component).
#[derive(Component)]
pub struct Tooltip(pub String);

#[derive(Resource, Default)]
pub struct ActiveTooltip(pub Option<Entity>);

/// Shows/hides toolbar tooltips based on `Hovered` state (flicker-free).
pub fn update_toolbar_tooltips(
    buttons: Query<(Entity, &Tooltip, &Hovered), Changed<Hovered>>,
    mut commands: Commands,
    mut active: ResMut<ActiveTooltip>,
    window: Single<&Window, With<PrimaryWindow>>,
) {
    // TODO: only show this after hovering for n seconds
    let cursor_pos = window.cursor_position();
    for (entity, tooltip, hovered) in &buttons {
        if hovered.get() {
            if let Some(old) = active.0.take() {
                commands.entity(old).try_despawn();
            }
            let tip = commands
                .spawn(popover::popover(
                    popover::PopoverProps::new(entity)
                        .with_position(cursor_pos)
                        .with_placement(popover::PopoverPlacement::BottomStart)
                        .with_padding(10.0)
                        .with_z_index(300),
                ))
                .id();
            commands.spawn((
                Text::new(tooltip.0.clone()),
                TextFont {
                    font_size: tokens::FONT_SM,
                    ..Default::default()
                },
                TextColor(tokens::TEXT_PRIMARY),
                ChildOf(tip),
            ));
            active.0 = Some(tip);
        } else if let Some(old) = active.0.take() {
            commands.entity(old).try_despawn();
        }
    }
}
