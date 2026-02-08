use std::fs;
use std::path::Path;

fn main() {
    // Write capabilities/default.json based on feature flags.
    // When debug-bridge is enabled, include its permission so the
    // eval_callback Tauri command is allowed from JS.
    let caps_dir = Path::new("capabilities");
    fs::create_dir_all(caps_dir).expect("create capabilities dir");

    let permissions = if cfg!(feature = "debug-bridge") {
        r#"{
  "identifier": "default",
  "description": "Default capabilities for AgentKernel Desktop",
  "windows": ["main"],
  "permissions": [
    "core:default",
    "shell:allow-open",
    "debug-bridge:default"
  ]
}"#
    } else {
        r#"{
  "identifier": "default",
  "description": "Default capabilities for AgentKernel Desktop",
  "windows": ["main"],
  "permissions": [
    "core:default",
    "shell:allow-open"
  ]
}"#
    };

    let caps_path = caps_dir.join("default.json");

    // Only write if content changed to avoid triggering Tauri's file watcher loop.
    let needs_write = match fs::read_to_string(&caps_path) {
        Ok(existing) => existing != permissions,
        Err(_) => true,
    };

    if needs_write {
        fs::write(&caps_path, permissions).expect("write capabilities");
    }

    tauri_build::build();
}
