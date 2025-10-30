use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::time::{Duration, Instant, SystemTime};
use chrono::Utc;
use crossbeam_channel::{Receiver, Sender};
use crate::sensor_data::SensorData;
use std::fs::File;
use std::io::Write;
use std::collections::HashMap;
use scheduled_thread_pool::ScheduledThreadPool;
use std::process;

/// Task types
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TaskType {
    ThermalControl,
    DataCompression,
    HealthMonitoring,
    AntennaAlignment,
}

/// Active task state for preemption
struct ActiveTask {
    priority: u8,
    should_yield: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    last_preempt: Arc<Mutex<Instant>>, // prevents repeated spam
}

pub fn schedule_tasks(
    rx: Receiver<SensorData>,
    tx_downlink: Sender<SensorData>,
    pool: Arc<ScheduledThreadPool>,
    expected_start_map: HashMap<TaskType, Arc<Mutex<Instant>>>,
    log_file: Arc<Mutex<File>>,
    running: Arc<AtomicBool>,
) {
    let mut thermal_miss_counter: usize = 0;
    let mut backoff_ms: u64 = 100;

    // Task configuration
    let mut task_config: HashMap<TaskType, (Duration, Duration, Duration, u8)> = HashMap::new();
    task_config.insert(TaskType::ThermalControl,   (Duration::from_micros(800), Duration::from_millis(15), Duration::from_millis(7), 3));
    task_config.insert(TaskType::DataCompression,  (Duration::from_micros(600), Duration::from_millis(15), Duration::from_millis(10), 2));
    task_config.insert(TaskType::HealthMonitoring, (Duration::from_micros(400), Duration::from_millis(15), Duration::from_millis(15), 1));
    task_config.insert(TaskType::AntennaAlignment, (Duration::from_micros(200), Duration::from_millis(15), Duration::from_millis(20), 1));

    let mut start_delay_thresholds: HashMap<TaskType, f64> = HashMap::new();
    start_delay_thresholds.insert(TaskType::ThermalControl, 10.0);
    start_delay_thresholds.insert(TaskType::DataCompression, 15.0);
    start_delay_thresholds.insert(TaskType::HealthMonitoring, 20.0);
    start_delay_thresholds.insert(TaskType::AntennaAlignment, 20.0);

    let active_tasks: Arc<Mutex<Vec<ActiveTask>>> = Arc::new(Mutex::new(Vec::new()));

    for (task_type, &(exec_time, deadline, period, priority)) in task_config.iter() {
        let task_type_clone = task_type.clone();
        let expected_lock = expected_start_map.get(task_type).unwrap().clone();
        let log_file_clone = log_file.clone();
        let tx_downlink_clone = tx_downlink.clone();
        let running_clone = running.clone();
        let active_tasks_clone = active_tasks.clone();
        let threshold_ms = start_delay_thresholds.get(task_type).copied().unwrap_or(5.0);

        let initial_delay = period / 2;

        pool.execute_at_fixed_rate(initial_delay, period, move || {
            if !running_clone.load(Ordering::Relaxed) {
                return;
            }

            // Actual start timestamp
            let actual_start = Instant::now();

// START delay check
{
    let mut expected = expected_lock.lock().unwrap();
    let start_delay = actual_start.saturating_duration_since(*expected);
    let start_delay_ms = start_delay.as_secs_f64() * 1000.0;

    // Log drift (always)
    let drift_msg = format!(
        "[{}] SCHEDULING DRIFT for {:?}: {:.3}ms",
        Utc::now(),
        task_type_clone,
        start_delay_ms
    );
    if let Ok(mut f) = log_file_clone.lock() { let _ = writeln!(f, "{}", drift_msg); }

    // Log violation if over threshold
    if start_delay_ms > threshold_ms {
        let msg = format!(
            "[{}] START DELAY VIOLATION for {:?}: {:.3}ms (threshold: {:.3}ms)",
            Utc::now(),
            task_type_clone,
            start_delay_ms,
            threshold_ms
        );
        if let Ok(mut f) = log_file_clone.lock() { let _ = writeln!(f, "{}", msg); }
    }

    *expected = actual_start + period;
}


            // Task control flags
            let task_running = Arc::new(AtomicBool::new(true));
            let yield_flag = Arc::new(AtomicBool::new(false));
            let last_preempt = Arc::new(Mutex::new(Instant::now() - Duration::from_secs(10)));

            // Add task to active_tasks AFTER start
            {
                let mut tasks = active_tasks_clone.lock().unwrap();
                tasks.push(ActiveTask {
                    priority,
                    should_yield: yield_flag.clone(),
                    running: task_running.clone(),
                    last_preempt: last_preempt.clone(),
                });
            }

            // Preemption logic (only ThermalControl)
            if priority == 3 {
                let tasks = active_tasks_clone.lock().unwrap();
                for t in tasks.iter() {
                    if t.priority < priority && t.running.load(Ordering::Relaxed) {
                        let mut last = t.last_preempt.lock().unwrap();
                        if last.elapsed() > Duration::from_millis(5) {
                            t.should_yield.store(true, Ordering::Relaxed);
                            *last = Instant::now();

                            let msg = format!(
                                "[{}] PREEMPTION: {:?} overriding lower-priority task (priority {})",
                                Utc::now(), task_type_clone, t.priority
                            );
                            let _ = log_file_clone.lock().unwrap().write_all(msg.as_bytes());
                            let _ = log_file_clone.lock().unwrap().write_all(b"\n");
                        }
                    }
                }
            }

            // Simulate task execution
            let start = Instant::now();
            while start.elapsed() < exec_time {
                if yield_flag.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(50)); //change duration to higher value to simulate completion delay (50ms). 
                    continue;
                }
            }

            // Mark task as finished
            task_running.store(false, Ordering::Relaxed);

            // COMPLETION delay check
            let completion_time = Instant::now();
            let completion_delay = completion_time.saturating_duration_since(actual_start);
            if completion_delay > deadline {
                let completion_delay_ms = completion_delay.as_secs_f64() * 1000.0;
                let msg = format!(
                    "[{}] COMPLETION DEADLINE VIOLATION for {:?}: {:.3}ms (deadline: {:.3?})",
                    Utc::now(), task_type_clone, completion_delay_ms, deadline
                );
                println!("{}", &msg);
                let _ = log_file_clone.lock().unwrap().write_all(msg.as_bytes());
                let _ = log_file_clone.lock().unwrap().write_all(b"\n");
            }

            // Remove task from active_tasks
            {
                let mut tasks = active_tasks_clone.lock().unwrap();
                tasks.retain(|t| !Arc::ptr_eq(&t.should_yield, &yield_flag));
            }

            // Forward telemetry
            let telemetry = SensorData {
                name: match task_type_clone {
                    TaskType::ThermalControl => "thermal",
                    TaskType::DataCompression => "compression",
                    TaskType::HealthMonitoring => "health",
                    TaskType::AntennaAlignment => "antenna",
                },
                value: 0.0,
                timestamp: Utc::now(),
                priority,
            };

            let _ = tx_downlink_clone.try_send(telemetry);
        });
    }

    // Consumer loop for sensor samples
    while let Ok(data) = rx.recv() {
        let now_sys = SystemTime::now();
        let timestamp_sys: SystemTime = data.timestamp.into();
        let latency_ms: f64 = now_sys.duration_since(timestamp_sys).unwrap_or(Duration::ZERO).as_secs_f64() * 1000.0;

        let msg = format!("[{}] Received sensor: {} val {:.3} | latency {:.4}ms", Utc::now(), data.name, data.value, latency_ms);
        println!("{}", &msg);
        if let Ok(mut f) = log_file.lock() { let _ = writeln!(f, "{}", msg); }

        // Fault handling
        if data.name == "faulty_sensor" || data.name == "late_fault_sensor" {
            let recovery_time = latency_ms;
            let msg = format!("[{}] Fault recovery time: {:.4}ms ({})", Utc::now(), recovery_time, data.name);
            println!("{}", &msg);
            if let Ok(mut f) = log_file.lock() { let _ = writeln!(f, "{}", msg); }
            if recovery_time > 200.0 {
                let abort_msg = format!("[{}] SIMULATED MISSION ABORT: recovery time exceeds 200ms ({})", Utc::now(), data.name);
                println!("{}", &abort_msg);
                if let Ok(mut f) = log_file.lock() { let _ = writeln!(f, "{}", abort_msg); }
                process::exit(1);
            }
        }

        // Thermal safety
        if data.name == "thermal" && latency_ms > 1000.0 {
            thermal_miss_counter += 1;
            if thermal_miss_counter >= 3 {
                let alert = format!("[{}] SAFETY ALERT: CRITICAL DATA (THERMAL) MISSED >3 CYCLES, backoff {}ms", Utc::now(), backoff_ms);
                println!("{}", &alert);
                if let Ok(mut f) = log_file.lock() { let _ = writeln!(f, "{}", alert); }
                std::thread::sleep(Duration::from_millis(backoff_ms));
                backoff_ms = (backoff_ms * 2).min(5000);
                thermal_miss_counter = 0;
            }
        } else {
            thermal_miss_counter = 0;
            backoff_ms = 100;
        }

        // Forward sample to downlink
        if let Err(e) = tx_downlink.try_send(data) {
            let msg = format!("[{}] DOWNLINK forward failed: {:?}", Utc::now(), e);
            println!("{}", &msg);
            if let Ok(mut f) = log_file.lock() { let _ = writeln!(f, "{}", msg); }
        }
    }

    let msg = format!("[{}] schedule_tasks: sensor receiver closed, exiting", Utc::now());
    println!("{}", &msg);
    if let Ok(mut f) = log_file.lock() { let _ = writeln!(f, "{}", msg); }
}
