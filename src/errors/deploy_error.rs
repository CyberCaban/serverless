use thiserror::Error;

#[derive(Debug, Error)]
pub enum DeployError {
    #[error("General Docker error: {0:?}")]
    DockerGeneral(Vec<bollard::errors::Error>),
}
