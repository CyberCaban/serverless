use std::{
    collections::HashMap,
    ops::{Deref, DerefMut},
    sync::Arc,
};

use tokio::sync::RwLock;

use crate::function_manager::RunningFunction;

type FunctionName = String;
#[derive(Debug)]
pub struct DeployedFunctions(Arc<RwLock<HashMap<FunctionName, RunningFunction>>>);
impl Deref for DeployedFunctions {
    type Target = Arc<RwLock<HashMap<FunctionName, RunningFunction>>>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl DerefMut for DeployedFunctions {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl DeployedFunctions {
    pub fn new() -> Self {
        Self(Arc::new(RwLock::new(HashMap::new())))
    }
}
