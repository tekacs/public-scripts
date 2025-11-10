#!/usr/bin/env scriptr
---
[dependencies]
anyhow = "1.0"
regex = "1.12.2"
---

// Resumes the most recent "resume <uuid>" command visible in the active tmux pane, useful for restarting a just-exited process without retyping it.
use anyhow::{bail, Context, Result};
use regex::Regex;
use std::process::Command;

fn main() -> Result<()> {
    let pane = Command::new("tmux")
        .args(["capture-pane", "-J", "-p", "-S", "-"])
        .output()
        .context("failed to run `tmux capture-pane`")?;

    if !pane.status.success() {
        bail!("tmux capture-pane failed: status={}", pane.status);
    }

    let text = String::from_utf8(pane.stdout).context("tmux output was not valid UTF-8")?;
    let re = Regex::new(r"\bcodex resume\s+([0-9a-fA-F-]{4,})\b")?;

    let command = re
        .captures_iter(&text)
        .filter_map(|caps| caps.get(0).map(|m| m.as_str().trim().to_owned()))
        .last()
        .context("no `codex resume <uuid>` line found in the tmux pane")?;

    println!("running: {command}");

    let status = Command::new("sh")
        .arg("-c")
        .arg(&command)
        .status()
        .context("failed to spawn command shell")?;

    if !status.success() {
        bail!("command `{command}` exited with {status}");
    }

    Ok(())
}
