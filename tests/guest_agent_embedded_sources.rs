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
        (
            "guest-agent/Cargo.toml",
            "src/embedded/guest_agent_cargo.toml",
        ),
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
