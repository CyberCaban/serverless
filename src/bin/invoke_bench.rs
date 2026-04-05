use reqwest::Client;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, atomic::{AtomicU64, Ordering}};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

#[derive(Debug)]
struct BenchConfig {
    base_url: String,
    function_name: String,
    duration_secs: u64,
    concurrency: usize,
    request_timeout_ms: u64,
    output_path: String,
}

#[derive(Default, Debug)]
struct WorkerStats {
    latencies_ms: Vec<f64>,
    success_count: u64,
    error_count: u64,
    timeout_count: u64,
    container_hits: HashMap<String, u64>,
}

#[derive(Debug, Serialize)]
struct BenchReport {
    target_function: String,
    base_url: String,
    duration_secs: u64,
    concurrency: usize,
    request_timeout_ms: u64,
    total_requests: u64,
    successful_requests: u64,
    failed_requests: u64,
    timeout_requests: u64,
    error_rate_percent: f64,
    timeout_rate_percent: f64,
    throughput_rps: f64,
    latency_ms: LatencyMetrics,
    load_distribution: LoadDistribution,
    generated_at_unix_ms: u128,
}

#[derive(Debug, Serialize)]
struct LatencyMetrics {
    average: f64,
    p50: f64,
    p95: f64,
    p99: f64,
    max: f64,
    min: f64,
}

#[derive(Debug, Serialize)]
struct LoadDistribution {
    known_containers: usize,
    hits_per_container: HashMap<String, u64>,
    ideal_hits_per_container: f64,
    max_deviation_percent: f64,
    jain_fairness_index: f64,
}

fn parse_args() -> BenchConfig {
    let mut base_url = "http://localhost:5000".to_string();
    let mut function_name = "example-go".to_string();
    let mut duration_secs = 30_u64;
    let mut concurrency = 20_usize;
    let mut request_timeout_ms = 2_500_u64;
    let mut output_path = "bench_report.json".to_string();

    let args: Vec<String> = std::env::args().collect();
    let mut index = 1;
    while index + 1 < args.len() {
        match args[index].as_str() {
            "--base-url" => base_url = args[index + 1].clone(),
            "--function" => function_name = args[index + 1].clone(),
            "--duration-secs" => {
                duration_secs = args[index + 1].parse().unwrap_or(duration_secs);
            }
            "--concurrency" => {
                concurrency = args[index + 1].parse().unwrap_or(concurrency);
            }
            "--timeout-ms" => {
                request_timeout_ms = args[index + 1].parse().unwrap_or(request_timeout_ms);
            }
            "--output" => output_path = args[index + 1].clone(),
            _ => {}
        }
        index += 2;
    }

    BenchConfig {
        base_url,
        function_name,
        duration_secs,
        concurrency,
        request_timeout_ms,
        output_path,
    }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }

    let rank = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[rank]
}

fn default_payload(function_name: &str, worker_id: usize, seq: u64) -> Value {
    if function_name == "example-rust" {
        let base = 12_000.0 + (worker_id as f64 * 17.0);
        let spike = if seq % 12 == 0 { 6_500.0 } else { 0.0 };
        return serde_json::json!({
            "jobId": format!("bench-job-{worker_id}-{seq}"),
            "facilityId": "factory-north-1",
            "meterId": "line-a-main-meter",
            "windowStart": "2026-04-05T08:00:00Z",
            "pricePerKwhUsd": 0.17,
            "rounds": 250,
            "readings": [
                { "timestamp": "2026-04-05T08:00:00Z", "watts": base + 120.0 + spike },
                { "timestamp": "2026-04-05T08:01:00Z", "watts": base + 80.0 },
                { "timestamp": "2026-04-05T08:02:00Z", "watts": base + 300.0 },
                { "timestamp": "2026-04-05T08:03:00Z", "watts": base + 60.0 },
                { "timestamp": "2026-04-05T08:04:00Z", "watts": base + 150.0 },
                { "timestamp": "2026-04-05T08:05:00Z", "watts": base + 90.0 }
            ]
        });
    }

    if function_name == "example-go" {
        let shift = (seq % 13) as i32;
        return serde_json::json!({
            "numbers": [9 + shift, 4, 6, 32, 5, 7, 82, 3, worker_id as i32]
        });
    }

    serde_json::json!({
        "orderId": format!("bench-order-{worker_id}-{seq}"),
        "customer": { "name": "Benchmark Client", "email": "bench@example.com" },
        "shippingCountry": "US",
        "couponCode": if seq % 2 == 0 { serde_json::Value::String("WELCOME10".to_string()) } else { serde_json::Value::Null },
        "expedited": seq % 5 == 0,
        "items": [
            { "sku": "SKU-100", "name": "Widget", "quantity": 2, "price": 14.99 },
            { "sku": "SKU-200", "name": "Cable", "quantity": 1 + (seq % 3), "price": 7.50 }
        ]
    })
}

async fn run_worker(
    worker_id: usize,
    client: Client,
    cfg: Arc<BenchConfig>,
    deadline: Instant,
    sequence: Arc<AtomicU64>,
) -> WorkerStats {
    let mut stats = WorkerStats::default();
    let endpoint = format!("{}/invoke/{}", cfg.base_url, cfg.function_name);

    while Instant::now() < deadline {
        let seq = sequence.fetch_add(1, Ordering::Relaxed);
        let payload = default_payload(&cfg.function_name, worker_id, seq);

        let started = Instant::now();
        let response = client.post(&endpoint).json(&payload).send().await;
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;

        match response {
            Ok(resp) => {
                if !resp.status().is_success() {
                    stats.error_count += 1;
                    continue;
                }

                let body = resp.json::<Value>().await;
                match body {
                    Ok(value) => {
                        stats.success_count += 1;
                        stats.latencies_ms.push(elapsed_ms);

                        let container_id = value
                            .get("containerId")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned)
                            .unwrap_or_else(|| "unknown".to_string());
                        *stats.container_hits.entry(container_id).or_insert(0) += 1;
                    }
                    Err(_) => {
                        stats.error_count += 1;
                    }
                }
            }
            Err(err) => {
                if err.is_timeout() {
                    stats.timeout_count += 1;
                } else {
                    stats.error_count += 1;
                }
            }
        }
    }

    stats
}

fn merge_worker_stats(all: Vec<WorkerStats>) -> WorkerStats {
    let mut merged = WorkerStats::default();

    for mut s in all {
        merged.latencies_ms.append(&mut s.latencies_ms);
        merged.success_count += s.success_count;
        merged.error_count += s.error_count;
        merged.timeout_count += s.timeout_count;
        for (container, hits) in s.container_hits {
            *merged.container_hits.entry(container).or_insert(0) += hits;
        }
    }

    merged
}

fn build_load_distribution(container_hits: HashMap<String, u64>) -> LoadDistribution {
    let known: HashMap<String, u64> = container_hits
        .into_iter()
        .filter(|(container, _)| container != "unknown")
        .collect();

    if known.is_empty() {
        return LoadDistribution {
            known_containers: 0,
            hits_per_container: HashMap::new(),
            ideal_hits_per_container: 0.0,
            max_deviation_percent: 0.0,
            jain_fairness_index: 0.0,
        };
    }

    let total_hits: u64 = known.values().copied().sum();
    let n = known.len() as f64;
    let ideal = total_hits as f64 / n;

    let mut max_deviation_percent = 0.0;
    let mut sum = 0.0;
    let mut sum_sq = 0.0;

    for &hits in known.values() {
        let hits_f = hits as f64;
        sum += hits_f;
        sum_sq += hits_f * hits_f;
        let deviation = if ideal <= f64::EPSILON {
            0.0
        } else {
            ((hits_f - ideal).abs() / ideal) * 100.0
        };
        if deviation > max_deviation_percent {
            max_deviation_percent = deviation;
        }
    }

    let jain = if sum_sq <= f64::EPSILON {
        0.0
    } else {
        (sum * sum) / (n * sum_sq)
    };

    LoadDistribution {
        known_containers: known.len(),
        hits_per_container: known,
        ideal_hits_per_container: ideal,
        max_deviation_percent,
        jain_fairness_index: jain,
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = Arc::new(parse_args());

    let client = Client::builder()
        .timeout(Duration::from_millis(cfg.request_timeout_ms))
        .build()?;

    let deadline = Instant::now() + Duration::from_secs(cfg.duration_secs);
    let sequence = Arc::new(AtomicU64::new(0));

    let all_stats = Arc::new(Mutex::new(Vec::with_capacity(cfg.concurrency)));
    let mut handles = Vec::with_capacity(cfg.concurrency);

    let bench_started = Instant::now();
    for worker_id in 0..cfg.concurrency {
        let worker_client = client.clone();
        let worker_cfg = Arc::clone(&cfg);
        let worker_sequence = Arc::clone(&sequence);
        let worker_deadline = deadline;
        let worker_all_stats = Arc::clone(&all_stats);

        handles.push(tokio::spawn(async move {
            let stats =
                run_worker(worker_id, worker_client, worker_cfg, worker_deadline, worker_sequence)
                    .await;
            worker_all_stats.lock().await.push(stats);
        }));
    }

    for handle in handles {
        let _ = handle.await;
    }

    let elapsed_secs = bench_started.elapsed().as_secs_f64();
    let all = all_stats.lock().await.drain(..).collect::<Vec<_>>();
    let mut merged = merge_worker_stats(all);

    merged
        .latencies_ms
        .sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let total_requests = merged.success_count + merged.error_count + merged.timeout_count;
    let throughput_rps = if elapsed_secs <= f64::EPSILON {
        0.0
    } else {
        merged.success_count as f64 / elapsed_secs
    };

    let error_rate_percent = if total_requests == 0 {
        0.0
    } else {
        (merged.error_count as f64 / total_requests as f64) * 100.0
    };

    let timeout_rate_percent = if total_requests == 0 {
        0.0
    } else {
        (merged.timeout_count as f64 / total_requests as f64) * 100.0
    };

    let latency = if merged.latencies_ms.is_empty() {
        LatencyMetrics {
            average: 0.0,
            p50: 0.0,
            p95: 0.0,
            p99: 0.0,
            max: 0.0,
            min: 0.0,
        }
    } else {
        let sum: f64 = merged.latencies_ms.iter().sum();
        LatencyMetrics {
            average: sum / merged.latencies_ms.len() as f64,
            p50: percentile(&merged.latencies_ms, 50.0),
            p95: percentile(&merged.latencies_ms, 95.0),
            p99: percentile(&merged.latencies_ms, 99.0),
            min: *merged.latencies_ms.first().unwrap_or(&0.0),
            max: *merged.latencies_ms.last().unwrap_or(&0.0),
        }
    };

    let load_distribution = build_load_distribution(merged.container_hits);

    let generated_at_unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();

    let report = BenchReport {
        target_function: cfg.function_name.clone(),
        base_url: cfg.base_url.clone(),
        duration_secs: cfg.duration_secs,
        concurrency: cfg.concurrency,
        request_timeout_ms: cfg.request_timeout_ms,
        total_requests,
        successful_requests: merged.success_count,
        failed_requests: merged.error_count,
        timeout_requests: merged.timeout_count,
        error_rate_percent,
        timeout_rate_percent,
        throughput_rps,
        latency_ms: latency,
        load_distribution,
        generated_at_unix_ms,
    };

    let output_json = serde_json::to_string_pretty(&report)?;
    tokio::fs::write(&cfg.output_path, output_json.as_bytes()).await?;

    println!("Benchmark report saved to {}", cfg.output_path);
    println!(
        "ok={} err={} timeout={} p95={:.2}ms p99={:.2}ms throughput={:.2} rps",
        report.successful_requests,
        report.failed_requests,
        report.timeout_requests,
        report.latency_ms.p95,
        report.latency_ms.p99,
        report.throughput_rps
    );

    Ok(())
}
