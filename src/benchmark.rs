use anyhow::Result;
use std::time::{Duration, Instant};

use crate::backend::{self, BackendType};
use crate::vmm::VmManager;

/// Results from a single benchmark iteration.
struct IterResult {
    create: Duration,
    start: Duration,
    exec: Duration,
    stop: Duration,
    remove: Duration,
}

impl IterResult {
    fn total(&self) -> Duration {
        self.create + self.start + self.exec + self.stop + self.remove
    }
}

/// Collected stats across iterations for one backend.
struct BenchmarkStats {
    backend: BackendType,
    iterations: usize,
    create: Vec<Duration>,
    start: Vec<Duration>,
    exec: Vec<Duration>,
    stop: Vec<Duration>,
    remove: Vec<Duration>,
    total: Vec<Duration>,
}

impl BenchmarkStats {
    fn new(backend: BackendType) -> Self {
        Self {
            backend,
            iterations: 0,
            create: Vec::new(),
            start: Vec::new(),
            exec: Vec::new(),
            stop: Vec::new(),
            remove: Vec::new(),
            total: Vec::new(),
        }
    }

    fn push(&mut self, r: IterResult) {
        self.create.push(r.create);
        self.start.push(r.start);
        self.exec.push(r.exec);
        self.stop.push(r.stop);
        self.remove.push(r.remove);
        self.total.push(r.total());
        self.iterations += 1;
    }
}

fn p50(durations: &[Duration]) -> Duration {
    let mut sorted: Vec<_> = durations.to_vec();
    sorted.sort();
    sorted[sorted.len() / 2]
}

fn p95(durations: &[Duration]) -> Duration {
    let mut sorted: Vec<_> = durations.to_vec();
    sorted.sort();
    let idx = ((sorted.len() as f64) * 0.95).ceil() as usize - 1;
    sorted[idx.min(sorted.len() - 1)]
}

fn fmt_ms(d: Duration) -> String {
    format!("{}ms", d.as_millis())
}

/// Run benchmarks across the specified backends.
pub async fn run_benchmark(backends: &[BackendType], iterations: usize, image: &str) -> Result<()> {
    let test_cmd = vec!["echo".to_string(), "hello".to_string()];
    let show_percentiles = iterations > 1;

    println!(
        "Benchmarking {} backend{} × {} iteration{} (image: {})\n",
        backends.len(),
        if backends.len() != 1 { "s" } else { "" },
        iterations,
        if iterations != 1 { "s" } else { "" },
        image,
    );

    let mut all_stats: Vec<BenchmarkStats> = Vec::new();

    for &bt in backends {
        if !backend::backend_available(bt) {
            println!("{:<15} skipped (not available)", format!("{}", bt));
            continue;
        }

        let mut stats = BenchmarkStats::new(bt);

        for i in 0..iterations {
            let sandbox_name = format!("agentkernel-bench-{}-{}", bt, i);
            let mut manager = VmManager::with_backend(Some(bt))?;

            // Create
            let t = Instant::now();
            manager.create(&sandbox_name, image, 1, 256).await?;
            let create = t.elapsed();

            // Start
            let t = Instant::now();
            manager.start(&sandbox_name).await?;
            let start = t.elapsed();

            // Exec
            let t = Instant::now();
            let _ = manager.exec_cmd(&sandbox_name, &test_cmd).await?;
            let exec = t.elapsed();

            // Stop
            let t = Instant::now();
            manager.stop(&sandbox_name).await?;
            let stop_d = t.elapsed();

            // Remove
            let t = Instant::now();
            manager.remove(&sandbox_name).await?;
            let remove = t.elapsed();

            stats.push(IterResult {
                create,
                start,
                exec,
                stop: stop_d,
                remove,
            });
        }

        all_stats.push(stats);
    }

    // Print results
    if show_percentiles {
        println!(
            "{:<15} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10}",
            "Backend", "Create", "Start", "Exec", "Stop", "Remove", "Total"
        );
        println!("{}", "-".repeat(85));
        for s in &all_stats {
            println!(
                "{:<15} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10}  (p50)",
                format!("{}", s.backend),
                fmt_ms(p50(&s.create)),
                fmt_ms(p50(&s.start)),
                fmt_ms(p50(&s.exec)),
                fmt_ms(p50(&s.stop)),
                fmt_ms(p50(&s.remove)),
                fmt_ms(p50(&s.total)),
            );
            println!(
                "{:<15} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10}  (p95)",
                "",
                fmt_ms(p95(&s.create)),
                fmt_ms(p95(&s.start)),
                fmt_ms(p95(&s.exec)),
                fmt_ms(p95(&s.stop)),
                fmt_ms(p95(&s.remove)),
                fmt_ms(p95(&s.total)),
            );
        }
    } else {
        println!(
            "{:<15} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10}",
            "Backend", "Create", "Start", "Exec", "Stop", "Remove", "Total"
        );
        println!("{}", "-".repeat(85));
        for s in &all_stats {
            println!(
                "{:<15} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10}",
                format!("{}", s.backend),
                fmt_ms(s.create[0]),
                fmt_ms(s.start[0]),
                fmt_ms(s.exec[0]),
                fmt_ms(s.stop[0]),
                fmt_ms(s.remove[0]),
                fmt_ms(s.total[0]),
            );
        }
    }

    println!(
        "\n{} iteration{} complete.",
        iterations,
        if iterations != 1 { "s" } else { "" }
    );

    Ok(())
}

/// Parse a comma-separated list of backend names into BackendTypes.
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
                other => anyhow::bail!("Unknown backend: '{}'", other),
            }
        })
        .collect()
}

/// Return all locally available backends.
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
        // Just ensure it doesn't panic
        let _ = available_backends();
    }

    #[test]
    fn test_fmt_ms() {
        assert_eq!(fmt_ms(Duration::from_millis(42)), "42ms");
        assert_eq!(fmt_ms(Duration::from_millis(1234)), "1234ms");
    }

    #[test]
    fn test_percentiles() {
        let durations: Vec<Duration> = (1..=100).map(|i| Duration::from_millis(i)).collect();
        // p50: index 50 out of 100 = 51ms (1-indexed data)
        assert_eq!(p50(&durations), Duration::from_millis(51));
        assert_eq!(p95(&durations), Duration::from_millis(95));
    }

    #[test]
    fn test_iter_result_total() {
        let r = IterResult {
            create: Duration::from_millis(10),
            start: Duration::from_millis(20),
            exec: Duration::from_millis(30),
            stop: Duration::from_millis(15),
            remove: Duration::from_millis(5),
        };
        assert_eq!(r.total(), Duration::from_millis(80));
    }
}
