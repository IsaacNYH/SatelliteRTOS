// src/main.rs

use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::fs::File;
use std::time::Duration;
use std::io::{Read, Write};
use chrono::Utc;
use crossbeam_channel::bounded;
use scheduled_thread_pool::ScheduledThreadPool;
use tokio::sync::mpsc;
use regex::Regex;
use std::collections::HashMap;

mod sensor_data;
mod sensor;
mod scheduler;
mod downlink;
mod fault;
mod cpu;

use crate::sensor::simulate_sensor;
use crate::scheduler::schedule_tasks;
use crate::downlink::{downlink_manager_async, spawn_forward_crossbeam_to_tokio};
use crate::fault::inject_faults;

fn create_named_log_file(name: &str) -> (Arc<Mutex<File>>, String) {
    let path = format!("{}.txt", name); // constant filename
    let f = File::create(&path).expect("Failed to create log file");
    println!("Created log file: {}", path);
    (Arc::new(Mutex::new(f)), path)
}


#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let running = Arc::new(AtomicBool::new(true));

    let (general_log, general_path) = create_named_log_file("ocs_log");
    let (scheduler_log, scheduler_path) = create_named_log_file("scheduler_log");
    let (downlink_log, downlink_path) = create_named_log_file("downlink_log");
    let (cpu_log, cpu_path) = create_named_log_file("cpu_log");

    let msg = format!("[{}] Satellite OCS Simulation Started", Utc::now());
    println!("{}", &msg);
    {
        let mut f = general_log.lock().unwrap();
        let _ = writeln!(f, "{}", &msg); 
    }

    let pool = Arc::new(ScheduledThreadPool::new(8));

    // Channels
    let (sensor_tx, sensor_rx) = bounded::<sensor_data::SensorData>(100);
    let (downlink_cross_tx, downlink_cross_rx) = bounded::<sensor_data::SensorData>(50);
    let (downlink_tokio_tx, downlink_tokio_rx) = mpsc::channel::<sensor_data::SensorData>(5000);

    let forward_handle = spawn_forward_crossbeam_to_tokio(downlink_cross_rx, downlink_tokio_tx);
let now = std::time::Instant::now();
let expected_start_thermal = Arc::new(Mutex::new(now));
let expected_start_compression = Arc::new(Mutex::new(now));
let expected_start_health = Arc::new(Mutex::new(now));
let expected_start_antenna = Arc::new(Mutex::new(now));

// CPU logger
let cpu_handle = cpu::cpu_logger(running.clone(), cpu_log.clone());

// SENSOR SIMULATION
// Use periods corresponding to RMS priorities (shorter period = higher priority)
let thermal_handle = simulate_sensor("thermal", 7_500, sensor_tx.clone(), general_log.clone(), running.clone());
let compression_handle = simulate_sensor("compression", 10_000, sensor_tx.clone(), general_log.clone(), running.clone());
let health_handle = simulate_sensor("health", 12_500, sensor_tx.clone(), general_log.clone(), running.clone());
let antenna_handle = simulate_sensor("antenna", 20_000, sensor_tx.clone(), general_log.clone(), running.clone());

// FAULT INJECTION
let fault_handles = inject_faults(sensor_tx.clone(), general_log.clone(), running.clone());

// Prepare expected start times map for scheduler
let mut expected_start_map: HashMap<scheduler::TaskType, Arc<Mutex<std::time::Instant>>> = HashMap::new();
expected_start_map.insert(scheduler::TaskType::ThermalControl, expected_start_thermal.clone());
expected_start_map.insert(scheduler::TaskType::DataCompression, expected_start_compression.clone());
expected_start_map.insert(scheduler::TaskType::HealthMonitoring, expected_start_health.clone());
expected_start_map.insert(scheduler::TaskType::AntennaAlignment, expected_start_antenna.clone());

// Start scheduler thread
let running_clone = running.clone();
let pool_clone = pool.clone();
let scheduler_handle = std::thread::spawn(move || {
    schedule_tasks(
        sensor_rx,
        downlink_cross_tx,
        pool_clone,
        expected_start_map,
        scheduler_log,
        running_clone,
    );
});


    let running_down = running.clone();
    let downlink_handle = tokio::spawn(downlink_manager_async(
        downlink_tokio_rx,
        300,
        100,
        downlink_log,
        running_down,
    ));

    tokio::time::sleep(Duration::from_secs(180)).await;

    running.store(false, Ordering::Relaxed);

    drop(sensor_tx);

thermal_handle.join().unwrap();
compression_handle.join().unwrap();
health_handle.join().unwrap();
antenna_handle.join().unwrap();

    for h in fault_handles {
        h.join().unwrap();
    }
    cpu_handle.join().unwrap();

    if let Err(e) = scheduler_handle.join() {
        println!("[{}] Warning: scheduler thread join failed: {:?}", Utc::now(), e);
    }

    forward_handle.join().unwrap();

    if let Err(e) = downlink_handle.await {
        println!("[{}] Warning: downlink task join failed: {:?}", Utc::now(), e);
    }

    let done_msg = format!("[{}] SIMULATION COMPLETE.", Utc::now());
    println!("{}", &done_msg);
    {
        let mut f = general_log.lock().unwrap();
        let _ = writeln!(f, "{}", &done_msg);
    }

    // Generate evaluation log sheet
    generate_evaluation_log(&general_path, &scheduler_path, &downlink_path, &cpu_path);
}

pub fn generate_evaluation_log(
    general_path: &str,
    scheduler_path: &str,
    _downlink_path: &str,
    cpu_path: &str,
) {
    let mut metrics: HashMap<String, Vec<f64>> = HashMap::new();
    let counts: HashMap<String, usize> = HashMap::new();

    // ----------------- JITTER (Sensors) -----------------
    let mut content = String::new();
    File::open(general_path)
        .expect("Failed to open general log")
        .read_to_string(&mut content)
        .unwrap();

    let re_jitter = Regex::new(r"Sensor (\w+) jitter \(MAD\): (\d+\.\d+)ms").unwrap();
    for cap in re_jitter.captures_iter(&content) {
        let sensor = cap[1].to_string();
        let key = format!("jitter_{}", sensor);
        metrics.entry(key).or_insert(Vec::new()).push(cap[2].parse::<f64>().unwrap());
    }

    // ----------------- DRIFT (Scheduler) -----------------
    content.clear();
    File::open(scheduler_path)
        .expect("Failed to open scheduler log")
        .read_to_string(&mut content)
        .unwrap();

    let re_drift = Regex::new(r"SCHEDULING DRIFT for (\w+): (\d+\.\d+)ms").unwrap();
    for cap in re_drift.captures_iter(&content) {
        let task = cap[1].to_string();
        let key = format!("drift_{}", task);
        metrics.entry(key).or_insert(Vec::new()).push(cap[2].parse::<f64>().unwrap());
    }

    // ----------------- FAULT RECOVERY TIME -----------------
    let re_recovery = Regex::new(r"Fault recovery time: (\d+\.\d+)ms").unwrap();
    for cap in re_recovery.captures_iter(&content) {
        metrics.entry("fault_recovery_time".to_string())
            .or_insert(Vec::new())
            .push(cap[1].parse::<f64>().unwrap());
    }

    // ----------------- CPU UTILIZATION -----------------
    content.clear();
    File::open(cpu_path)
        .expect("Failed to open CPU log")
        .read_to_string(&mut content)
        .unwrap();

    let re_cpu = Regex::new(r"CPU Active: (\d+\.\d+)%").unwrap();
    for cap in re_cpu.captures_iter(&content) {
        metrics.entry("cpu_utilization".to_string())
            .or_insert(Vec::new())
            .push(cap[1].parse::<f64>().unwrap());
    }

    // ----------------- WRITE EVALUATION LOG -----------------
    let eval_path = format!("evaluation_log_{}.txt", Utc::now().format("%Y-%m-%d_%H-%M-%S"));
    let mut eval_file = File::create(&eval_path).expect("Failed to create evaluation log");
    writeln!(eval_file, "[{}] Post-Run Evaluation Log", Utc::now()).unwrap();

    // Compute mean, min, max, std_dev for each metric
    for (key, values) in &metrics {
        if !values.is_empty() {
            let mean = values.iter().sum::<f64>() / values.len() as f64;
            let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
            let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let std_dev = if values.len() > 1 {
                let variance = values.iter().map(|&x| (x - mean).powi(2)).sum::<f64>() / (values.len() - 1) as f64;
                variance.sqrt()
            } else { 0.0 };
            writeln!(eval_file, "{}: mean={:.3}, std_dev={:.3}, min={:.3}, max={:.3}", key, mean, std_dev, min, max).unwrap();
        }
    }

    // Write counts
    for (key, count) in &counts {
        writeln!(eval_file, "{}: {}", key, count).unwrap();
    }

    // CPU active vs idle explicitly
    if let Some(cpu_vals) = metrics.get("cpu_utilization") {
        if !cpu_vals.is_empty() {
            let avg_active = cpu_vals.iter().sum::<f64>() / cpu_vals.len() as f64;
            let avg_idle = 100.0 - avg_active;
            writeln!(eval_file, "Average CPU: Active={:.2}%, Idle={:.2}%", avg_active, avg_idle).unwrap();
        }
    }

    println!("[{}] Evaluation log generated: {}", Utc::now(), eval_path);
}