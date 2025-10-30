// src/downlink.rs

use chrono::Utc;
use lapin::{
    options::{BasicPublishOptions, ExchangeDeclareOptions, QueueDeclareOptions, QueueBindOptions},
    types::FieldTable, BasicProperties, Channel, Connection, ConnectionProperties, ExchangeKind,
};
use serde::Serialize;
use tokio::time::{interval, Duration as TokioDuration};
use std::time::Instant;
use tokio::sync::mpsc::{Receiver as TokioReceiver, Sender as TokioSender};
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::fs::File;
use std::io::Write;
use crate::sensor_data::SensorData;
use std::time::Duration;

#[derive(Serialize)]
struct CompressedData {
    name: &'static str,
    value: f32,
    timestamp: String,
}

fn write_log(log_file: &Arc<Mutex<File>>, msg: &str) {
    println!("{}", msg);
    if let Ok(mut f) = log_file.lock() {
        let _ = writeln!(f, "{}", msg);
    }
}

pub fn spawn_forward_crossbeam_to_tokio(
    cross_rx: crossbeam_channel::Receiver<SensorData>,
    tokio_tx: TokioSender<SensorData>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        while let Ok(data) = cross_rx.recv() {
            if tokio_tx.blocking_send(data).is_err() {
                break;
            }
        }
    })
}

pub async fn downlink_manager_async(
    mut rx: TokioReceiver<SensorData>,
    buffer_capacity: usize,
    flush_interval_ms: u64,
    log_file: Arc<Mutex<File>>,
    running: Arc<AtomicBool>,
) {
    let init_start = Instant::now();

    let conn = match Connection::connect(
        "amqp://guest:guest@127.0.0.1:5672/%2f",
        ConnectionProperties::default(),
    ).await {
        Ok(c) => c,
        Err(e) => {
            write_log(&log_file, &format!("[{}] ERROR: Failed to connect: {:?}", Utc::now(), e));
            return;
        }
    };

    let channel = match conn.create_channel().await {
        Ok(ch) => ch,
        Err(e) => {
            write_log(&log_file, &format!("[{}] ERROR: Failed to create channel: {:?}", Utc::now(), e));
            return;
        }
    };

    let init_ms = init_start.elapsed().as_millis();
    if init_ms > 200 {
        write_log(&log_file, &format!("[{}] DOWNLINK INIT WARNING: {}ms (>5ms)", Utc::now(), init_ms));
        return;
    } else {
        write_log(&log_file, &format!("[{}] Downlink initialized in {}ms", Utc::now(), init_ms));
    }

    // Declare exchange & queue
    if let Err(e) = channel.exchange_declare(
        "sensor_exchange", ExchangeKind::Direct, ExchangeDeclareOptions::default(), FieldTable::default()
    ).await {
        write_log(&log_file, &format!("[{}] ERROR: exchange_declare failed: {:?}", Utc::now(), e));
        return;
    }

    if let Err(e) = channel.queue_declare(
        "sensor_queue", QueueDeclareOptions::default(), FieldTable::default()
    ).await {
        write_log(&log_file, &format!("[{}] ERROR: queue_declare failed: {:?}", Utc::now(), e));
        return;
    }

    if let Err(e) = channel.queue_bind(
        "sensor_queue", "sensor_exchange", "sensor_queue", QueueBindOptions::default(), FieldTable::default()
    ).await {
        write_log(&log_file, &format!("[{}] ERROR: queue_bind failed: {:?}", Utc::now(), e));
        return;
    }

    let mut buffer: Vec<SensorData> = Vec::with_capacity(buffer_capacity);
    let mut degraded = false;

    let mut ticker = interval(TokioDuration::from_millis(flush_interval_ms));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        if !running.load(Ordering::Relaxed) {
            break;
        }

        tokio::select! {
            maybe = rx.recv() => {
                match maybe {
                    Some(data) => {
                                // To simulate rounded buffer overflow, slow down the rate in which rx handles samples
                                // std::thread::sleep(Duration::from_millis(400)); // slow processing

                        let fill_percent = (buffer.len() as f32 / buffer_capacity as f32) * 100.0;

                        if buffer.len() >= buffer_capacity {
                            write_log(&log_file, &format!("[{}] BUFFER FULL: Dropped {}", Utc::now(), data.name));
                        } else {
                            if fill_percent > 80.0 {
                                if !degraded {
                                    write_log(&log_file, &format!("[{}] Entering degraded mode ({:.2}%)", Utc::now(), fill_percent));
                                    degraded = true;
                                }
                                if data.priority >= 2 {
                                    buffer.push(data);
                                } else {
                                    write_log(&log_file, &format!("[{}] Dropped low priority {}", Utc::now(), data.name));
                                }
                            } else {
                                if degraded {
                                    write_log(&log_file, &format!("[{}] Exiting degraded mode ({:.2}%)", Utc::now(), fill_percent));
                                    degraded = false;
                                }
                                buffer.push(data);
                            }
                        }

                        if !buffer.is_empty() {
                            if let Err(e) = flush_buffer(&channel, &mut buffer, log_file.clone()).await {
                                write_log(&log_file, &format!("[{}] FLUSH ERROR: {:?}", Utc::now(), e));
                            }
                        }
                    }
                    None => {
                        if !buffer.is_empty() {
                            let _ = flush_buffer(&channel, &mut buffer, log_file.clone()).await;
                        }
                        write_log(&log_file, &format!("[{}] Receiver closed, exiting", Utc::now()));
                        return;
                    }
                }
            }
            _ = ticker.tick() => {
                if !buffer.is_empty() {
                    if let Err(e) = flush_buffer(&channel, &mut buffer, log_file.clone()).await {
                        write_log(&log_file, &format!("[{}] FLUSH ERROR (tick): {:?}", Utc::now(), e));
                    }
                }
            }
        }
    }

    // On shutdown, flush remaining
    if !buffer.is_empty() {
        let _ = flush_buffer(&channel, &mut buffer, log_file.clone()).await;
    }
}

async fn flush_buffer(channel: &Channel, buffer: &mut Vec<SensorData>, log_file: Arc<Mutex<File>>) -> Result<(), lapin::Error> {
    for data in buffer.iter() {
        let compressed = CompressedData {
            name: data.name,
            value: data.value,
            timestamp: Utc::now().to_rfc3339(),
        };
        let serialized = serde_json::to_vec(&compressed).unwrap();

        let start = Instant::now();
        channel.basic_publish(
            "sensor_exchange",
            "sensor_queue",
            BasicPublishOptions::default(),
            &serialized,
            BasicProperties::default()
        ).await?.await?;

        let latency_ms = start.elapsed().as_secs_f64() * 1000.0;
        if latency_ms > 30.0 {
            write_log(&log_file, &format!("[{}] LATENCY VIOLATION: {:.2}ms", Utc::now(), latency_ms));
        } else {
            write_log(&log_file, &format!("[{}] Sent {} | Latency {:.2}ms", Utc::now(), data.name, latency_ms));
        }
    }

    buffer.clear();
    Ok(())
}