use bevy::prelude::*;
use jackdaw_api::{
    KeyCombo, KeybindRegistry, Modifiers, OperatorContext, OperatorRegistry, PanelExtensionRegistry,
};

pub struct ExtensionLoaderPlugin;

impl Plugin for ExtensionLoaderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<OperatorRegistry>()
            .init_resource::<KeybindRegistry>()
            .init_resource::<PanelExtensionRegistry>()
            .add_systems(
                Update,
                keybind_dispatch_system
                    .run_if(in_state(crate::AppState::Editor))
                    .run_if(crate::no_dialog_open),
            );
    }
}

fn keybind_dispatch_system(world: &mut World) {
    let keyboard = world.resource::<ButtonInput<KeyCode>>();
    let ctrl = keyboard.pressed(KeyCode::ControlLeft) || keyboard.pressed(KeyCode::ControlRight);
    let shift = keyboard.pressed(KeyCode::ShiftLeft) || keyboard.pressed(KeyCode::ShiftRight);
    let alt = keyboard.pressed(KeyCode::AltLeft) || keyboard.pressed(KeyCode::AltRight);
    let modifiers = Modifiers { ctrl, shift, alt };

    let just_pressed: Vec<KeyCode> = keyboard.get_just_pressed().copied().collect();
    if just_pressed.is_empty() {
        return;
    }

    let mut matched_operator_id = None;
    let keybinds = world.resource::<KeybindRegistry>();
    for key in &just_pressed {
        let combo = KeyCombo {
            key: *key,
            modifiers,
        };
        if let Some(op_id) = keybinds.lookup(&combo) {
            matched_operator_id = Some(op_id.to_string());
            break;
        }
    }

    let Some(operator_id) = matched_operator_id else {
        return;
    };

    let operators = world.resource::<OperatorRegistry>();
    let Some(mut operator) = operators.create(&operator_id) else {
        warn!("Keybind references unknown operator: {operator_id}");
        return;
    };

    let label = operator_id.clone();
    let mut ctx = OperatorContext::new(world, true);

    if !operator.poll(&ctx) {
        return;
    }

    let result = operator.invoke(&mut ctx);
    match result {
        jackdaw_api::OperatorResult::Finished => {
            ctx.finish(&label);
        }
        jackdaw_api::OperatorResult::Cancelled => {}
        jackdaw_api::OperatorResult::Running => {
            // TODO: modal operator support
            ctx.finish(&label);
        }
    }
}
