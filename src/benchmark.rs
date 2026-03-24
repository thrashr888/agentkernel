use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::Serialize;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use crate::backend::{self, BackendType};
use crate::vmm::VmManager;

#[derive(Debug)]
struct IterResult {
    create: Duration,
    start: Duration,
    exec: Duration,
    stop: Duration,
    remove: Duration,
    cli_total: Duration,
}

impl IterResult {
    fn startup(&self) -> Duration {
        self.create + self.start
    }

    fn lifecycle_total(&self) -> Duration {
        self.create + self.start + self.exec + self.stop + self.remove
    }
}

#[derive(Debug)]
struct BenchmarkStats {
    backend: BackendType,
    measured_iterations: usize,
    warmup_iterations: usize,
    create: Vec<Duration>,
    start: Vec<Duration>,
    exec: Vec<Duration>,
    stop: Vec<Duration>,
    remove: Vec<Duration>,
    startup: Vec<Duration>,
    lifecycle_total: Vec<Duration>,
    cli_total: Vec<Duration>,
}

impl BenchmarkStats {
    fn new(backend: BackendType, warmup_iterations: usize) -> Self {
        Self {
            backend,
            measured_iterations: 0,
            warmup_iterations,
            create: Vec::new(),
            start: Vec::new(),
            exec: Vec::new(),
            stop: Vec::new(),
            remove: Vec::new(),
            startup: Vec::new(),
            lifecycle_total: Vec::new(),
            cli_total: Vec::new(),
        }
    }

    fn push(&mut self, r: IterResult) {
        self.create.push(r.create);
        self.start.push(r.start);
        self.exec.push(r.exec);
        self.stop.push(r.stop);
        self.remove.push(r.remove);
        self.startup.push(r.startup());
        self.lifecycle_total.push(r.lifecycle_total());
        self.cli_total.push(r.cli_total);
        self.measured_iterations += 1;
    }

    fn total_wall_time(&self) -> Duration {
        self.cli_total.iter().copied().sum()
    }

    fn to_report(&self) -> BackendBenchmarkReport {
        let startup = summarize(&self.startup);
        let exec = summarize(&self.exec);
        let lifecycle_total = summarize(&self.lifecycle_total);
        let total = summarize(&self.cli_total);
        let throughput_per_second = if self.total_wall_time().is_zero() {
            0.0
        } else {
            self.measured_iterations as f64 / self.total_wall_time().as_secs_f64()
        };

        let startup_score = inverse_ms_score(startup.avg_ms);
        let exec_score = inverse_ms_score(exec.avg_ms);
        let latency_score = inverse_ms_score(total.avg_ms);
        let throughput_score = throughput_per_second;
        let total_score = startup_score * 0.30
            + exec_score * 0.05
            + latency_score * 0.50
            + throughput_score * 0.15;

        BackendBenchmarkReport {
            backend: self.backend.to_string(),
            measured_iterations: self.measured_iterations,
            warmup_iterations: self.warmup_iterations,
            startup,
            exec,
            lifecycle_total,
            total,
            throughput_per_second,
            scores: ScoreBreakdown {
                startup_score,
                exec_score,
                latency_score,
                throughput_score,
                total_score,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MetricSummary {
    pub min_ms: f64,
    pub avg_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ScoreBreakdown {
    pub startup_score: f64,
    pub exec_score: f64,
    pub latency_score: f64,
    pub throughput_score: f64,
    pub total_score: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BackendBenchmarkReport {
    pub backend: String,
    pub measured_iterations: usize,
    pub warmup_iterations: usize,
    pub startup: MetricSummary,
    pub exec: MetricSummary,
    pub lifecycle_total: MetricSummary,
    pub total: MetricSummary,
    pub throughput_per_second: f64,
    pub scores: ScoreBreakdown,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BenchmarkReport {
    pub generated_at: String,
    pub image: String,
    pub measured_iterations: usize,
    pub warmup_iterations: usize,
    pub primary_metric: String,
    pub total_score: f64,
    pub backends: Vec<BackendBenchmarkReport>,
}

fn duration_ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn avg_ms(durations: &[Duration]) -> f64 {
    durations.iter().map(|d| duration_ms(*d)).sum::<f64>() / durations.len() as f64
}

fn percentile(durations: &[Duration], percentile: f64) -> Duration {
    let mut sorted: Vec<_> = durations.to_vec();
    sorted.sort();
    let idx = ((sorted.len() as f64) * percentile).ceil() as usize - 1;
    sorted[idx.min(sorted.len() - 1)]
}

fn p50(durations: &[Duration]) -> Duration {
    percentile(durations, 0.50)
}

fn p95(durations: &[Duration]) -> Duration {
    percentile(durations, 0.95)
}

fn summarize(durations: &[Duration]) -> MetricSummary {
    let mut sorted: Vec<_> = durations.to_vec();
    sorted.sort();
    MetricSummary {
        min_ms: duration_ms(sorted[0]),
        avg_ms: avg_ms(durations),
        p50_ms: duration_ms(p50(durations)),
        p95_ms: duration_ms(p95(durations)),
        max_ms: duration_ms(*sorted.last().unwrap()),
    }
}

fn inverse_ms_score(ms: f64) -> f64 {
    1000.0 / ms.max(1.0)
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn round_summary(summary: &mut MetricSummary) {
    summary.min_ms = round2(summary.min_ms);
    summary.avg_ms = round2(summary.avg_ms);
    summary.p50_ms = round2(summary.p50_ms);
    summary.p95_ms = round2(summary.p95_ms);
    summary.max_ms = round2(summary.max_ms);
}

fn round_scores(scores: &mut ScoreBreakdown) {
    scores.startup_score = round2(scores.startup_score);
    scores.exec_score = round2(scores.exec_score);
    scores.latency_score = round2(scores.latency_score);
    scores.throughput_score = round2(scores.throughput_score);
    scores.total_score = round2(scores.total_score);
}

fn round_backend_report(report: &mut BackendBenchmarkReport) {
    round_summary(&mut report.startup);
    round_summary(&mut report.exec);
    round_summary(&mut report.lifecycle_total);
    round_summary(&mut report.total);
    report.throughput_per_second = round2(report.throughput_per_second);
    round_scores(&mut report.scores);
}

fn round_report(report: &mut BenchmarkReport) {
    report.total_score = round2(report.total_score);
    for backend in &mut report.backends {
        round_backend_report(backend);
    }
}

fn run_cli_iteration(backend: BackendType, image: &str) -> Result<Duration> {
    let backend_name = backend.to_string();
    let exe = std::env::current_exe().context("failed to locate current agentkernel binary")?;
    let temp_dir =
        tempfile::tempdir().context("failed to create temp dir for end-to-end benchmark")?;
    let started = Instant::now();
    let output = Command::new(exe)
        .current_dir(temp_dir.path())
        .args([
            "run",
            "-B",
            &backend_name,
            "--image",
            image,
            "echo",
            "hello",
        ])
        .output()
        .context("failed to spawn end-to-end benchmark command")?;
    let elapsed = started.elapsed();

    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "end-to-end benchmark command failed for backend {}: {}{}",
            backend_name,
            stdout,
            stderr
        );
    }

    Ok(elapsed)
}

async fn run_single_iteration(
    backend: BackendType,
    image: &str,
    iteration_label: &str,
) -> Result<IterResult> {
    let test_cmd = vec!["echo".to_string(), "hello".to_string()];
    let sandbox_name = format!("agentkernel-bench-{}-{}", backend, iteration_label);
    let mut manager = VmManager::with_backend(Some(backend))?;

    let t = Instant::now();
    manager.create(&sandbox_name, image, 1, 256).await?;
    let create = t.elapsed();

    let t = Instant::now();
    manager.start(&sandbox_name).await?;
    let start = t.elapsed();

    let t = Instant::now();
    let _ = manager.exec_cmd(&sandbox_name, &test_cmd).await?;
    let exec = t.elapsed();

    let t = Instant::now();
    manager.stop(&sandbox_name).await?;
    let stop = t.elapsed();

    let t = Instant::now();
    manager.remove(&sandbox_name).await?;
    let remove = t.elapsed();

    let cli_total = run_cli_iteration(backend, image)?;

    Ok(IterResult {
        create,
        start,
        exec,
        stop,
        remove,
        cli_total,
    })
}

pub async fn run_benchmark(
    backends: &[BackendType],
    iterations: usize,
    warmup: usize,
    image: &str,
    render_progress: bool,
) -> Result<BenchmarkReport> {
    if iterations == 0 {
        bail!("iterations must be at least 1");
    }

    if render_progress {
        println!(
            "Benchmarking {} backend{} × {} measured iteration{} (+ {} warmup) with image {}\n",
            backends.len(),
            if backends.len() != 1 { "s" } else { "" },
            iterations,
            if iterations != 1 { "s" } else { "" },
            warmup,
            image,
        );
    }

    let mut reports = Vec::new();

    for &backend_type in backends {
        if !backend::backend_available(backend_type) {
            if render_progress {
                println!("{:<15} skipped (not available)", backend_type);
            }
            continue;
        }

        if render_progress {
            println!("Benchmarking {backend_type}...");
        }

        let mut stats = BenchmarkStats::new(backend_type, warmup);

        for warmup_index in 0..warmup {
            let label = format!("warmup-{warmup_index}");
            let _ = run_single_iteration(backend_type, image, &label).await?;
        }

        for iteration_index in 0..iterations {
            let result =
                run_single_iteration(backend_type, image, &iteration_index.to_string()).await?;
            stats.push(result);
        }

        reports.push(stats.to_report());
    }

    if reports.is_empty() {
        bail!("No requested backends were available to benchmark");
    }

    let total_score = reports
        .iter()
        .map(|report| report.scores.total_score)
        .sum::<f64>()
        / reports.len() as f64;

    let mut report = BenchmarkReport {
        generated_at: Utc::now().to_rfc3339(),
        image: image.to_string(),
        measured_iterations: iterations,
        warmup_iterations: warmup,
        primary_metric: "total_score".to_string(),
        total_score,
        backends: reports,
    };
    round_report(&mut report);

    Ok(report)
}

fn fmt_ms(ms: f64) -> String {
    format!("{ms:.2}ms")
}

pub fn emit_report(report: &BenchmarkReport, json: bool, output: Option<&Path>) -> Result<()> {
    let json_body = serde_json::to_string_pretty(report)?;

    if let Some(path) = output {
        fs::write(path, &json_body)
            .with_context(|| format!("failed to write benchmark report to {}", path.display()))?;
    }

    if json {
        println!("{json_body}");
        return Ok(());
    }

    println!(
        "{:<15} {:>12} {:>12} {:>12} {:>12} {:>10}",
        "Backend", "Startup", "Exec", "Total", "Throughput", "Score"
    );
    println!("{}", "-".repeat(80));
    for backend in &report.backends {
        println!(
            "{:<15} {:>12} {:>12} {:>12} {:>12.2}/s {:>10.2}",
            backend.backend,
            fmt_ms(backend.startup.avg_ms),
            fmt_ms(backend.exec.avg_ms),
            fmt_ms(backend.total.avg_ms),
            backend.throughput_per_second,
            backend.scores.total_score,
        );
        if backend.measured_iterations > 1 {
            println!(
                "{:<15} {:>12} {:>12} {:>12} {:>12} {:>10}",
                "",
                format!("p95 {}", fmt_ms(backend.startup.p95_ms)),
                format!("p95 {}", fmt_ms(backend.exec.p95_ms)),
                format!("p95 {}", fmt_ms(backend.total.p95_ms)),
                "",
                "",
            );
        }
    }

    println!(
        "\nPrimary metric: {} = {:.2}",
        report.primary_metric, report.total_score
    );

    if let Some(path) = output {
        println!("Saved JSON report to {}", path.display());
    }

    Ok(())
}

pub fn parse_backends(input: &str) -> Result<Vec<BackendType>> {
    input
        .split(',')
        .map(|s| {
            let s = s.trim().to_lowercase();
            match s.as_str() {
                "docker" => Ok(BackendType::Docker),
                "podman" => Ok(BackendType::Podman),
                "firecracker" => Ok(BackendType::Firecracker),
                "apple" => Ok(BackendType::Apple),
                "hyperlight" => Ok(BackendType::Hyperlight),
                "kubernetes" | "k8s" => Ok(BackendType::Kubernetes),
                "nomad" => Ok(BackendType::Nomad),
                other => bail!("Unknown backend: '{}'", other),
            }
        })
        .collect()
}

pub fn available_backends() -> Vec<BackendType> {
    let all = [
        BackendType::Docker,
        BackendType::Podman,
        BackendType::Firecracker,
        BackendType::Apple,
        BackendType::Hyperlight,
    ];
    all.into_iter()
        .filter(|b| backend::backend_available(*b))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_backends() {
        let bs = parse_backends("docker,podman").unwrap();
        assert_eq!(bs, vec![BackendType::Docker, BackendType::Podman]);
    }

    #[test]
    fn test_parse_backends_with_spaces() {
        let bs = parse_backends("docker , apple").unwrap();
        assert_eq!(bs, vec![BackendType::Docker, BackendType::Apple]);
    }

    #[test]
    fn test_parse_backends_unknown() {
        assert!(parse_backends("docker,wat").is_err());
    }

    #[test]
    fn test_available_backends() {
        let _ = available_backends();
    }

    #[test]
    fn test_percentiles() {
        let durations: Vec<Duration> = (1..=100).map(Duration::from_millis).collect();
        assert_eq!(p50(&durations), Duration::from_millis(50));
        assert_eq!(p95(&durations), Duration::from_millis(95));
    }

    #[test]
    fn test_iter_result_total_and_startup() {
        let r = IterResult {
            create: Duration::from_millis(10),
            start: Duration::from_millis(20),
            exec: Duration::from_millis(30),
            stop: Duration::from_millis(15),
            remove: Duration::from_millis(5),
            cli_total: Duration::from_millis(100),
        };
        assert_eq!(r.startup(), Duration::from_millis(30));
        assert_eq!(r.lifecycle_total(), Duration::from_millis(80));
    }

    #[test]
    fn test_summarize() {
        let durations = vec![
            Duration::from_millis(10),
            Duration::from_millis(20),
            Duration::from_millis(30),
        ];
        let summary = summarize(&durations);
        assert_eq!(summary.min_ms, 10.0);
        assert_eq!(summary.avg_ms, 20.0);
        assert_eq!(summary.p50_ms, 20.0);
        assert_eq!(summary.p95_ms, 30.0);
        assert_eq!(summary.max_ms, 30.0);
    }

    #[test]
    fn test_emit_report_writes_json_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("report.json");
        let report = BenchmarkReport {
            generated_at: "2026-03-23T00:00:00Z".to_string(),
            image: "alpine:3.20".to_string(),
            measured_iterations: 3,
            warmup_iterations: 1,
            primary_metric: "total_score".to_string(),
            total_score: 12.34,
            backends: vec![BackendBenchmarkReport {
                backend: "docker".to_string(),
                measured_iterations: 3,
                warmup_iterations: 1,
                startup: MetricSummary {
                    min_ms: 100.0,
                    avg_ms: 110.0,
                    p50_ms: 105.0,
                    p95_ms: 120.0,
                    max_ms: 120.0,
                },
                exec: MetricSummary {
                    min_ms: 10.0,
                    avg_ms: 12.0,
                    p50_ms: 11.0,
                    p95_ms: 15.0,
                    max_ms: 15.0,
                },
                lifecycle_total: MetricSummary {
                    min_ms: 120.0,
                    avg_ms: 130.0,
                    p50_ms: 128.0,
                    p95_ms: 140.0,
                    max_ms: 140.0,
                },
                total: MetricSummary {
                    min_ms: 180.0,
                    avg_ms: 190.0,
                    p50_ms: 188.0,
                    p95_ms: 200.0,
                    max_ms: 200.0,
                },
                throughput_per_second: 5.26,
                scores: ScoreBreakdown {
                    startup_score: 9.09,
                    exec_score: 83.33,
                    latency_score: 5.26,
                    throughput_score: 5.26,
                    total_score: 12.34,
                },
            }],
        };

        emit_report(&report, false, Some(&path)).unwrap();
        let written = fs::read_to_string(path).unwrap();
        assert!(written.contains("\"total_score\": 12.34"));
        assert!(written.contains("\"backend\": \"docker\""));
        assert!(written.contains("\"lifecycle_total\""));
    }
}
