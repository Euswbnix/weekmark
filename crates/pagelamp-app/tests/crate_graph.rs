//! Crate-graph rules (docs/design/v0.3-model-access.md §0.2, §10 rule 1):
//! - `pagelamp-mcp` and `pagelamp-core` never depend on `pagelamp-llm`, `pagelamp-app` or
//!   `pagelamp-canvas`, so the MCP code path can't reach a model, the network or Canvas
//!   (docs/ARCHITECTURE.md §3 rule 1);
//! - `pagelamp-core`'s `test-support` feature (a way to build a `RenderedPrompt` without the
//!   policy gate) is enabled only by dev-dependencies.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

/// `cargo metadata` of the workspace (offline: the lockfile is enough).
fn metadata() -> Value {
    let output = std::process::Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--offline",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo metadata runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

/// Workspace package → its normal (non-dev, non-build) dependencies: (name, features).
fn normal_dependencies(metadata: &Value) -> BTreeMap<String, Vec<(String, Vec<String>)>> {
    metadata["packages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|package| {
            let dependencies = package["dependencies"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|dependency| dependency["kind"].is_null())
                .map(|dependency| {
                    let features = dependency["features"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|feature| feature.as_str().unwrap().to_string())
                        .collect();
                    (dependency["name"].as_str().unwrap().to_string(), features)
                })
                .collect();
            (package["name"].as_str().unwrap().to_string(), dependencies)
        })
        .collect()
}

/// Every workspace crate `root` depends on through normal dependencies, `root` included.
fn closure(graph: &BTreeMap<String, Vec<(String, Vec<String>)>>, root: &str) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut todo = vec![root.to_string()];
    while let Some(name) = todo.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        if let Some(dependencies) = graph.get(&name) {
            todo.extend(
                dependencies
                    .iter()
                    .filter(|(dependency, _)| graph.contains_key(dependency))
                    .map(|(dependency, _)| dependency.clone()),
            );
        }
    }
    seen
}

#[test]
fn the_mcp_server_and_core_never_depend_on_the_model_layer() {
    let graph = normal_dependencies(&metadata());
    assert!(
        graph.contains_key("pagelamp-llm"),
        "the test sees the workspace"
    );
    for root in ["pagelamp-mcp", "pagelamp-core", "pagelamp-extract"] {
        let reached = closure(&graph, root);
        for forbidden in ["pagelamp-llm", "pagelamp-app", "pagelamp-canvas"] {
            assert!(
                !reached.contains(forbidden),
                "{root} depends on {forbidden}: {reached:?}"
            );
        }
    }
}

#[test]
fn only_dev_dependencies_enable_the_test_support_feature() {
    let graph = normal_dependencies(&metadata());
    for (package, dependencies) in &graph {
        for (dependency, features) in dependencies {
            assert!(
                !(dependency == "pagelamp-core" && features.iter().any(|f| f == "test-support")),
                "{package} enables pagelamp-core/test-support outside dev-dependencies"
            );
        }
    }
}
