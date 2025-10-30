use std::fs::File;
use std::collections::HashMap;
use chrono::Utc;
use regex::Regex;
use std::io::{Read, Write};

#[derive(Debug, Default)]
struct Metrics {
    sensor_latencies: Vec<f64>,
    jitter: HashMap<String, Vec<f64>>,
    drift: HashMap<String, Vec<f64>>,
    start_delays: HashMap<String, Vec<f64>>,
    completion_deadlines: HashMap<String, Vec<f64>>,
    fault_recovery_time: Vec<f64>,
    cpu_active: Vec<f64>,
    cpu_idle: Vec<f64>,
    preemptions: HashMap<String, usize>,
    rejected_commands: Vec<String>,
}

impl Metrics {
    fn merge(&mut self, other: Metrics) {
        self.sensor_latencies.extend(other.sensor_latencies);
        for (k, v) in other.jitter { self.jitter.entry(k).or_default().extend(v); }
        for (k, v) in other.drift { self.drift.entry(k).or_default().extend(v); }
        for (k, v) in other.start_delays { self.start_delays.entry(k).or_default().extend(v); }
        for (k, v) in other.completion_deadlines { self.completion_deadlines.entry(k).or_default().extend(v); }
        self.fault_recovery_time.extend(other.fault_recovery_time);
        self.cpu_active.extend(other.cpu_active);
        self.cpu_idle.extend(other.cpu_idle);
        for (k, v) in other.preemptions { *self.preemptions.entry(k).or_default() += v; }
        self.rejected_commands.extend(other.rejected_commands);
    }

    fn aggregate(&self) -> HashMap<String, HashMap<String, f64>> {
        fn stats(values: &[f64]) -> HashMap<String, f64> {
            if values.is_empty() { return HashMap::new(); }
            let mean = values.iter().sum::<f64>() / values.len() as f64;
            let std_dev = if values.len() > 1 {
                (values.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (values.len() - 1) as f64).sqrt()
            } else { 0.0 };
            HashMap::from([
                ("mean".to_string(), mean),
                ("std_dev".to_string(), std_dev),
                ("min".to_string(), *values.iter().min_by(|a,b| a.partial_cmp(b).unwrap()).unwrap()),
                ("max".to_string(), *values.iter().max_by(|a,b| a.partial_cmp(b).unwrap()).unwrap()),
            ])
        }

        let mut agg = HashMap::new();
        if !self.sensor_latencies.is_empty() { agg.insert("sensor_latencies".into(), stats(&self.sensor_latencies)); }
        for (k, v) in &self.jitter { agg.insert(format!("jitter_{}", k), stats(v)); }
        for (k, v) in &self.drift { agg.insert(format!("drift_{}", k), stats(v)); }
        for (k, v) in &self.start_delays { agg.insert(format!("start_delay_{}", k), stats(v)); }
        for (k, v) in &self.completion_deadlines { agg.insert(format!("completion_deadline_{}", k), stats(v)); }
        if !self.fault_recovery_time.is_empty() { agg.insert("fault_recovery_time".into(), stats(&self.fault_recovery_time)); }
        if !self.cpu_active.is_empty() { 
            agg.insert("cpu_active".into(), stats(&self.cpu_active));
            agg.insert("cpu_idle".into(), stats(&self.cpu_idle));
        }
        agg
    }
}

fn parse_log(log_path: &str) -> Metrics {
    let mut metrics = Metrics::default();
    let mut content = String::new();
    File::open(log_path).unwrap().read_to_string(&mut content).unwrap();

    // Flexible regex patterns
    let re_sensor = Regex::new(r"(?i)Received sensor: .*? \| latency (\d+\.?\d*)ms").unwrap();
    let re_jitter = Regex::new(r"(?i)Sensor (\w+) jitter.*?: (\d+\.?\d*)ms").unwrap();
    let re_drift = Regex::new(r"(?i)SCHEDULING DRIFT for (\w+): (\d+\.?\d*)ms").unwrap();
    let re_start_delay = Regex::new(r"(?i)START DELAY VIOLATION for (\w+): (\d+\.?\d*)ms").unwrap();
    let re_completion = Regex::new(r"(?i)COMPLETION DEADLINE VIOLATION for (\w+): (\d+\.?\d*)ms").unwrap();
    let re_recovery = Regex::new(r"(?i)Fault recovery time: (\d+\.?\d*)ms").unwrap();
    let re_cpu = Regex::new(r"(?i)CPU Active: (\d+\.?\d+)%.*?Idle: (\d+\.?\d+)%").unwrap();
    let re_preemption = Regex::new(r"(?i)PREEMPTION: (\w+) overriding").unwrap();
    let re_rejected = Regex::new(r"(?i)Command .*? rejected: .*").unwrap();

    for cap in re_sensor.captures_iter(&content) { metrics.sensor_latencies.push(cap[1].parse().unwrap()); }
    for cap in re_jitter.captures_iter(&content) { metrics.jitter.entry(cap[1].to_string()).or_default().push(cap[2].parse().unwrap()); }
    for cap in re_drift.captures_iter(&content) { metrics.drift.entry(cap[1].to_string()).or_default().push(cap[2].parse().unwrap()); }
    for cap in re_start_delay.captures_iter(&content) { metrics.start_delays.entry(cap[1].to_string()).or_default().push(cap[2].parse().unwrap()); }
    for cap in re_completion.captures_iter(&content) { metrics.completion_deadlines.entry(cap[1].to_string()).or_default().push(cap[2].parse().unwrap()); }
    for cap in re_recovery.captures_iter(&content) { metrics.fault_recovery_time.push(cap[1].parse().unwrap()); }
    for cap in re_cpu.captures_iter(&content) { 
        metrics.cpu_active.push(cap[1].parse().unwrap());
        metrics.cpu_idle.push(cap[2].parse().unwrap());
    }
    for cap in re_preemption.captures_iter(&content) { *metrics.preemptions.entry(cap[1].to_string()).or_default() += 1; }
    for cap in re_rejected.captures_iter(&content) { metrics.rejected_commands.push(cap[0].to_string()); }

    metrics
}

fn run_benchmark(log_paths: &[&str]) -> Metrics {
    let mut combined = Metrics::default();

    for path in log_paths {
        let m = parse_log(path);
        combined.merge(m);
    }

    // Aggregate metrics
    let agg = combined.aggregate();

    // Write summary
    let mut summary = File::create("benchmark_summary.txt").unwrap();
    writeln!(summary, "[{}] Benchmark Summary:", Utc::now()).unwrap();
    for (key, stats) in &agg { writeln!(summary, "{}: {:?}", key, stats).unwrap(); }

    if !combined.cpu_active.is_empty() {
        let avg_active = combined.cpu_active.iter().sum::<f64>() / combined.cpu_active.len() as f64;
        let avg_idle = combined.cpu_idle.iter().sum::<f64>() / combined.cpu_idle.len() as f64;
        writeln!(summary, "Average CPU: Active={:.2}%, Idle={:.2}%", avg_active, avg_idle).unwrap();
    }

    writeln!(summary, "Preemptions per task: {:?}", combined.preemptions).unwrap();
    writeln!(summary, "Rejected commands: {:?}", combined.rejected_commands).unwrap();

    println!("[{}] Benchmark summary written", Utc::now());
    combined
}

// ----------------- main -----------------
fn main() {

let logs = vec![
    "ocs_log.txt",
    "scheduler_log.txt",
    "cpu_log.txt",
    "downlink_log.txt",
];

    let _agg = run_benchmark(&logs);
}
