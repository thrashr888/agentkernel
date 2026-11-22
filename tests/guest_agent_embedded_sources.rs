use std::path::Path;

#[test]
fn embedded_guest_agent_sources_match_canonical_sources() {
    let project_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for (canonical, embedded) in [
        (
            "guest-agent/src/main.rs",
            "src/embedded/guest_agent_main.rs",
        ),
        ("guest-agent/src/pty.rs", "src/embedded/guest_agent_pty.rs"),
    ] {
        let canonical_source =
            std::fs::read(project_root.join(canonical)).expect("read canonical guest-agent source");
        let embedded_source =
            std::fs::read(project_root.join(embedded)).expect("read embedded guest-agent source");
        assert_eq!(
            embedded_source, canonical_source,
            "embedded guest-agent source is out of sync: {embedded}"
        );
    }
}

#[test]
fn embedded_guest_manifest_differs_only_in_gate_source_path() {
    let canonical = include_str!("../guest-agent/Cargo.toml");
    let embedded = include_str!("../src/embedded/guest_agent_cargo.toml");
    // The repository builds the canonical gate outside the nested crate so
    // Cargo can package it. Standalone guest builds use the materialized copy.
    let canonical_path = r#"path = "../src/embedded/docker_rex_gate/main.rs""#;
    let embedded_path = r#"path = "src/docker_rex_gate/main.rs""#;
    assert_eq!(canonical.matches(canonical_path).count(), 1);
    assert_eq!(embedded, canonical.replace(canonical_path, embedded_path));
}
