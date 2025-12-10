#!/usr/bin/env scriptr
---
[dependencies]
which = "7"
dirs = "5"
---

use std::env;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    // Find the real bd binary, skipping our wrapper symlink at ~/bin/bd
    let my_symlink = dirs::home_dir()
        .map(|h| h.join("bin/bd"))
        .unwrap_or_default();

    let bd_path = which::which_all("bd")
        .ok()
        .and_then(|mut paths| paths.find(|p| *p != my_symlink))
        .unwrap_or_else(|| {
            eprintln!("bd: could not find real bd binary in PATH");
            std::process::exit(1);
        });

    let mut cmd = Command::new(&bd_path);

    // Check if we're in a pervasive subdir that should use the shared db
    if should_use_pervasive_db() {
        cmd.arg("--db");
        cmd.arg(expand_home("~/repos/pervasive/pervasive/.beads/beads.db"));
    }

    cmd.args(&args);

    // Replace current process with bd
    let err = cmd.exec();
    eprintln!("Failed to exec bd: {}", err);
    std::process::exit(1);
}

fn should_use_pervasive_db() -> bool {
    let Ok(cwd) = env::current_dir() else {
        return false;
    };

    let pervasive_base = expand_home("~/repos/pervasive");
    let pervasive_path = Path::new(&pervasive_base);

    // Check if we're under ~/repos/pervasive
    if !cwd.starts_with(pervasive_path) {
        return false;
    }

    // Get the immediate subdirectory name under ~/repos/pervasive
    let relative = cwd.strip_prefix(pervasive_path).unwrap();
    let first_component = relative.components().next();

    if let Some(comp) = first_component {
        let name = comp.as_os_str().to_string_lossy();
        // Match 'pervasive' or anything starting with 'pv-'
        return name == "pervasive" || name.starts_with("pv-");
    }

    false
}

fn expand_home(path: &str) -> String {
    if path.starts_with("~/") {
        if let Ok(home) = env::var("HOME") {
            return format!("{}{}", home, &path[1..]);
        }
    }
    path.to_string()
}
