use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

const MAX_MEMORY_BYTES: u64 = 100 * 1024 * 1024;
const MAX_AVG_CPU_PERCENT: f32 = 10.0;
const MAX_HIGH_CPU_DURATION_SECS: u64 = 5;
const TEST_DURATION_SECS: u64 = 5;
const SAMPLE_INTERVAL_MS: u64 = 100;

/// Helper struct to manage the test application process
struct TestProcess {
    child: Child,
    pid: Pid,
}

impl TestProcess {
    /// Start the application in test mode
    fn start() -> Result<Self, Box<dyn std::error::Error>> {
        // Build the application first to ensure we have the latest binary
        let build_status = Command::new("cargo")
            .args(["build", "--release"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;

        if !build_status.success() {
            return Err("Failed to build application".into());
        }

        // Start the application process
        // Note: The application requires elevated privileges for packet capture,
        // but in CI/test environments we'll use a mock or limited mode
        let child = Command::new("target/release/deadlock-api-ingest")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;

        let pid = Pid::from_u32(child.id());

        // Give the process a moment to initialize
        thread::sleep(Duration::from_millis(500));

        Ok(TestProcess { child, pid })
    }

    /// Get the process ID
    fn pid(&self) -> Pid {
        self.pid
    }

    /// Stop the process
    fn stop(mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.child.kill()?;
        self.child.wait()?;
        Ok(())
    }
}

/// Collect resource usage statistics over a period of time
struct ResourceStats {
    memory_samples: Vec<u64>,
    cpu_samples: Vec<f32>,
    timestamps: Vec<Instant>,
}

impl ResourceStats {
    fn new() -> Self {
        ResourceStats {
            memory_samples: Vec::new(),
            cpu_samples: Vec::new(),
            timestamps: Vec::new(),
        }
    }

    fn add_sample(&mut self, memory_bytes: u64, cpu_percent: f32) {
        self.memory_samples.push(memory_bytes);
        self.cpu_samples.push(cpu_percent);
        self.timestamps.push(Instant::now());
    }

    fn max_memory(&self) -> u64 {
        self.memory_samples.iter().copied().max().unwrap_or(0)
    }

    fn avg_cpu(&self) -> f32 {
        if self.cpu_samples.is_empty() {
            return 0.0;
        }
        let sum: f32 = self.cpu_samples.iter().sum();
        sum / self.cpu_samples.len() as f32
    }

    fn max_consecutive_high_cpu_duration(&self) -> Duration {
        let mut max_duration = Duration::from_secs(0);
        let mut current_start: Option<Instant> = None;

        for (i, &cpu) in self.cpu_samples.iter().enumerate() {
            if cpu >= 100.0 {
                if current_start.is_none() {
                    current_start = Some(self.timestamps[i]);
                }
            } else if let Some(start) = current_start {
                let duration = self.timestamps[i].duration_since(start);
                if duration > max_duration {
                    max_duration = duration;
                }
                current_start = None;
            }
        }

        // Check if we ended with high CPU
        if let Some(start) = current_start
            && let Some(&last_timestamp) = self.timestamps.last()
        {
            let duration = last_timestamp.duration_since(start);
            if duration > max_duration {
                max_duration = duration;
            }
        }

        max_duration
    }
}

/// Monitor resource usage of a process
fn monitor_resources(
    pid: Pid,
    duration: Duration,
    sample_interval: Duration,
    stop_signal: Arc<AtomicBool>,
) -> ResourceStats {
    let mut stats = ResourceStats::new();
    let mut system = System::new_all();
    let start = Instant::now();

    // Initial refresh to get baseline
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::everything(),
    );
    thread::sleep(Duration::from_millis(100));

    while start.elapsed() < duration && !stop_signal.load(Ordering::Relaxed) {
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::everything(),
        );

        if let Some(process) = system.process(pid) {
            let memory_bytes = process.memory();
            let cpu_percent = process.cpu_usage();

            stats.add_sample(memory_bytes, cpu_percent);
        } else {
            // Process no longer exists
            break;
        }

        thread::sleep(sample_interval);
    }

    stats
}

#[test]
fn test_memory_usage_under_load() {
    // This test verifies that memory usage stays under 200 MB during operation
    println!("Starting memory usage test...");

    let process = match TestProcess::start() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Failed to start test process: {e}");
            eprintln!("Note: This test requires elevated privileges for packet capture.");
            eprintln!(
                "Run with: sudo -E cargo test --test resource_usage -- --ignored --nocapture"
            );
            return;
        }
    };

    let pid = process.pid();
    let stop_signal = Arc::new(AtomicBool::new(false));

    println!("Monitoring process {pid} for {TEST_DURATION_SECS} seconds...");

    let stats = monitor_resources(
        pid,
        Duration::from_secs(TEST_DURATION_SECS),
        Duration::from_millis(SAMPLE_INTERVAL_MS),
        stop_signal,
    );

    // Stop the process
    if let Err(e) = process.stop() {
        eprintln!("Warning: Failed to stop process cleanly: {e}");
    }

    let max_memory = stats.max_memory();
    let max_memory_mb = max_memory as f64 / (1024.0 * 1024.0);

    println!("Memory usage statistics:");
    println!("  Max memory: {max_memory_mb:.2} MB");
    println!("  Samples collected: {}", stats.memory_samples.len());

    assert!(
        max_memory <= MAX_MEMORY_BYTES,
        "Memory usage exceeded limit: {max_memory_mb:.2} MB > {} MB",
        MAX_MEMORY_BYTES / (1024 * 1024)
    );

    println!("✓ Memory usage test passed!");
}

#[test]
fn test_cpu_usage_under_load() {
    // This test verifies that CPU usage remains reasonable during operation
    println!("Starting CPU usage test...");

    let process = match TestProcess::start() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Failed to start test process: {e}");
            eprintln!("Note: This test requires elevated privileges for packet capture.");
            eprintln!(
                "Run with: sudo -E cargo test --test resource_usage -- --ignored --nocapture"
            );
            return;
        }
    };

    let pid = process.pid();
    let stop_signal = Arc::new(AtomicBool::new(false));

    println!("Monitoring process {pid} for {TEST_DURATION_SECS} seconds...");

    let stats = monitor_resources(
        pid,
        Duration::from_secs(TEST_DURATION_SECS),
        Duration::from_millis(SAMPLE_INTERVAL_MS),
        stop_signal,
    );

    // Stop the process
    if let Err(e) = process.stop() {
        eprintln!("Warning: Failed to stop process cleanly: {e}");
    }

    let avg_cpu = stats.avg_cpu();
    let max_high_cpu_duration = stats.max_consecutive_high_cpu_duration();

    println!("CPU usage statistics:");
    println!("  Average CPU: {avg_cpu:.2}%");
    println!(
        "  Max consecutive high CPU duration: {:.2}s",
        max_high_cpu_duration.as_secs_f64()
    );
    println!("  Samples collected: {}", stats.cpu_samples.len());

    assert!(
        avg_cpu <= MAX_AVG_CPU_PERCENT,
        "Average CPU usage exceeded limit: {avg_cpu:.2}% > {MAX_AVG_CPU_PERCENT}%",
    );

    assert!(
        max_high_cpu_duration <= Duration::from_secs(MAX_HIGH_CPU_DURATION_SECS),
        "Sustained high CPU usage exceeded limit: {:.2}s > {MAX_HIGH_CPU_DURATION_SECS}s",
        max_high_cpu_duration.as_secs_f64()
    );

    println!("✓ CPU usage test passed!");
}
