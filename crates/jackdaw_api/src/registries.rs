use std::collections::HashMap;

use bevy::prelude::*;

use crate::SectionBuildFn;
use crate::operator::{Operator, OperatorContext, OperatorResult};

trait OperatorFactory: Send + Sync {
    fn id(&self) -> &str;
    fn label(&self) -> &str;
    fn description(&self) -> &str;
    fn create(&self) -> Box<dyn OperatorInstance>;
}

pub trait OperatorInstance: Send + Sync {
    fn poll(&self, ctx: &OperatorContext) -> bool;
    fn execute(&mut self, ctx: &mut OperatorContext) -> OperatorResult;
    fn invoke(&mut self, ctx: &mut OperatorContext) -> OperatorResult;
}

struct ConcreteFactory<O: Operator + Default>(std::marker::PhantomData<O>);

impl<O: Operator + Default> OperatorFactory for ConcreteFactory<O> {
    fn id(&self) -> &str {
        O::ID
    }
    fn label(&self) -> &str {
        O::LABEL
    }
    fn description(&self) -> &str {
        O::DESCRIPTION
    }
    fn create(&self) -> Box<dyn OperatorInstance> {
        Box::new(O::default())
    }
}

impl<O: Operator> OperatorInstance for O {
    fn poll(&self, ctx: &OperatorContext) -> bool {
        Operator::poll(self, ctx)
    }
    fn execute(&mut self, ctx: &mut OperatorContext) -> OperatorResult {
        Operator::execute(self, ctx)
    }
    fn invoke(&mut self, ctx: &mut OperatorContext) -> OperatorResult {
        Operator::invoke(self, ctx)
    }
}

#[derive(Resource, Default)]
pub struct OperatorRegistry {
    factories: HashMap<String, Box<dyn OperatorFactory>>,
}

impl OperatorRegistry {
    pub fn register<O: Operator + Default>(&mut self) {
        self.factories.insert(
            O::ID.to_string(),
            Box::new(ConcreteFactory::<O>(std::marker::PhantomData)),
        );
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str, &str)> {
        self.factories
            .values()
            .map(|f| (f.id(), f.label(), f.description()))
    }

    pub fn create(&self, id: &str) -> Option<Box<dyn OperatorInstance>> {
        self.factories.get(id).map(|f| f.create())
    }
}

#[derive(Resource, Default)]
pub struct KeybindRegistry {
    bindings: HashMap<KeyCombo, String>,
}

impl KeybindRegistry {
    pub fn bind(&mut self, keys: KeyCombo, operator_id: String) {
        self.bindings.insert(keys, operator_id);
    }

    pub fn lookup(&self, keys: &KeyCombo) -> Option<&str> {
        self.bindings.get(keys).map(|s| s.as_str())
    }

    pub fn iter(&self) -> impl Iterator<Item = (&KeyCombo, &str)> {
        self.bindings.iter().map(|(k, v)| (k, v.as_str()))
    }
}

#[derive(Clone, Hash, Eq, PartialEq)]
pub struct KeyCombo {
    pub key: KeyCode,
    pub modifiers: Modifiers,
}

#[derive(Clone, Copy, Hash, Eq, PartialEq, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl KeyCombo {
    pub fn new(key: KeyCode) -> Self {
        Self {
            key,
            modifiers: Modifiers::default(),
        }
    }

    pub fn ctrl(mut self) -> Self {
        self.modifiers.ctrl = true;
        self
    }

    pub fn shift(mut self) -> Self {
        self.modifiers.shift = true;
        self
    }

    pub fn alt(mut self) -> Self {
        self.modifiers.alt = true;
        self
    }
}

#[derive(Resource, Default)]
pub struct PanelExtensionRegistry {
    extensions: HashMap<String, Vec<SectionBuildFn>>,
}

impl PanelExtensionRegistry {
    pub fn add(&mut self, panel_id: String, section: SectionBuildFn) {
        self.extensions.entry(panel_id).or_default().push(section);
    }

    pub fn get(&self, panel_id: &str) -> &[SectionBuildFn] {
        self.extensions
            .get(panel_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }
}
