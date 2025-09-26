use axum::{Router, extract::Path, response::Json, routing::post};
use serde_json::Value;

mod container_manager;
mod models;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = Router::new()
        .route("/deploy/{function_name}", post(deploy_function))
        .route("/invoke/{function_name}", post(invoke_function));
    let listener = tokio::net::TcpListener::bind("0.0.0.0:5000").await?;
    println!("Running on port 5000");

    axum::serve(listener, app).await?;
    Ok(())
}

async fn deploy_function(Path(function_name): Path<String>) -> Json<Value> {
    Json(serde_json::json!({
        "status": format!("Deploying {}...", function_name)
    }))
}

async fn invoke_function(function_name: String) -> Json<Value> {
    Json(serde_json::json!({
        "status": format!("Invoking {}...", function_name)
    }))
}
