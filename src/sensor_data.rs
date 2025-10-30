// src/sensor_data.rs

use chrono::{DateTime, Utc};
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensorData {
    pub name: &'static str,
    pub value: f32,
    pub timestamp: DateTime<Utc>,
    pub priority: u8,
}
