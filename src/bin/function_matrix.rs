use anyhow::{bail, Result};
use clap::Parser;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{path::PathBuf, time::Duration};
use tokio::{fs, process::Command, time::sleep};

#[derive(Clone, Debug)]
struct MatrixConfig {
    replicas: u16,
    load_balancer: &'static str,
    replica_weights: Vec<usize>,
}

#[derive(Debug)]
struct CliConfig {
    base_url: String,
    function_name: String,
    output_dir: PathBuf,
    replica_sets: Vec<u16>,
    bench_duration_secs: u64,
    bench_concurrency: usize,
    bench_timeout_ms: u64,
    bench_pattern: String,
    bench_all_patterns: bool,
    bench_target_rps: f64,
    bench_low_rps: f64,
    bench_high_rps: f64,
    bench_pulse_period_secs: u64,
}

#[derive(Debug, Parser)]
#[command(name = "function_matrix")]
#[command(about = "Run matrix benchmark scenarios against deployed serverless function")]
struct CliArgs {
    #[arg(long, default_value = "http://localhost:5000")]
    base_url: String,

    #[arg(long = "function", default_value = "example-go")]
    function_name: String,

    #[arg(long, default_value = "matrix_results")]
    output_dir: PathBuf,

    #[arg(long, default_value = "1,2,4,8,16,32")]
    replicas: String,

    #[arg(long, default_value_t = 30)]
    bench_duration_secs: u64,

    #[arg(long, default_value_t = 20)]
    bench_concurrency: usize,

    #[arg(long, default_value_t = 2500)]
    bench_timeout_ms: u64,

    #[arg(long, default_value = "stress")]
    bench_pattern: String,

    #[arg(long, default_value_t = false)]
    all_bench_patterns: bool,

    #[arg(long, default_value_t = 100.0)]
    target_rps: f64,

    #[arg(long, default_value_t = 50.0)]
    low_rps: f64,

    #[arg(long, default_value_t = 200.0)]
    high_rps: f64,

    #[arg(long, default_value_t = 5)]
    pulse_period_secs: u64,

    #[arg(long, default_value_t = false)]
    all_three_cases: bool,
}

#[derive(Debug, Deserialize)]
struct OperationStatus {
    kind: Option<String>,
    state: String,
    #[serde(rename = "functionName")]
    function_name: Option<String>,
    #[serde(rename = "functionDeployed")]
    function_deployed: Option<bool>,
    accepted: Option<bool>,
    error: Option<String>,
    logs: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct OperationResponse {
    id: String,
}

#[derive(Debug, Serialize)]
struct MatrixSummary {
    target_function: String,
    base_url: String,
    replica_sets: Vec<u16>,
    bench_duration_secs: u64,
    bench_concurrency: usize,
    bench_timeout_ms: u64,
    bench_patterns: Vec<String>,
    bench_target_rps: f64,
    bench_low_rps: f64,
    bench_high_rps: f64,
    bench_pulse_period_secs: u64,
    generated_at_unix_ms: u128,
    runs: Vec<MatrixRunResult>,
}

#[derive(Debug, Serialize)]
struct MatrixRunResult {
    replicas: u16,
    load_balancer: String,
    bench_pattern: String,
    expected_rps: Option<f64>,
    operation_id: String,
    update_status: OperationStatusReport,
    benchmark_report_path: String,
    benchmark_report: Value,
}

#[derive(Debug, Serialize)]
struct OperationStatusReport {
    kind: Option<String>,
    state: String,
    function_name: Option<String>,
    function_deployed: Option<bool>,
    accepted: Option<bool>,
    error: Option<String>,
}

fn parse_replica_sets(input: &str) -> Vec<u16> {
    let parsed: Vec<u16> = input
        .split(',')
        .filter_map(|value| value.trim().parse::<u16>().ok())
        .filter(|value| *value > 0)
        .collect();

    if parsed.is_empty() {
        vec![1, 2, 4, 8, 16, 32]
    } else {
        parsed
    }
}

fn parse_args() -> CliConfig {
    let args = CliArgs::parse();

    let mut bench_pattern = args.bench_pattern.to_ascii_lowercase();
    let mut bench_all_patterns = args.all_bench_patterns;
    if bench_pattern == "all" {
        bench_all_patterns = true;
    }

    let mut bench_target_rps = args.target_rps;
    let mut bench_low_rps = args.low_rps;
    let mut bench_high_rps = args.high_rps;
    let mut bench_pulse_period_secs = args.pulse_period_secs;

    if args.all_three_cases {
        bench_all_patterns = true;
        bench_pattern = "all".to_string();
        bench_target_rps = 300_000.0;
        bench_low_rps = 100_000.0;
        bench_high_rps = 300_000.0;
        bench_pulse_period_secs = 5;
    }

    CliConfig {
        base_url: args.base_url,
        function_name: args.function_name,
        output_dir: args.output_dir,
        replica_sets: parse_replica_sets(&args.replicas),
        bench_duration_secs: args.bench_duration_secs,
        bench_concurrency: args.bench_concurrency,
        bench_timeout_ms: args.bench_timeout_ms,
        bench_pattern,
        bench_all_patterns,
        bench_target_rps,
        bench_low_rps,
        bench_high_rps,
        bench_pulse_period_secs,
    }
}

fn selected_bench_patterns(cfg: &CliConfig) -> Vec<String> {
    if cfg.bench_all_patterns {
        return vec!["stable".to_string(), "pulse".to_string(), "stress".to_string()];
    }

    vec![cfg.bench_pattern.to_ascii_lowercase()]
}

fn expected_rps_for_pattern(cfg: &CliConfig, bench_pattern: &str) -> Option<f64> {
    match bench_pattern {
        "stable" => Some(cfg.bench_target_rps),
        "pulse" => Some((cfg.bench_high_rps + cfg.bench_low_rps) / 2.0),
        "stress" => None,
        _ => None,
    }
}

fn matrix_for(replicas: u16) -> Vec<MatrixConfig> {
    if replicas == 1 {
        return vec![MatrixConfig {
            replicas,
            load_balancer: "round_robin",
            replica_weights: vec![1],
        }];
    }

    vec![
        MatrixConfig {
            replicas,
            load_balancer: "round_robin",
            replica_weights: vec![1; replicas as usize],
        },
        MatrixConfig {
            replicas,
            load_balancer: "least_loaded",
            replica_weights: vec![1; replicas as usize],
        },
        MatrixConfig {
            replicas,
            load_balancer: "random",
            replica_weights: vec![1; replicas as usize],
        },
        MatrixConfig {
            replicas,
            load_balancer: "weighted_priority",
            replica_weights: (1..=replicas as usize).rev().collect(),
        },
    ]
}

fn current_unix_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

async fn post_json<T: serde::de::DeserializeOwned>(client: &Client, url: &str, body: Value) -> Result<T> {
    let response = client.post(url).json(&body).send().await?.error_for_status()?;
    Ok(response.json::<T>().await?)
}

async fn patch_json<T: serde::de::DeserializeOwned>(client: &Client, url: &str, body: Value) -> Result<T> {
    let response = client.patch(url).json(&body).send().await?.error_for_status()?;
    Ok(response.json::<T>().await?)
}

async fn wait_for_operation(client: &Client, base_url: &str, operation_id: &str) -> Result<OperationStatus> {
    let status_url = format!("{base_url}/deploy/status/{operation_id}");

    for _ in 0..180 {
        let response = client.get(&status_url).send().await?;
        if response.status().is_success() {
            let status = response.json::<OperationStatus>().await?;
            match status.state.as_str() {
                "running" => sleep(Duration::from_secs(1)).await,
                "finished" => return Ok(status),
                "failed" => {
                    let error = status.error.clone().unwrap_or_else(|| "unknown error".to_string());
                    let logs = status.logs.clone().unwrap_or_default();
                    bail!("operation {operation_id} failed: {error}\nlogs: {logs:?}");
                }
                other => bail!("unexpected operation state '{other}' for {operation_id}"),
            }
        } else {
            sleep(Duration::from_secs(1)).await;
        }
    }

    bail!("timed out waiting for operation {operation_id}")
}

async fn ensure_deployed(client: &Client, cfg: &CliConfig) -> Result<OperationStatus> {
    let deploy_url = format!("{}/deploy/{}", cfg.base_url, cfg.function_name);
    let response: OperationResponse = post_json(client, &deploy_url, Value::Null).await?;
    wait_for_operation(client, &cfg.base_url, &response.id).await
}

async fn apply_config(
    client: &Client,
    cfg: &CliConfig,
    matrix: MatrixConfig,
) -> Result<(String, OperationStatus)> {
    let update_url = format!("{}/functions/{}", cfg.base_url, cfg.function_name);
    let body = serde_json::json!({
        "replicas": matrix.replicas,
        "loadBalancer": matrix.load_balancer,
        "replicaWeights": matrix.replica_weights,
    });

    let response: OperationResponse = patch_json(client, &update_url, body).await?;
    let status = wait_for_operation(client, &cfg.base_url, &response.id).await?;
    Ok((response.id, status))
}

async fn run_benchmark(
    cfg: &CliConfig,
    matrix: MatrixConfig,
    bench_pattern: &str,
    output_path: &std::path::Path,
) -> Result<Value> {
    let output_path_string = output_path.to_string_lossy().to_string();
    let bench_duration_secs = cfg.bench_duration_secs.to_string();
    let bench_concurrency = cfg.bench_concurrency.to_string();
    let bench_timeout_ms = cfg.bench_timeout_ms.to_string();
    let bench_target_rps = cfg.bench_target_rps.to_string();
    let bench_low_rps = cfg.bench_low_rps.to_string();
    let bench_high_rps = cfg.bench_high_rps.to_string();
    let bench_pulse_period_secs = cfg.bench_pulse_period_secs.to_string();

    let mut command = Command::new("cargo");
    command
        .args([
            "run",
            "--release",
            "--bin",
            "invoke_bench",
            "--",
            "--base-url",
            &cfg.base_url,
            "--function",
            &cfg.function_name,
            "--pattern",
            bench_pattern,
            "--duration-secs",
            &bench_duration_secs,
            "--concurrency",
            &bench_concurrency,
            "--timeout-ms",
            &bench_timeout_ms,
            "--target-rps",
            &bench_target_rps,
            "--low-rps",
            &bench_low_rps,
            "--high-rps",
            &bench_high_rps,
            "--pulse-period-secs",
            &bench_pulse_period_secs,
            "--output",
            &output_path_string,
        ])
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());

    let status = command.status().await?;
    if !status.success() {
        bail!("benchmark failed for replicas={} balancer={}", matrix.replicas, matrix.load_balancer);
    }

    let report_text = fs::read_to_string(output_path).await?;
    Ok(serde_json::from_str::<Value>(&report_text)?)
}

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = parse_args();
    fs::create_dir_all(&cfg.output_dir).await?;
    let bench_patterns = selected_bench_patterns(&cfg);

    let client = Client::builder()
        .timeout(Duration::from_secs(20))
        .build()?;

    let initial_deploy = ensure_deployed(&client, &cfg).await?;
    if !initial_deploy.accepted.unwrap_or(false) {
        bail!("initial deploy was not accepted");
    }

    let mut runs = Vec::new();

    for &replicas in &cfg.replica_sets {
        for matrix in matrix_for(replicas) {
            for bench_pattern in &bench_patterns {
                let (operation_id, update_status) = apply_config(&client, &cfg, matrix.clone()).await?;
                if !update_status.accepted.unwrap_or(false) {
                    bail!(
                        "config update for replicas={} balancer={} was not accepted",
                        matrix.replicas,
                        matrix.load_balancer
                    );
                }
                if update_status.function_deployed != Some(true) {
                    bail!(
                        "config update for replicas={} balancer={} finished but function is not deployed",
                        matrix.replicas,
                        matrix.load_balancer
                    );
                }

                let case_dir = cfg
                    .output_dir
                    .join(&cfg.function_name)
                    .join(format!("{}-{}", matrix.replicas, matrix.load_balancer));
                fs::create_dir_all(&case_dir).await?;
                let bench_output_path = case_dir.join(format!("bench-{}.json", bench_pattern));
                let bench_report = run_benchmark(&cfg, matrix.clone(), bench_pattern, &bench_output_path).await?;

                runs.push(MatrixRunResult {
                    replicas: matrix.replicas,
                    load_balancer: matrix.load_balancer.to_string(),
                    bench_pattern: bench_pattern.clone(),
                    expected_rps: expected_rps_for_pattern(&cfg, bench_pattern),
                    operation_id,
                    update_status: OperationStatusReport {
                        kind: update_status.kind.clone(),
                        state: update_status.state.clone(),
                        function_name: update_status.function_name.clone(),
                        function_deployed: update_status.function_deployed,
                        accepted: update_status.accepted,
                        error: update_status.error.clone(),
                    },
                    benchmark_report_path: bench_output_path.to_string_lossy().to_string(),
                    benchmark_report: bench_report,
                });
            }
        }
    }

    let summary = MatrixSummary {
        target_function: cfg.function_name.clone(),
        base_url: cfg.base_url.clone(),
        replica_sets: cfg.replica_sets.clone(),
        bench_duration_secs: cfg.bench_duration_secs,
        bench_concurrency: cfg.bench_concurrency,
        bench_timeout_ms: cfg.bench_timeout_ms,
        bench_patterns,
        bench_target_rps: cfg.bench_target_rps,
        bench_low_rps: cfg.bench_low_rps,
        bench_high_rps: cfg.bench_high_rps,
        bench_pulse_period_secs: cfg.bench_pulse_period_secs,
        generated_at_unix_ms: current_unix_ms(),
        runs,
    };

    let summary_path = cfg.output_dir.join(format!("{}-matrix-summary.json", cfg.function_name));
    fs::write(&summary_path, serde_json::to_string_pretty(&summary)?).await?;
    println!("Matrix summary saved to {}", summary_path.to_string_lossy());
    Ok(())
}
