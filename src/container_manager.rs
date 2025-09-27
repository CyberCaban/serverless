#![allow(dead_code)]

use std::path::PathBuf;

use anyhow::{Ok, Result, anyhow, bail};
use bollard::{Docker, body_full, query_parameters::BuildImageOptionsBuilder};

use futures_util::StreamExt;
use tokio::io::AsyncReadExt;
pub struct ContainerManager {
    docker: Docker,
}
impl ContainerManager {
    pub fn new() -> Result<Self> {
        let docker = Docker::connect_with_local_defaults()?;
        Ok(Self { docker })
    }

    pub async fn build_image(
        &self,
        context_path: &str,
        image_name: &str,
        dockerfile_path: &str,
    ) -> Result<()> {
        let tar_path = self
            .create_build_context(context_path)
            .await
            .map_err(|e| anyhow!("Failed to create_build_context: {e}"))?;
        let build_image_options = BuildImageOptionsBuilder::new()
            .dockerfile(dockerfile_path)
            .t(image_name)
            .rm(true)
            .build();
        let bytes = Self::tar_to_bytes(&tar_path).await?;
        let mut image =
            self.docker
                .build_image(build_image_options, None, Some(body_full(bytes.into())));
        while let Some(info) = image.next().await {
            if let Err(e) = info {
                bail!("Build failed: {e}")
            }
        }
        Ok(())
    }

    async fn tar_to_bytes(tar_path: &str) -> Result<Vec<u8>> {
        let mut file = tokio::fs::File::open(tar_path).await?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).await?;
        Ok(bytes)
    }

    async fn create_build_context(&self, path: &str) -> Result<String> {
        let temp_dir = if cfg!(target_os = "windows") {
            PathBuf::from(std::env::var("TEMP").unwrap_or_else(|_| "C:\\Windows\\Temp".to_string()))
        } else {
            PathBuf::from("/tmp")
        };
        let tar_path = temp_dir.join(format!("build-context-{}", uuid::Uuid::new_v4()));
        let output = tokio::process::Command::new("tar")
            .arg("-cf")
            .arg(&tar_path)
            .arg("-C")
            .arg(path)
            .arg(".")
            .output()
            .await
            .map_err(|e| anyhow!("Failed to create tar for build context: {e}"))?;
        if !output.status.success() {
            bail!(
                "Tar command failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(tar_path.to_string_lossy().to_string())
    }
}
