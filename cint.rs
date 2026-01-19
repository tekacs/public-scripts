#!/usr/bin/env scriptr
---
[dependencies]
---

use std::env;
use std::process::{Command, exit};

fn main() {
    let args: Vec<String> = env::args().collect();
    let path = args.get(1).map(|s| s.as_str()).unwrap_or(".");

    let status = Command::new("cargo")
        .args(["install", "--locked", "--path", path])
        .status()
        .expect("failed to execute cargo");

    exit(status.code().unwrap_or(1));
}
