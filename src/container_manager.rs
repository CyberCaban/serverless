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
use log::{error, info, warn};
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;

use crate::errors::deploy_error::DeployError;
use crate::function_manager::FunctionConfig;

const MB_TO_BYTES: i64 = 1024 * 1024;
pub const MANAGED_CONTAINER_LABEL: &str = "serverless.managed=true";

fn managed_container_labels() -> HashMap<String, String> {
    HashMap::from([(
        "serverless.managed".to_string(),
        "true".to_string(),
    )])
}

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

    pub async fn try_invoke_http(
        &self,
        container_id: &str,
        inner_port: u16,
        payload: &Value,
    ) -> Result<Value> {
        self.try_invoke_with_exec(container_id, inner_port, payload).await
    }

    async fn try_invoke_with_exec(
        &self,
        container_id: &str,
        inner_port: u16,
        payload: &Value,
    ) -> Result<Value> {
        let payload_json = serde_json::to_string(payload)?;
        let url = format!("http://localhost:{inner_port}/");

        let curl_command = [
            "curl",
            "-s",
            "-X",
            "POST",
            &url,
            "-H",
            "Content-Type: application/json",
            "-d",
            &payload_json,
        ]
        .iter()
        .map(|value| value.to_string())
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

        let mut full_output: Vec<u8> = Vec::new();
        if let bollard::exec::StartExecResults::Attached { mut output, .. } = output {
            while let Some(log_output) = output.next().await {
                if let Ok(msg) = log_output {
                    full_output.extend(msg.into_bytes());
                }
            }
        }

        let raw_output = String::from_utf8_lossy(&full_output).trim().to_string();
        Self::parse_invoke_body(&raw_output)
    }

    fn parse_invoke_body(raw: &str) -> Result<Value> {
        if raw.trim().is_empty() {
            return Ok(json!({ "raw": raw }));
        }

        match serde_json::from_str::<Value>(raw) {
            Ok(parsed) => Ok(parsed),
            Err(_) => Ok(json!({ "raw": raw })),
        }
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

    fn resource_name_from_image_name(image_name: &str) -> String {
        image_name.replace(':', "-")
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
            labels: Some(managed_container_labels()),
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

    pub async fn create_container_from_template(
        &self,
        container_config: &ContainerCreateBody,
        image_name: &str,
    ) -> Result<ContainerId> {
        let name = ContainerManager::container_name_from_image_name(image_name);
        let options = CreateContainerOptionsBuilder::new().name(&name).build();
        let response = self
            .docker
            .create_container(Some(options), container_config.clone())
            .await?;
        Ok(response.id)
    }

    async fn create_network_if_not_exists(&self, name: &str) -> Result<()> {
        let networks = self
            .docker
            .list_networks(None::<ListNetworksOptions>)
            .await?;
        if networks.iter().any(|n| n.name == Some(name.to_string())) {
            info!(
                "Docker network '{}' already exists. Skipping creation...",
                name
            );
            return Ok(());
        }
        let config = NetworkCreateRequest {
            name: name.to_string(),
            ..Default::default()
        };
        info!("Creating docker network: '{}'", name);
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
            info!("Shared volume '{volume_name}' already exists. Skipping creation...");
            return Ok(());
        }
        let config = VolumeCreateOptions {
            name: Some(volume_name.to_string()),
            ..Default::default()
        };
        info!("Creating volume: '{volume_name}'");
        self.docker.create_volume(config).await?;
        Ok(())
    }

    pub async fn setup_function_template(
        &self,
        image_name: &str,
        function_config: &FunctionConfig,
    ) -> Result<ContainerCreateBody> {
        let resource_name = Self::resource_name_from_image_name(image_name);
        self.create_network_if_not_exists(&resource_name).await?;
        self.create_shared_volume_if_not_exists(&resource_name)
            .await?;
        let mounts = vec![Mount {
            target: Some("/shared_data".to_string()),
            source: Some(resource_name.clone()),
            typ: Some(bollard::secret::MountTypeEnum::VOLUME),
            ..Default::default()
        }];
        let host_config = HostConfig {
            mounts: Some(mounts),
            network_mode: Some(resource_name),
            memory: Some(function_config.memory * MB_TO_BYTES),
            ..Default::default()
        };
        info!("Creating container template for '{image_name}'");
        Ok(ContainerCreateBody {
            image: Some(image_name.to_string()),
            labels: Some(managed_container_labels()),
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
        info!(
            "Building image '{image_name}' with dockerfile '{dockerfile_path}' from '{context_path}'"
        );
        let tar_path = self
            .create_build_context(context_path)
            .await
            .map_err(|e| anyhow!("Failed to create_build_context: {e}"))?;
        let build_image_options = BuildImageOptionsBuilder::new()
            .dockerfile(dockerfile_path)
            .t(image_name)
            .rm(true)
            .forcerm(true)
            .build();
        let bytes = Self::tar_to_bytes(&tar_path).await?;
        let mut image =
            self.docker
                .build_image(build_image_options, None, Some(body_full(bytes.into())));
        let mut errors = Vec::with_capacity(20);
        let mut daemon_error: Option<String> = None;
        while let Some(info) = image.next().await {
            match info {
                Ok(build_info) => {
                    if let Some(stream) = build_info.stream.as_deref() {
                        let line = stream.trim();
                        if !line.is_empty() {
                            info!("[docker build:{image_name}] {line}");
                        }
                    }
                    if let Some(status) = build_info.status.as_deref() {
                        let progress = build_info.progress.as_deref().unwrap_or("");
                        let id = build_info.id.as_deref().unwrap_or("");
                        let suffix = if id.is_empty() {
                            progress.to_string()
                        } else if progress.is_empty() {
                            format!(" {id}")
                        } else {
                            format!(" {id} {progress}")
                        };
                        let line = format!("{status}{suffix}").trim().to_string();
                        if !line.is_empty() {
                            info!("[docker build:{image_name}] {line}");
                        }
                    }
                    if let Some(err_text) = build_info.error.as_deref() {
                        let line = err_text.trim();
                        if !line.is_empty() {
                            error!("[docker build:{image_name}] {line}");
                            daemon_error = Some(line.to_string());
                        }
                    }
                }
                Err(e) => {
                    error!("[docker build:{image_name}] stream error: {e}");
                    errors.push(e);
                }
            }
        }
        if !errors.is_empty() {
            return Err(DeployError::DockerGeneral(errors).into());
        }
        if let Some(err) = daemon_error {
            bail!("Docker build failed for '{image_name}': {err}");
        }
        warn!("Docker build for '{image_name}' completed");
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
