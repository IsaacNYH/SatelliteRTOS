# SatelliteRTOS
This project simulates a real-time satellite onboard computer system built in Rust, emphasizing task scheduling, concurrency, and fault tolerance.



Key Features:
Concurrent sensor simulation with priority-based task generation.

Bounded buffer for inter-thread communication using crossbeam_channel.

Real-time scheduler with jitter and drift measurement.

CPU usage tracking via sysinfo.

Optional RabbitMQ integration for distributed message passing.

Fault injection to test recovery and resilience.

Benchmarking support with criterion or bmabenchmark.



Log Files (Output):
ocs_log.txt: 
Non scheduler-specific or CPU-specific tasks such assensor arrivals, latency, fault handling, safety alerts, and the start/stop banners.

scheduler_log.txt: 
Scheduler timing diagnostics: start-drift, start-delay violations, pre-emptions, and deadline misses.

downlink_log.txt: 
Downlink batch transmissions, queue warnings, and async manager shutdown.

cpu_log.txt: 
Periodic CPU-active percentage samples.

evaluation_log_*.txt: 
Statistical summary (mean / std-dev / min / max) of jitter, drift, fault recovery, and CPU usage.

benchmark_summary.txt: 
Aggregates detailed benchmark statistics across multiple simulation runs, including latency, scheduling accuracy, and preemption counts.
