#!/usr/bin/env scriptr
---
[dependencies]
anyhow = "1"
clap = { version = "4.5", features = ["derive"] }
---

use anyhow::{bail, Context, Result};
use clap::Parser;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Parser)]
#[command(
    about = "List working-copy Rust files whose changes are rustfmt-only",
    long_about = "check-rustfmt compares rustfmt outputs between a jj base revset and its target to flag formatting-only Rust changes.\n\nIt works in three stages:\n  - enumerate changed files with `jj diff --from <base> --to <target> --name-only`\n  - fetch the historical contents via `jj file show -r <base> -- <path>`\n  - run rustfmt on both versions and print the paths whose formatted bytes match\n\nUse `--config` to point at a specific rustfmt configuration, `--base` to pick an alternate baseline revset (default: `parents(@)`), and `--target` if you want to compare two non-working-copy revisions. Pass `--revert` to immediately restore formatting-only files in the target to match the base revision."
)]
struct Args {
    /// Optional rustfmt config path
    #[arg(long = "config", value_name = "PATH")]
    config: Option<PathBuf>,

    /// Base revset for comparison (defaults to parents of working copy)
    #[arg(long, value_name = "REVSET", default_value = "parents(@)")]
    base: String,

    /// Target revset to diff against base (defaults to working copy)
    #[arg(long, value_name = "REV", default_value = "@")]
    target: String,

    /// Restore formatting-only files to match the base revision
    #[arg(long)]
    revert: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    if let Some(config) = &args.config {
        if !config.exists() {
            bail!("config path {:?} does not exist", config);
        }
    }

    let changed = changed_files(&args.base, &args.target)?;
    let mut fmt_only = Vec::new();

    for path in changed {
        if !path.ends_with(".rs") {
            continue;
        }

        // Skip files that are deleted or otherwise missing locally.
        if !Path::new(&path).is_file() {
            continue;
        }

        match is_formatting_only(&path, &args.base, args.config.as_deref())? {
            Some(true) => fmt_only.push(path),
            Some(false) => {}
            None => {}
        }
    }

    fmt_only.sort();

    if args.revert {
        restore_files(&fmt_only, &args.base, &args.target)?;
    }

    for path in fmt_only {
        println!("{}", path);
    }

    Ok(())
}

fn changed_files(base: &str, target: &str) -> Result<Vec<String>> {
    let output = Command::new("jj")
        .env("JJ_NO_PAGER", "1")
        .arg("diff")
        .arg("--from")
        .arg(base)
        .arg("--to")
        .arg(target)
        .arg("--name-only")
        .arg("--quiet")
        .output()
        .context("failed to run jj diff")?;

    if !output.status.success() {
        bail!(
            "jj diff exited with status {}",
            output.status.code().unwrap_or(-1)
        );
    }

    let stdout = String::from_utf8(output.stdout).context("jj diff output was not UTF-8")?;
    Ok(stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.to_string())
        .collect())
}

fn is_formatting_only(path: &str, base: &str, config: Option<&Path>) -> Result<Option<bool>> {
    let Some(base_contents) = base_contents(base, path)? else {
        return Ok(None);
    };
    let work_contents = fs::read(path).with_context(|| format!("failed to read {}", path))?;

    let base_fmt = run_rustfmt(&base_contents, path, config)?;
    let work_fmt = run_rustfmt(&work_contents, path, config)?;

    Ok(Some(base_fmt == work_fmt))
}

fn base_contents(base: &str, path: &str) -> Result<Option<Vec<u8>>> {
    let output = Command::new("jj")
        .env("JJ_NO_PAGER", "1")
        .arg("file")
        .arg("show")
        .arg("-r")
        .arg(base)
        .arg("--no-pager")
        .arg("--")
        .arg(path)
        .output()
        .with_context(|| format!("failed to fetch {path} from {base}"))?;

    if output.status.success() {
        return Ok(Some(output.stdout));
    }

    // Exit code 1 indicates the file does not exist at the base revision.
    if output.status.code() == Some(1) {
        return Ok(None);
    }

    bail!(
        "jj file show exited with status {} for {}",
        output.status.code().unwrap_or(-1),
        path
    );
}

fn run_rustfmt(input: &[u8], path: &str, config: Option<&Path>) -> Result<Vec<u8>> {
    let mut cmd = Command::new("rustfmt");
    cmd.arg("--edition")
        .arg("2021")
        .arg("--emit")
        .arg("stdout")
        .arg("--quiet")
        .arg("--color")
        .arg("never");

    if let Some(cfg) = config {
        cmd.arg("--config-path").arg(cfg);
    }

    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("failed to spawn rustfmt")?;

    {
        let stdin = child.stdin.as_mut().context("failed to open rustfmt stdin")?;
        stdin
            .write_all(input)
            .context("failed to write source to rustfmt")?;
    }

    let output = child
        .wait_with_output()
        .context("failed to read rustfmt output")?;

    if !output.status.success() {
        bail!(
            "rustfmt exited with status {} for {}",
            output.status.code().unwrap_or(-1),
            path
        );
    }

    Ok(output.stdout)
}

fn restore_files(paths: &[String], base: &str, target: &str) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }

    let mut cmd = Command::new("jj");
    cmd.env("JJ_NO_PAGER", "1")
        .arg("restore")
        .arg("--from")
        .arg(base);

    if target != "@" {
        cmd.arg("--into").arg(target);
    }

    cmd.arg("--");
    for path in paths {
        cmd.arg(path);
    }

    let status = cmd.status().context("failed to run jj restore")?;

    if !status.success() {
        bail!(
            "jj restore exited with status {}",
            status.code().unwrap_or(-1)
        );
    }

    Ok(())
}
