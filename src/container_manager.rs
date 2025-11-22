#![allow(dead_code)]

use std::result::Result::Ok;
use std::{collections::HashMap, path::PathBuf};

use anyhow::{Result, anyhow, bail};
use bollard::query_parameters::{ListNetworksOptions, ListVolumesOptions};
use bollard::secret::{Mount, NetworkCreateRequest, VolumeCreateOptions};
use bollard::{
    Docker, body_full,
    exec::StartExecOptions,
    query_parameters::{
        self, BuildImageOptionsBuilder, CreateContainerOptionsBuilder,
        RemoveContainerOptionsBuilder,
    },
    secret::{ContainerCreateBody, ExecConfig, HostConfig, PortBinding},
};
use futures_util::StreamExt;
use tokio::io::AsyncReadExt;

use crate::function_manager::FunctionConfig;

const MB_TO_BYTES: i64 = 1024 * 1024;

type ContainerId = String;
#[derive(Debug)]
pub struct ContainerManager {
    docker: Docker,
}
impl ContainerManager {
    pub fn new() -> Result<Self> {
        let docker = Docker::connect_with_local_defaults()?;
        Ok(Self { docker })
    }

    pub async fn try_exec(&self, container_id: &str) -> Result<()> {
        let curl_command = [
            "curl",
            "-s",
            "-X",
            "POST",
            "http://localhost:3000/",
            "-H",
            "Content-Type: application/json",
            "-d",
            r#"{ "name": "test" }"#,
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        let config = ExecConfig {
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            cmd: Some(curl_command),
            ..Default::default()
        };

        let exec = self.docker.create_exec(container_id, config).await?;
        let output = self
            .docker
            .start_exec(&exec.id, None::<StartExecOptions>)
            .await?;

        if let bollard::exec::StartExecResults::Attached { mut output, .. } = output {
            while let Some(log_output) = output.next().await {
                match log_output {
                    Ok(msg) => {
                        println!("{}", String::from_utf8_lossy(&msg.into_bytes()));
                    }
                    Err(e) => {
                        eprintln!("Error: {}", e)
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn remove_container(&self, container_id: &str) {
        let options = RemoveContainerOptionsBuilder::new().force(true).build();
        let _ = self
            .docker
            .remove_container(container_id, Some(options))
            .await;
    }

    pub async fn is_created(&self, container_id: &str) -> bool {
        self.docker
            .inspect_container(
                container_id,
                None::<query_parameters::InspectContainerOptions>,
            )
            .await
            .unwrap()
            .created
            .is_some()
    }

    pub async fn start_container(&self, container_id: &str) -> Result<()> {
        self.docker
            .start_container(
                container_id,
                None::<bollard::query_parameters::StartContainerOptions>,
            )
            .await?;
        Ok(())
    }

    pub fn container_name_from_image_name(image_name: &str) -> String {
        format!(
            "function-{}-{}",
            image_name.replace(':', "-"),
            uuid::Uuid::now_v7()
        )
    }

    pub fn image_name_from_container_id(image_name: &str, container_id: &str) -> String {
        format!("function-{}-{}", image_name.replace(":", "-"), container_id)
    }

    pub async fn create_container(
        &self,
        function_config: &FunctionConfig,
        image_name: &str,
        port: u16,
    ) -> Result<ContainerId> {
        let name = ContainerManager::container_name_from_image_name(image_name);
        let options = CreateContainerOptionsBuilder::new().name(&name).build();
        let config = ContainerCreateBody {
            image: Some(String::from(image_name)),
            host_config: Some(HostConfig {
                port_bindings: Some({
                    let mut bindings = HashMap::new();
                    bindings.insert(
                        function_config.inner_port.to_string(),
                        Some(vec![PortBinding {
                            host_ip: Some("0.0.0.0".to_string()),
                            host_port: Some(port.to_string()),
                        }]),
                    );
                    bindings
                }),
                memory: Some(function_config.memory * MB_TO_BYTES),
                ..Default::default()
            }),
            ..Default::default()
        };
        let response = self.docker.create_container(Some(options), config).await?;
        Ok(response.id)
    }

    async fn create_network_if_not_exists(&self, name: &str) -> Result<()> {
        let networks = self
            .docker
            .list_networks(None::<ListNetworksOptions>)
            .await?;
        if networks.iter().any(|n| n.name == Some(name.to_string())) {
            return Ok(());
        }
        let config = NetworkCreateRequest {
            name: name.to_string(),
            ..Default::default()
        };
        self.docker.create_network(config).await?;
        Ok(())
    }

    async fn create_shared_volume_if_not_exists(&self, volume_name: &str) -> Result<()> {
        let list_volumes_options = ListVolumesOptions {
            filters: Some(HashMap::from([(
                "name".to_string(),
                vec![volume_name.to_string()],
            )])),
        };
        let volumes = self.docker.list_volumes(Some(list_volumes_options)).await?;
        if volumes.volumes.is_some() {
            return Ok(());
        }
        let config = VolumeCreateOptions {
            name: Some(volume_name.to_string()),
            ..Default::default()
        };
        self.docker.create_volume(config).await?;
        Ok(())
    }

    pub async fn create_container_config(&self, image_name: &str) -> Result<ContainerCreateBody> {
        let network_name = image_name;
        self.create_network_if_not_exists(network_name).await?;
        self.create_shared_volume_if_not_exists(image_name).await?;
        let mounts = vec![Mount {
            target: Some("/shared_data".to_string()),
            source: Some(image_name.to_string()),
            typ: Some(bollard::secret::MountTypeEnum::VOLUME),
            ..Default::default()
        }];
        let host_config = HostConfig {
            mounts: Some(mounts),
            network_mode: Some(network_name.to_string()),
            ..Default::default()
        };
        Ok(ContainerCreateBody {
            image: Some(image_name.to_string()),
            host_config: Some(host_config),
            ..Default::default()
        })
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
        let tar_path = temp_dir.join(format!("build-context-{}", uuid::Uuid::now_v7()));
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
