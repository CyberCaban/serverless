use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize)]
pub struct FunctionRequest {
    pub fn_name: String,
    pub payload: serde_json::Value,
    pub headers: HashMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FunctionResponse {
    pub status: u16,
    pub body: serde_json::Value,
    pub headers: HashMap<String, String>,
}
