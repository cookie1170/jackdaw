use bevy::prelude::*;
use jackdaw_commands::{CommandGroup, CommandHistory, EditorCommand};

pub trait Operator: Send + Sync + 'static {
    const ID: &'static str;
    const LABEL: &'static str;
    const DESCRIPTION: &'static str = "";

    fn poll(&self, ctx: &OperatorContext) -> bool {
        let _ = ctx;
        true
    }

    fn execute(&mut self, ctx: &mut OperatorContext) -> OperatorResult;

    fn invoke(&mut self, ctx: &mut OperatorContext) -> OperatorResult {
        self.execute(ctx)
    }
}

pub enum OperatorResult {
    Finished,
    Cancelled,
    Running,
}

pub struct OperatorContext<'a> {
    world: &'a mut World,
    recorded_commands: Vec<Box<dyn EditorCommand>>,
    creates_history_entry: bool,
}

impl<'a> OperatorContext<'a> {
    pub fn new(world: &'a mut World, creates_history_entry: bool) -> Self {
        Self {
            world,
            recorded_commands: Vec::new(),
            creates_history_entry,
        }
    }

    pub fn world(&self) -> &World {
        self.world
    }

    pub fn world_mut(&mut self) -> &mut World {
        self.world
    }

    pub fn execute_command(&mut self, mut cmd: Box<dyn EditorCommand>) {
        cmd.execute(self.world);
        self.recorded_commands.push(cmd);
    }

    pub fn invoke_operator<O: Operator + Default>(&mut self) -> OperatorResult {
        let mut op = O::default();
        let mut nested_ctx = OperatorContext {
            world: self.world,
            recorded_commands: Vec::new(),
            creates_history_entry: false,
        };
        let result = op.invoke(&mut nested_ctx);
        self.recorded_commands.extend(nested_ctx.recorded_commands);
        result
    }

    pub fn finish(self, operator_label: &str) {
        if !self.creates_history_entry || self.recorded_commands.is_empty() {
            return;
        }
        let group = CommandGroup {
            commands: self.recorded_commands,
            label: operator_label.to_string(),
        };
        let mut history = self.world.resource_mut::<CommandHistory>();
        history.push_executed(Box::new(group));
    }
}
