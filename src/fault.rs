// src/fault.rs

use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::time::Duration;
use chrono::Utc;
use crossbeam_channel::Sender;
use crate::sensor_data::SensorData;
use std::fs::File;
use std::io::Write;

/// Inject periodic faults. Runs in background threads.
pub fn inject_faults(tx: Sender<SensorData>, log_file: Arc<Mutex<File>>, running: Arc<AtomicBool>) -> Vec<std::thread::JoinHandle<()>> {
    let mut handles = Vec::new();

    let tx_value = tx.clone();
    let log_value = Arc::clone(&log_file);
    let running_value = running.clone();
    handles.push(std::thread::spawn(move || {
        loop {
            if !running_value.load(Ordering::Relaxed) {
                break;
            }
            std::thread::sleep(Duration::from_secs(30));
            let data = SensorData {
                name: "faulty_sensor",
                value: -999.0,
                timestamp: Utc::now(),
                priority: 0,
            };
            let msg = format!("[{}] FAULT INJECTION: injecting faulty_sensor", Utc::now());
            println!("{}", &msg);
            if let Ok(mut f) = log_value.lock() { let _ = writeln!(f, "{}", msg); }
            if tx_value.send(data).is_err() {
                let err = format!("[{}] FAULT INJECTION: channel closed", Utc::now());
                println!("{}", &err);
                if let Ok(mut f) = log_value.lock() { let _ = writeln!(f, "{}", err); }
                break;
            }
        }
    }));

    let tx_time = tx.clone();
    let log_time = Arc::clone(&log_file);
    let running_time = running.clone();
    handles.push(std::thread::spawn(move || {
        loop {
            if !running_time.load(Ordering::Relaxed) {
                break;
            }
            std::thread::sleep(Duration::from_secs(60));

            let data = SensorData {
                name: "late_fault_sensor",
                value: 20.0,
                timestamp: Utc::now() - chrono::Duration::milliseconds(100), // increase number to higher than 200 to simulate simulation termination
                priority: 0,
            };
            let msg = format!("[{}] FAULT INJECTION: injecting late_fault_sensor (simulated delay)", Utc::now());
            println!("{}", &msg);
            if let Ok(mut f) = log_time.lock() { let _ = writeln!(f, "{}", msg); }
            if tx_time.send(data).is_err() {
                let err = format!("[{}] FAULT INJECTION: channel closed", Utc::now());
                println!("{}", &err);
                if let Ok(mut f) = log_time.lock() { let _ = writeln!(f, "{}", err); }
                break;
            }
        }
    }));

    handles
}