use thiserror::Error;

#[derive(Debug, Error)]
pub enum FunctionError {
    #[error("Function not deployed")]
    FunctionNotDeployed,
    #[error("No running containers")]
    NoRunningContainers,
}
