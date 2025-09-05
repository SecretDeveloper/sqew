use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct Queue {
    pub id: i64,
    pub name: String,
    pub max_attempts: i32,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct Message {
    pub id: i64,
    pub queue_id: i64,
    pub payload: String,
    pub attempts: i32,
    pub available_at: i64,
    pub created_at: i64,
}
