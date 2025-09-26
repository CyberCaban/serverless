#![allow(dead_code)]
use bollard::Docker;

pub struct ContainerManager {
    docker: Docker,
}

impl ContainerManager {
    pub async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let docker = Docker::connect_with_local_defaults()?;
        Ok(Self { docker })
    }
}
