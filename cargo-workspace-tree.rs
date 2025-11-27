#!/usr/bin/env scriptr
---
[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
---

use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::process::{Command, Stdio};

#[derive(Deserialize)]
struct CargoMetadata {
    workspace_members: Vec<String>,
    resolve: Resolve,
}

#[derive(Deserialize)]
struct Resolve {
    nodes: Vec<Node>,
}

#[derive(Deserialize)]
struct Node {
    id: String,
    deps: Vec<Dep>,
}

#[derive(Deserialize)]
struct Dep {
    name: String,
    pkg: String,
    dep_kinds: Vec<DepKind>,
}

#[derive(Deserialize)]
struct DepKind {
    kind: Option<String>,
}

fn name_from_id(id: &str) -> &str {
    // ID format: path+file:///path/to/pkg#version
    // Extract last path segment before #
    id.rsplit_once('#')
        .and_then(|(path, _)| path.rsplit_once('/'))
        .map(|(_, name)| name)
        .unwrap_or(id)
}

const HELP: &str = "\
cargo-workspace-tree - Show workspace-internal dependencies

Usage: cargo-workspace-tree [OPTIONS]

Like `cargo tree`, but filtered to only show dependencies between workspace
members. For each workspace package, lists which other workspace packages it
depends on (external crates are omitted).

All arguments are passed through to `cargo metadata`.

Common options:
  --manifest-path <PATH>  Path to Cargo.toml
  -p, --package <SPEC>    Package to use as the root
  --frozen                Require Cargo.lock is up to date
  --locked                Require Cargo.lock and cache are up to date
  --offline               Run without network access

Examples:
  cargo-workspace-tree
  cargo-workspace-tree --manifest-path /path/to/Cargo.toml
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{HELP}");
        return;
    }

    let output = Command::new("cargo")
        .arg("metadata")
        .arg("--format-version")
        .arg("1")
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .output()
        .expect("failed to run cargo metadata");

    if !output.status.success() {
        std::process::exit(output.status.code().unwrap_or(1));
    }

    let meta: CargoMetadata =
        serde_json::from_slice(&output.stdout).expect("failed to parse cargo metadata");

    let workspace_ids: HashSet<&str> = meta.workspace_members.iter().map(|s| s.as_str()).collect();

    // Build tree: package name -> sorted list of (dep_name, kind_suffix)
    let mut tree: HashMap<&str, Vec<(&str, &str)>> = HashMap::new();

    for node in &meta.resolve.nodes {
        if !workspace_ids.contains(node.id.as_str()) {
            continue;
        }

        let pkg_name = name_from_id(&node.id);
        let deps: Vec<(&str, &str)> = node
            .deps
            .iter()
            .filter(|d| workspace_ids.contains(d.pkg.as_str()))
            .map(|d| {
                let suffix = if d.dep_kinds.iter().any(|k| k.kind.as_deref() == Some("dev")) {
                    " [dev]"
                } else if d.dep_kinds.iter().any(|k| k.kind.as_deref() == Some("build")) {
                    " [build]"
                } else {
                    ""
                };
                (d.name.as_str(), suffix)
            })
            .collect();

        tree.insert(pkg_name, deps);
    }

    // Sort and print
    let mut names: Vec<&str> = tree.keys().copied().collect();
    names.sort();

    for name in names {
        let deps = tree.get(name).unwrap();
        if deps.is_empty() {
            println!("{name} (no workspace deps)");
        } else {
            println!("{name}");
            let mut sorted_deps = deps.clone();
            sorted_deps.sort_by_key(|(n, _)| *n);
            for (i, (dep, suffix)) in sorted_deps.iter().enumerate() {
                let prefix = if i == sorted_deps.len() - 1 { "└──" } else { "├──" };
                println!("  {prefix} {dep}{suffix}");
            }
        }
    }
}
