use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::post,
};
use serde::{Deserialize, Serialize};
use std::{
    net::SocketAddr,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Default)]
struct AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnalyzeRequest {
    job_id: String,
    facility_id: String,
    meter_id: String,
    window_start: String,
    readings: Vec<PowerReading>,
    price_per_kwh_usd: Option<f64>,
    rounds: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PowerReading {
    timestamp: String,
    watts: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadingAnomaly {
    index: usize,
    timestamp: String,
    watts: f64,
    deviation_percent: f64,
    risk_score: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AnalyzeResponse {
    job_id: String,
    facility_id: String,
    meter_id: String,
    window_start: String,
    window_end: String,
    input_points: usize,
    rounds: u32,
    baseline_watts: f64,
    peak_watts: f64,
    min_watts: f64,
    total_kwh: f64,
    estimated_cost_usd: f64,
    average_risk_score: f64,
    anomaly_count: usize,
    top_anomalies: Vec<ReadingAnomaly>,
    processed_at_unix_ms: u128,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

fn intensive_transform(mut value: u64, rounds: u32) -> u64 {
    for round in 0..rounds {
        value ^= value >> 12;
        value ^= value << 25;
        value ^= value >> 27;
        value = value.wrapping_mul(0x2545_f491_4f6c_dd1d);
        value = value.rotate_left((round % 63) + 1);
        value = value.wrapping_add(0x9E37_79B9_7F4A_7C15 ^ u64::from(round));
    }

    value
}

fn process_readings(
    readings: &[PowerReading],
    rounds: u32,
) -> (f64, f64, f64, f64, f64, usize, Vec<ReadingAnomaly>) {
    let mut transformed_values = Vec::with_capacity(readings.len());
    let mut total_watts = 0.0;
    let mut min_watts = f64::MAX;
    let mut peak_watts = f64::MIN;
    let mut anomaly_count = 0usize;
    let mut risk_total = 0.0;

    for (index, reading) in readings.iter().enumerate() {
        min_watts = min_watts.min(reading.watts);
        peak_watts = peak_watts.max(reading.watts);
        total_watts += reading.watts;

        let seed = reading.watts.to_bits() ^ ((index as u64 + 1) * 0x9E37_79B9_7F4A_7C15);
        let transformed = intensive_transform(seed, rounds);
        let normalized = transformed as f64 / u64::MAX as f64;
        risk_total += normalized;

        transformed_values.push((index, transformed));

        if normalized > 0.82 {
            anomaly_count += 1;
        }
    }

    let baseline_watts = total_watts / readings.len() as f64;
    let total_kwh = (total_watts / 1000.0) / 60.0;
    let average_risk_score = risk_total / readings.len() as f64;

    let mut anomalies = Vec::with_capacity(readings.len());
    for (index, transformed) in transformed_values {
        let reading = &readings[index];
        let deviation_percent = if baseline_watts.abs() < f64::EPSILON {
            0.0
        } else {
            ((reading.watts - baseline_watts).abs() / baseline_watts) * 100.0
        };

        let risk_score = transformed as f64 / u64::MAX as f64;
        anomalies.push(ReadingAnomaly {
            index,
            timestamp: reading.timestamp.clone(),
            watts: reading.watts,
            deviation_percent,
            risk_score,
        });
    }

    anomalies.sort_by(|a, b| b.risk_score.total_cmp(&a.risk_score));
    let top_anomalies = anomalies.into_iter().take(3).collect();

    (
        baseline_watts,
        peak_watts,
        min_watts,
        total_kwh,
        average_risk_score,
        anomaly_count,
        top_anomalies,
    )
}

async fn analyze_batch(
    State(_state): State<AppState>,
    Json(request): Json<AnalyzeRequest>,
) -> Result<Json<AnalyzeResponse>, (StatusCode, Json<ErrorResponse>)> {
    if request.job_id.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "jobId is required".to_string(),
            }),
        ));
    }

    if request.facility_id.trim().is_empty() || request.meter_id.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "facilityId and meterId are required".to_string(),
            }),
        ));
    }

    if request.readings.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "readings must contain at least one value".to_string(),
            }),
        ));
    }

    if request.readings.iter().any(|reading| reading.watts < 0.0) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "reading watts must be >= 0".to_string(),
            }),
        ));
    }

    let rounds = request.rounds.unwrap_or(1200);
    if rounds == 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "rounds must be greater than zero".to_string(),
            }),
        ));
    }

    let (
        baseline_watts,
        peak_watts,
        min_watts,
        total_kwh,
        average_risk_score,
        anomaly_count,
        top_anomalies,
    ) = process_readings(&request.readings, rounds);

    let price_per_kwh_usd = request.price_per_kwh_usd.unwrap_or(0.12);
    let estimated_cost_usd = total_kwh * price_per_kwh_usd;

    let window_end = request
        .readings
        .last()
        .map(|reading| reading.timestamp.clone())
        .unwrap_or_else(|| request.window_start.clone());

    let processed_at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();

    Ok(Json(AnalyzeResponse {
        job_id: request.job_id,
        facility_id: request.facility_id,
        meter_id: request.meter_id,
        window_start: request.window_start,
        window_end,
        input_points: request.readings.len(),
        rounds,
        baseline_watts,
        peak_watts,
        min_watts,
        total_kwh,
        estimated_cost_usd,
        average_risk_score,
        anomaly_count,
        top_anomalies,
        processed_at_unix_ms,
    }))
}

#[tokio::main]
async fn main() {
    let app = Router::new().route("/", post(analyze_batch)).with_state(AppState);
    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));

    println!("Rust batch analytics server running on {addr}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind server socket");

    axum::serve(listener, app)
        .await
        .expect("server should run until shutdown");
}
