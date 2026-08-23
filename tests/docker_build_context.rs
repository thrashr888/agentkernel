use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn docker_context_contains_all_root_include_str_files() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let includes = include_str_files(&root.join("src"));
    let copy_sources = docker_copy_sources(&root.join("Dockerfile"));
    let allow_rules = dockerignore_allow_rules(&root.join(".dockerignore"));
    let package_include_patterns = cargo_package_include_patterns(&root.join("Cargo.toml"));

    assert!(!includes.is_empty(), "expected to find include_str! files");

    for include in includes {
        let relative = include
            .strip_prefix(root)
            .expect("include_str! path must be inside the repository")
            .to_path_buf();

        assert!(
            copy_sources
                .iter()
                .any(|source| copy_source_covers(root, source, &include)),
            "Dockerfile does not copy include_str! file {}",
            relative.display()
        );
        assert_dockerignore_allows(&allow_rules, &relative);
        assert!(
            package_include_patterns
                .iter()
                .any(|pattern| cargo_pattern_covers(pattern, &relative)),
            "Cargo.toml package.include does not include {}",
            relative.display()
        );
    }
}

fn include_str_files(src: &Path) -> BTreeSet<PathBuf> {
    let mut files = BTreeSet::new();
    collect_include_str_files(src, &mut files);
    files
}

fn collect_include_str_files(path: &Path, files: &mut BTreeSet<PathBuf>) {
    let metadata = fs::metadata(path)
        .unwrap_or_else(|error| panic!("cannot inspect {}: {error}", path.display()));

    if metadata.is_dir() {
        for entry in fs::read_dir(path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
        {
            collect_include_str_files(
                &entry
                    .unwrap_or_else(|error| panic!("cannot read directory entry: {error}"))
                    .path(),
                files,
            );
        }
        return;
    }

    if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
        return;
    }

    let contents = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    let mut remaining = contents.as_str();
    while let Some(start) = remaining.find("include_str!(") {
        remaining = &remaining[start + "include_str!(".len()..];
        let Some(open_quote) = remaining.find('"') else {
            break;
        };
        remaining = &remaining[open_quote + 1..];
        let Some(close_quote) = remaining.find('"') else {
            break;
        };
        let include_path = &remaining[..close_quote];
        let resolved = path
            .parent()
            .expect("Rust source file must have a parent")
            .join(include_path)
            .canonicalize()
            .unwrap_or_else(|error| {
                panic!(
                    "include_str! target {} from {} is missing: {error}",
                    include_path,
                    path.display()
                )
            });
        files.insert(resolved);
        remaining = &remaining[close_quote + 1..];
    }
}

fn docker_copy_sources(dockerfile: &Path) -> Vec<PathBuf> {
    let contents = fs::read_to_string(dockerfile)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", dockerfile.display()));

    contents
        .lines()
        .filter_map(|line| {
            let line = line.split('#').next()?.trim();
            let mut parts = line.split_whitespace();
            if parts.next()? != "COPY" {
                return None;
            }
            let args: Vec<_> = parts.collect();
            if args.is_empty() || args.iter().any(|arg| arg.starts_with("--from=")) {
                return None;
            }
            Some(
                args[..args.len() - 1]
                    .iter()
                    .map(|source| PathBuf::from(*source))
                    .collect::<Vec<_>>(),
            )
        })
        .flatten()
        .collect()
}

fn copy_source_covers(root: &Path, source: &Path, include: &Path) -> bool {
    let source = root.join(source);
    include == source || (source.is_dir() && include.starts_with(source))
}

fn dockerignore_allow_rules(dockerignore: &Path) -> Vec<String> {
    let contents = fs::read_to_string(dockerignore)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", dockerignore.display()));

    contents
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with('!'))
        .map(|line| line[1..].trim_end_matches('/').to_string())
        .collect()
}

fn cargo_package_include_patterns(manifest: &Path) -> Vec<String> {
    let contents = fs::read_to_string(manifest)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", manifest.display()));
    let value: toml::Value = toml::from_str(&contents)
        .unwrap_or_else(|error| panic!("cannot parse {}: {error}", manifest.display()));

    value["package"]["include"]
        .as_array()
        .expect("Cargo.toml package.include must be an array")
        .iter()
        .map(|pattern| {
            pattern
                .as_str()
                .expect("Cargo.toml package.include entries must be strings")
                .trim_start_matches('/')
                .to_string()
        })
        .collect()
}

fn cargo_pattern_covers(pattern: &str, path: &Path) -> bool {
    let path = path.to_string_lossy();
    if let Some(prefix) = pattern.strip_suffix("/**/*") {
        path.starts_with(&format!("{prefix}/"))
    } else if let Some(prefix) = pattern.strip_suffix("/**") {
        path.starts_with(&format!("{prefix}/"))
    } else if let Some((prefix, suffix)) = pattern.split_once('*') {
        path.strip_prefix(prefix)
            .and_then(|remaining| remaining.strip_suffix(suffix))
            .is_some_and(|middle| !middle.contains('/'))
    } else {
        pattern == path
    }
}

fn assert_dockerignore_allows(rules: &[String], relative: &Path) {
    let components: Vec<_> = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect();

    for length in 1..=components.len() {
        let prefix = components[..length].join("/");
        assert!(
            rules.iter().any(|rule| rule_covers(rule, &prefix)),
            ".dockerignore does not allow {}",
            prefix
        );
    }
}

fn rule_covers(rule: &str, path: &str) -> bool {
    if let Some(prefix) = rule.strip_suffix("/**") {
        path == prefix || path.starts_with(&format!("{prefix}/"))
    } else {
        rule == path
    }
}
