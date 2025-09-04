use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Queue {
    pub id: i64,
    pub name: String,
    pub dlq_id: Option<i64>,
    pub max_attempts: i32,
    pub visibility_ms: i32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Message {
    pub id: i64,
    pub queue_id: i64,
    pub payload_json: String,
    pub priority: i32,
    pub idempotency_key: Option<String>,
    pub attempts: i32,
    pub available_at: i64,
    pub lease_expires_at: Option<i64>,
    pub leased_by: Option<String>,
    pub created_at: i64,
    pub expires_at: Option<i64>,
}
