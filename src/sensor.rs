// src/sensor.rs

use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::{Duration, Instant};
use chrono::Utc;
use rand::Rng;
use crossbeam_channel::Sender;
use crate::sensor_data::SensorData;
use std::fs::File;
use std::io::Write;
use std::sync::Mutex;

/// Simulates a sensor running in a background thread
pub fn simulate_sensor(
    name: &'static str,
    period_ms: u64,
    tx: Sender<SensorData>,
    log_file: Arc<Mutex<File>>,
    running: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut rng = rand::rng();
        let period = Duration::from_millis(period_ms);
        let mut next_sample = Instant::now() + period;
        let mut last_sample_time = Instant::now();

        while running.load(Ordering::Relaxed) {
            let now = Instant::now();
            let value = match name {
                "thermal" => rng.random_range(20.0..100.0) as f32,
                "gyro" => rng.random_range(0.0..1.0) as f32,
                "battery" => rng.random_range(0.0..100.0) as f32,
                _ => 0.0,
            };

            let sample = SensorData {
                name,
                value,
                timestamp: Utc::now(),
                priority: 0,
            };

            // Calculate jitter as deviation from period
            let jitter_ms = (now.duration_since(last_sample_time).as_secs_f64() * 1000.0 - period_ms as f64).abs();
            last_sample_time = now;

            let msg = format!(
                "[{}] Sensor {} jitter (MAD): {:.3}ms",
                Utc::now(),
                name,
                jitter_ms
            );
            if let Ok(mut f) = log_file.lock() { let _ = writeln!(f, "{}", msg); }

            if tx.send(sample).is_err() { break; }

            if next_sample > Instant::now() {
                std::thread::sleep(next_sample - Instant::now());
            }
            next_sample += period;
        }
    })
}

