use reqwest::Client;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, atomic::{AtomicU64, Ordering}};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::time::{Instant as TokioInstant, sleep_until};

#[derive(Clone, Copy, Debug)]
enum LoadPattern {
    Stable,
    Pulse,
    Stress,
}

#[derive(Debug)]
struct BenchConfig {
    base_url: String,
    function_name: String,
    duration_secs: u64,
    concurrency: usize,
    request_timeout_ms: u64,
    output_path: String,
    load_pattern: LoadPattern,
    target_rps: f64,
    low_rps: f64,
    high_rps: f64,
    pulse_period_secs: u64,
}

#[derive(Debug)]
struct BenchJob {
    worker_id: usize,
    seq: u64,
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
    load_pattern: LoadPatternReport,
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
struct LoadPatternReport {
    kind: String,
    target_rps: Option<f64>,
    low_rps: Option<f64>,
    high_rps: Option<f64>,
    pulse_period_secs: Option<u64>,
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
    let mut load_pattern = LoadPattern::Stress;
    let mut target_rps = 100.0_f64;
    let mut low_rps = 50.0_f64;
    let mut high_rps = 200.0_f64;
    let mut pulse_period_secs = 5_u64;

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
            "--pattern" => {
                let value = args[index + 1].to_ascii_lowercase();
                load_pattern = match value.as_str() {
                    "stable" => LoadPattern::Stable,
                    "pulse" | "pulsating" => LoadPattern::Pulse,
                    "stress" => LoadPattern::Stress,
                    _ => LoadPattern::Stress,
                };
            }
            "--target-rps" => {
                target_rps = args[index + 1].parse().unwrap_or(target_rps);
            }
            "--low-rps" => {
                low_rps = args[index + 1].parse().unwrap_or(low_rps);
            }
            "--high-rps" => {
                high_rps = args[index + 1].parse().unwrap_or(high_rps);
            }
            "--pulse-period-secs" => {
                pulse_period_secs = args[index + 1].parse().unwrap_or(pulse_period_secs);
            }
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
        load_pattern,
        target_rps,
        low_rps,
        high_rps,
        pulse_period_secs,
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
    mut receiver: mpsc::Receiver<BenchJob>,
    client: Client,
    cfg: Arc<BenchConfig>,
) -> WorkerStats {
    let mut stats = WorkerStats::default();
    let endpoint = format!("{}/invoke/{}", cfg.base_url, cfg.function_name);

    while let Some(job) = receiver.recv().await {
        let payload = default_payload(&cfg.function_name, job.worker_id, job.seq);

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

fn current_pulse_rps(cfg: &BenchConfig, elapsed: Duration) -> f64 {
    let period = Duration::from_secs(cfg.pulse_period_secs.max(1));
    let phase = (elapsed.as_secs_f64() / period.as_secs_f64()).floor() as u64;
    if phase.is_multiple_of(2) {
        cfg.high_rps.max(0.1)
    } else {
        cfg.low_rps.max(0.1)
    }
}

async fn run_producer(
    cfg: Arc<BenchConfig>,
    senders: Vec<mpsc::Sender<BenchJob>>,
    deadline: TokioInstant,
    sequence: Arc<AtomicU64>,
) {
    let start = TokioInstant::now();
    let mut last_tick = TokioInstant::now();
    let mut token_budget = 0.0_f64;

    loop {
        let now = TokioInstant::now();
        if now >= deadline {
            break;
        }

        match cfg.load_pattern {
            LoadPattern::Stress => {
                let seq = sequence.fetch_add(1, Ordering::Relaxed);
                let worker_id = (seq as usize) % cfg.concurrency;
                let job = BenchJob { worker_id, seq };
                if senders[worker_id].send(job).await.is_err() {
                    break;
                }
                continue;
            }
            LoadPattern::Stable | LoadPattern::Pulse => {
                let elapsed_since_tick = now.duration_since(last_tick).as_secs_f64();
                last_tick = now;

                let rate_rps = match cfg.load_pattern {
                    LoadPattern::Stable => cfg.target_rps.max(0.1),
                    LoadPattern::Pulse => current_pulse_rps(&cfg, now.duration_since(start)),
                    LoadPattern::Stress => unreachable!(),
                };

                token_budget += rate_rps * elapsed_since_tick;
                let mut to_dispatch = token_budget.floor() as usize;

                if to_dispatch == 0 {
                    // Tokio timers are not microsecond-precise on all platforms;
                    // produce in short ticks and accumulate tokens instead.
                    sleep_until((now + Duration::from_millis(1)).min(deadline)).await;
                    continue;
                }

                // Prevent unbounded bursts if loop was paused for too long.
                to_dispatch = to_dispatch.min(20_000);

                let mut dispatched = 0_usize;
                for _ in 0..to_dispatch {
                    let seq = sequence.fetch_add(1, Ordering::Relaxed);
                    let worker_id = (seq as usize) % cfg.concurrency;
                    let job = BenchJob { worker_id, seq };
                    if senders[worker_id].send(job).await.is_err() {
                        return;
                    }
                    dispatched += 1;
                }

                token_budget -= dispatched as f64;
            }
        }
    }
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

    let deadline = TokioInstant::now() + Duration::from_secs(cfg.duration_secs);
    let sequence = Arc::new(AtomicU64::new(0));

    let mut handles = Vec::with_capacity(cfg.concurrency);
    let mut senders = Vec::with_capacity(cfg.concurrency);

    for worker_id in 0..cfg.concurrency {
        let (sender, receiver) = mpsc::channel::<BenchJob>(1024);
        senders.push(sender);

        let worker_client = client.clone();
        let worker_cfg = Arc::clone(&cfg);

        handles.push(tokio::spawn(async move {
            let _ = worker_id;
            run_worker(receiver, worker_client, worker_cfg).await
        }));
    }

    let producer_cfg = Arc::clone(&cfg);
    let producer_sequence = Arc::clone(&sequence);
    let producer_senders = senders;
    let producer_handle = tokio::spawn(async move {
        run_producer(producer_cfg, producer_senders, deadline, producer_sequence).await;
    });

    let bench_started = Instant::now();

    let _ = producer_handle.await;

    let mut collected_stats = Vec::with_capacity(cfg.concurrency);
    for handle in handles {
        if let Ok(stats) = handle.await {
            collected_stats.push(stats);
        }
    }

    let elapsed_secs = bench_started.elapsed().as_secs_f64();
    let mut merged = merge_worker_stats(collected_stats);

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
        load_pattern: LoadPatternReport {
            kind: match cfg.load_pattern {
                LoadPattern::Stable => "stable".to_string(),
                LoadPattern::Pulse => "pulse".to_string(),
                LoadPattern::Stress => "stress".to_string(),
            },
            target_rps: matches!(cfg.load_pattern, LoadPattern::Stable).then_some(cfg.target_rps),
            low_rps: matches!(cfg.load_pattern, LoadPattern::Pulse).then_some(cfg.low_rps),
            high_rps: matches!(cfg.load_pattern, LoadPattern::Pulse).then_some(cfg.high_rps),
            pulse_period_secs: matches!(cfg.load_pattern, LoadPattern::Pulse)
                .then_some(cfg.pulse_period_secs),
        },
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
