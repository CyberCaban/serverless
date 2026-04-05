use axum::response::Json;
use serde_json::Value;

pub mod deploy;
pub mod get_status;
pub mod invoke;
pub mod list_functions;
pub mod replicas;
pub mod stop;

pub type EndpointResult = std::result::Result<Json<Value>, crate::errors::ApiErrorResponse>;