// src/cpu.rs

use std::sync::{Arc, atomic::{AtomicBool, Ordering}, Mutex};
use std::time::Duration;
use std::io::Write;
use sysinfo::{System, Cpu};
use std::fs::File;

/// Spawns a thread to periodically log CPU usage (active vs idle %) 
/// into the provided cpu_log file.
pub fn cpu_logger(running: Arc<AtomicBool>, log_file: Arc<Mutex<File>>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut system = System::new_all();

        // Warm up: first refresh is always 0.0
        system.refresh_all();
        std::thread::sleep(Duration::from_millis(200));

        loop {
            if !running.load(Ordering::Relaxed) {
                break;
            }

            system.refresh_all();

            let cpus = system.cpus();
            let avg_busy = if !cpus.is_empty() {
                cpus.iter().map(|cpu: &Cpu| cpu.cpu_usage()).sum::<f32>() / cpus.len() as f32
            } else {
                0.0
            };
            let avg_idle = 100.0 - avg_busy;

            let msg = format!("CPU Active: {:.2}% | Idle: {:.2}%", avg_busy, avg_idle);

            println!("{}", &msg);

            // Write to the shared CPU log file
            if let Ok(mut f) = log_file.lock() {
                let _ = writeln!(f, "{}", msg);
                let _ = f.flush();
            }

            std::thread::sleep(Duration::from_secs(1));
        }
    })
}
