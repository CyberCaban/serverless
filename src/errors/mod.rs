use axum::Json;
use serde::Serialize;
use serde_json::Value;

pub mod deploy_error;
pub mod function_error;

pub struct SerializableError(anyhow::Error);
impl Serialize for SerializableError {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}
impl From<anyhow::Error> for SerializableError {
    fn from(value: anyhow::Error) -> Self {
        SerializableError(value)
    }
}

impl From<redis::RedisError> for SerializableError {
    fn from(value: redis::RedisError) -> Self {
        SerializableError(anyhow::Error::from(value))
    }
}

pub fn serialize_err(e: anyhow::Error) -> Json<Value> {
    let error: SerializableError = e.into();
    Json(serde_json::json!({
        "error": error
    }))
}
