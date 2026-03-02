use std::sync::Arc;
use tokio::signal;

use crate::AppState;

#[cfg(unix)]
async fn wait_for_shutdown_signal() {
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("Failed to create SIGTERM handler");

    tokio::select! {
        _ = signal::ctrl_c() => {
            println!("Received Ctrl+C signal");
        }
        _ = sigterm.recv() => {
            println!("Received SIGTERM signal");
        }
    }
}

#[cfg(windows)]
async fn wait_for_shutdown_signal() {
    let mut ctrl_break = tokio::signal::windows::ctrl_break()
        .expect("Failed to install Ctrl+Break handler");

    tokio::select! {
        _ = signal::ctrl_c() => {
            println!("Received Ctrl+C signal");
        }
        _ = ctrl_break.recv() => {
            println!("Received Ctrl+Break signal");
        }
    }
}

#[cfg(not(any(unix, windows)))]
async fn wait_for_shutdown_signal() {
    signal::ctrl_c()
        .await
        .expect("Failed to install Ctrl+C handler");
}

pub async fn shutdown_signal(state: Arc<AppState>) {
    wait_for_shutdown_signal().await;
    println!("\nServer closing...");
    state
        .function_manager
        .cleanup_containers(&state.redis_manager)
        .await;
}
