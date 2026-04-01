use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

use crate::errors::function_error::FunctionError;

pub mod deploy_error;
pub mod function_error;

#[derive(Debug, Serialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    pub details: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ApiErrorResponse {
    #[serde(skip_serializing)]
    pub status: StatusCode,
    pub error: ApiError,
}

impl ApiErrorResponse {
    pub fn new(
        status: StatusCode,
        code: impl Into<String>,
        message: impl Into<String>,
        details: Vec<String>,
    ) -> Self {
        Self {
            status,
            error: ApiError {
                code: code.into(),
                message: message.into(),
                details,
            },
        }
    }
}

impl IntoResponse for ApiErrorResponse {
    fn into_response(self) -> Response {
        (self.status, Json(self)).into_response()
    }
}

pub fn serialize_err(e: anyhow::Error) -> ApiErrorResponse {
    let (status, code) = error_meta(&e);
    ApiErrorResponse::new(
        status,
        code,
        e.to_string(),
        e.chain().skip(1).map(|cause| cause.to_string()).collect(),
    )
}

fn error_meta(error: &anyhow::Error) -> (StatusCode, &'static str) {
    if let Some(function_error) = error.downcast_ref::<FunctionError>() {
        return match function_error {
            FunctionError::FunctionNotDeployed => (StatusCode::NOT_FOUND, "FUNCTION_NOT_DEPLOYED"),
            FunctionError::NoRunningContainers => {
                (StatusCode::SERVICE_UNAVAILABLE, "NO_RUNNING_CONTAINERS")
            }
        };
    }

    if error.downcast_ref::<redis::RedisError>().is_some() {
        return (StatusCode::INTERNAL_SERVER_ERROR, "REDIS_ERROR");
    }

    if let Some(io_error) = error.downcast_ref::<std::io::Error>() {
        return match io_error.kind() {
            std::io::ErrorKind::NotFound => (StatusCode::NOT_FOUND, "NOT_FOUND"),
            std::io::ErrorKind::InvalidInput => (StatusCode::BAD_REQUEST, "BAD_REQUEST"),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, "IO_ERROR"),
        };
    }

    if error.downcast_ref::<serde_json::Error>().is_some() {
        return (StatusCode::BAD_REQUEST, "BAD_REQUEST");
    }

    let message = error.to_string();
    if message.contains("не найден") || message.contains("не развернута") {
        return (StatusCode::NOT_FOUND, "NOT_FOUND");
    }

    (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR")
}
