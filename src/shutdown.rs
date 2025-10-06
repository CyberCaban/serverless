use std::sync::Arc;
use tokio::signal::{
    self,
    unix::{SignalKind, signal},
};

use crate::AppState;

pub async fn shutdown_signal(state: Arc<AppState>) {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install ctrl+c handler");
    };
    #[cfg(unix)]
    let mut sigterm = signal(SignalKind::terminate()).expect("Failed to create sigterm handler");

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            println!("\nServer closing...");
            state.function_manager.cleanup_containers().await;
            dbg!(&state);
        },
        _ = sigterm.recv() => {
            println!("server shutdown");
        }
    }
}
