use std::fs;
use std::path::Path;

const LEGACY_IMAGE: &str = concat!("alpine:3", ".20");

fn scan(path: &Path, failures: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let relative = path
            .strip_prefix(env!("CARGO_MANIFEST_DIR"))
            .unwrap_or(&path);
        let relative_text = relative.to_string_lossy();

        if path.is_dir() {
            if matches!(
                relative_text.as_ref(),
                ".git" | ".beads" | "target" | "node_modules" | "sdk/nodejs/dist"
            ) {
                continue;
            }
            scan(&path, failures);
            continue;
        }

        if relative_text == "plan/0001-platform-modernization.md"
            || (relative_text.starts_with("autoresearch/") && relative_text.ends_with(".json"))
        {
            continue;
        }

        let Ok(contents) = fs::read_to_string(&path) else {
            continue;
        };
        let lines: Vec<_> = contents.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            if !line.contains(LEGACY_IMAGE) {
                continue;
            }

            let previous = index
                .checked_sub(1)
                .and_then(|i| lines.get(i))
                .copied()
                .unwrap_or("");
            if line.to_ascii_lowercase().contains("legacy compatibility")
                || previous
                    .to_ascii_lowercase()
                    .contains("legacy compatibility")
            {
                continue;
            }

            failures.push(format!("{}:{}: {}", relative_text, index + 1, line.trim()));
        }
    }
}

#[test]
fn alpine_320_only_appears_in_documented_legacy_compatibility_cases() {
    let mut failures = Vec::new();
    scan(Path::new(env!("CARGO_MANIFEST_DIR")), &mut failures);

    assert!(
        failures.is_empty(),
        "Alpine 3.20 must not be a product default; annotate intentional compatibility coverage:\n{}",
        failures.join("\n")
    );
}
