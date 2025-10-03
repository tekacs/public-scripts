#!/usr/bin/env scriptr
---
[dependencies]
anyhow = "1"
clap = { version = "4.5", features = ["derive"] }
---

use anyhow::{bail, Context, Result};
use clap::Parser;
use std::collections::VecDeque;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

#[derive(Debug)]
struct RustfmtFailure {
    status: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

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
    let base_for_threads = Arc::<str>::from(args.base.clone());
    let config_for_threads = args
        .config
        .as_ref()
        .map(|cfg| Arc::new(cfg.clone()));

    let worker_count = {
        let available = thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        std::cmp::max(1, std::cmp::min(available, changed.len()))
    };

    let work_queue = Arc::new(Mutex::new(VecDeque::from(changed)));
    let results = Arc::new(Mutex::new(Vec::new()));

    thread::scope(|scope| {
        for _ in 0..worker_count {
            let queue = Arc::clone(&work_queue);
            let results = Arc::clone(&results);
            let base = Arc::clone(&base_for_threads);
            let config = config_for_threads.clone();

            scope.spawn(move || loop {
                let path = {
                    let mut queue = queue.lock().expect("work queue poisoned");
                    queue.pop_front()
                };

                let Some(path) = path else {
                    break;
                };

                if !path.ends_with(".rs") {
                    continue;
                }

                if !Path::new(&path).is_file() {
                    continue;
                }

                let config_path = config
                    .as_ref()
                    .map(|cfg| cfg.as_ref().as_path());

                match is_formatting_only(&path, base.as_ref(), config_path) {
                    Ok(Some(true)) => {
                        results
                            .lock()
                            .expect("result queue poisoned")
                            .push(path);
                    }
                    Ok(_) => {}
                    Err(err) => eprintln!("{err}"),
                }
            });
        }
    });

    let mut fmt_only = results
        .lock()
        .expect("result queue poisoned");
    fmt_only.sort();

    if args.revert {
        restore_files(&fmt_only, &args.base, &args.target)?;
    }

    for path in fmt_only.iter() {
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

    let base_fmt = match run_rustfmt(&base_contents, path, "base version", config) {
        Ok(fmt) => fmt,
        Err(err) => {
            eprintln!("{err}");
            return Ok(None);
        }
    };

    let work_fmt = match run_rustfmt(&work_contents, path, "working tree copy", config) {
        Ok(fmt) => fmt,
        Err(err) => {
            eprintln!("{err}");
            return Ok(None);
        }
    };

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

fn run_rustfmt(
    input: &[u8],
    path: &str,
    which: &str,
    config: Option<&Path>,
) -> Result<Vec<u8>> {
    match run_rustfmt_inner(input, config) {
        Ok(output) => Ok(output),
        Err(failure) if failure.is_trailing_whitespace() => {
            let trimmed = trim_trailing_whitespace(input);

            if trimmed != input {
                eprintln!(
                    "note: trimming trailing whitespace while formatting {which} of {path}"
                );
            }

            match run_rustfmt_inner(&trimmed, config) {
                Ok(output) => Ok(output),
                Err(second) => Err(format_rustfmt_error(second, path, which)),
            }
        }
        Err(failure) => Err(format_rustfmt_error(failure, path, which)),
    }
}

impl RustfmtFailure {
    fn is_trailing_whitespace(&self) -> bool {
        fn contains_marker(buf: &[u8]) -> bool {
            !buf.is_empty()
                && String::from_utf8_lossy(buf).contains("left behind trailing whitespace")
        }

        contains_marker(&self.stderr) || contains_marker(&self.stdout)
    }
}

fn format_rustfmt_error(failure: RustfmtFailure, path: &str, which: &str) -> anyhow::Error {
    let mut message = match failure.status {
        Some(status) => format!(
            "rustfmt exited with status {status} while formatting {which} of {path}"
        ),
        None => format!(
            "rustfmt failed while formatting {which} of {path} (no status available)"
        ),
    };

    if !failure.stderr.is_empty() {
        let stderr = String::from_utf8_lossy(&failure.stderr);
        message.push_str(": ");
        message.push_str(stderr.trim());
    } else if !failure.stdout.is_empty() {
        let stdout = String::from_utf8_lossy(&failure.stdout);
        message.push_str(": ");
        message.push_str(stdout.trim());
    }

    anyhow::anyhow!(message)
}

fn trim_trailing_whitespace(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut start = 0;

    while start < input.len() {
        if let Some(rel_newline) = input[start..].iter().position(|&b| b == b'\n') {
            let newline_idx = start + rel_newline;
            let mut end = newline_idx;
            let mut had_cr = false;

            if end > start && input[end - 1] == b'\r' {
                had_cr = true;
                end -= 1;
            }

            while end > start {
                match input[end - 1] {
                    b' ' | b'\t' => end -= 1,
                    _ => break,
                }
            }

            output.extend_from_slice(&input[start..end]);
            if had_cr {
                output.push(b'\r');
            }
            output.push(b'\n');
            start = newline_idx + 1;
        } else {
            let mut end = input.len();
            while end > start {
                match input[end - 1] {
                    b' ' | b'\t' => end -= 1,
                    _ => break,
                }
            }
            output.extend_from_slice(&input[start..end]);
            break;
        }
    }

    output
}

fn run_rustfmt_inner(input: &[u8], config: Option<&Path>) -> Result<Vec<u8>, RustfmtFailure> {
    let mut cmd = Command::new("rustfmt");
    cmd.arg("--emit")
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
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| RustfmtFailure {
            status: None,
            stdout: Vec::new(),
            stderr: format!("failed to spawn rustfmt: {err}").into_bytes(),
        })?;

    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| RustfmtFailure {
                status: None,
                stdout: Vec::new(),
                stderr: b"failed to open rustfmt stdin".to_vec(),
            })?;
        stdin
            .write_all(input)
            .map_err(|err| RustfmtFailure {
                status: None,
                stdout: Vec::new(),
                stderr: format!("failed to write source to rustfmt: {err}").into_bytes(),
            })?;
    }

    let output = child.wait_with_output().map_err(|err| RustfmtFailure {
        status: None,
        stdout: Vec::new(),
        stderr: format!("failed to read rustfmt output: {err}").into_bytes(),
    })?;

    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(RustfmtFailure {
            status: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
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
