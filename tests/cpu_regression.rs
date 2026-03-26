//! CPU regression harness for piped stdin + keyboard path.
//!
//! A historic bug kept the main thread in a tight poll/read loop against the pipe
//! (using mio/kqueue to watch stdin, which is always "ready" when piped), driving
//! qtail to ~96% CPU. This test floods stdin for several seconds and samples the
//! child process %cpu via `ps`.
//!
//! # `kernel_task`
//! On macOS, `kernel_task` is not causally tied to a single user PID — it aggregates
//! thermal throttling, memory compression, driver work, etc. It cannot be asserted
//! from user space. To correlate manually: note the child PID printed during this
//! test and compare with Activity Monitor / `top -o cpu` while the test is running.
#![cfg(unix)]

use std::io::Write;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

fn qtail_bin() -> std::path::PathBuf {
    // CARGO_BIN_EXE_qtail is set by cargo test when running integration tests.
    // Fall back to locating the binary next to the test runner for `just cpu-regression`.
    if let Some(p) = std::env::var_os("CARGO_BIN_EXE_qtail") {
        return p.into();
    }
    // Test runner is at target/{profile}/deps/cpu_regression-*, binary at target/{profile}/qtail.
    let test_exe = std::env::current_exe().expect("current_exe");
    test_exe
        .parent() // target/{profile}/deps
        .and_then(|p| p.parent()) // target/{profile}
        .map(|p| p.join("qtail"))
        .filter(|p| p.exists())
        .unwrap_or_else(|| {
            panic!(
                "❌ could not find qtail binary; run `cargo build` first, then `just cpu-regression`"
            )
        })
}

fn sample_cpu_percent(pid: u32) -> Option<f64> {
    let out = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "%cpu="])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

/// Floods piped stdin for several seconds while sampling the child `%cpu` via `ps`.
///
/// Ignored by default — timing-sensitive and not suitable for CI. Run locally:
/// ```
/// just cpu-regression
/// ```
/// or:
/// ```
/// cargo test --test cpu_regression -- --ignored --nocapture
/// ```
#[test]
#[ignore = "samples live CPU via ps; run: just cpu-regression"]
fn qtail_child_cpu_stays_low_with_piped_stdin_flood() {
    const FLOOD_SECS: u64 = 5;
    const SAMPLE_INTERVAL_MS: u64 = 250;
    // ps reports per-core %, so on an M1 Pro (10 cores) a single-threaded 100% spin
    // shows as ~100%. We allow up to 120% to tolerate brief startup bursts.
    const MAX_CPU_PCT: f64 = 120.0;

    let mut child = Command::new(qtail_bin())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("⚠️  failed to spawn qtail — did you run `just cpu-regression`?");

    let pid = child.id();
    eprintln!("✅ qtail pid={pid} — correlate kernel_task in Activity Monitor with this PID");

    let mut stdin = child.stdin.take().expect("stdin pipe");
    let writer = thread::spawn(move || {
        let line = format!("{}\n", "x".repeat(200));
        let deadline = Instant::now() + Duration::from_secs(FLOOD_SECS);
        while Instant::now() < deadline {
            if stdin.write_all(line.as_bytes()).is_err() {
                break;
            }
        }
    });

    let started = Instant::now();
    let mut samples: Vec<f64> = Vec::new();
    while started.elapsed() < Duration::from_secs(FLOOD_SECS) {
        thread::sleep(Duration::from_millis(SAMPLE_INTERVAL_MS));
        if let Some(cpu) = sample_cpu_percent(pid) {
            samples.push(cpu);
        }
    }

    let _ = child.kill();
    let _ = child.wait();
    let _ = writer.join();

    assert!(
        !samples.is_empty(),
        "❌ no CPU samples for pid {pid} — is `ps -p <pid> -o %%cpu=` available on this platform?"
    );

    let max_cpu = samples.iter().copied().fold(0.0_f64, f64::max);
    let mean_cpu = samples.iter().sum::<f64>() / samples.len() as f64;

    eprintln!(
        "cpu_regression: pid={pid} samples={} mean={mean_cpu:.1}% max={max_cpu:.1}% (limit {MAX_CPU_PCT}%)",
        samples.len(),
    );

    assert!(
        max_cpu < MAX_CPU_PCT,
        "❌ qtail max CPU {max_cpu:.1}% exceeded {MAX_CPU_PCT}% — possible busy loop. mean={mean_cpu:.1}%"
    );

    eprintln!("✅ CPU within limit: max={max_cpu:.1}% mean={mean_cpu:.1}%");
}
