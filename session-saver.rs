#!/usr/bin/env scriptr
---
[dependencies]
clap = { version = "4.5", features = ["derive"] }
colored = "2"
anyhow = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
regex = "1"
dirs = "5"
sysinfo = "0.30"
---

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use colored::*;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::Command;
use sysinfo::{Pid, System};

#[derive(Parser)]
#[command(about = "Save and resume tmux codex sessions")]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Save codex sessions (default: all, or specify session[:window])
    Save {
        /// Target to save (session or session:window)
        target: Option<String>,
    },
    /// Save already-terminated sessions from scrollback
    SaveTerminated {
        /// Target to save (session or session:window)
        target: Option<String>,
    },
    /// List saved sessions
    List,
    /// Discover currently running codex sessions
    Discover {
        /// Optional target to filter (session or session:window)
        target: Option<String>,
    },
    /// Resume saved sessions (default: all, or specify identifier)
    Resume {
        /// Session identifier (window name or substring)
        identifier: Option<String>,
        /// Target tmux session to resume in (creates if doesn't exist)
        #[arg(short = 's', long)]
        session: Option<String>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct SavedSession {
    session: String,
    window: String,
    directory: String,
    resume_command: String,
}

#[derive(Debug)]
struct TmuxWindow {
    session: String,
    window: String,
    command: String,
    directory: String,
    pid: Option<u32>,
}

fn sessions_file() -> PathBuf {
    dirs::home_dir().unwrap().join("sessions.jsonl")
}

fn window_exists(session: &str, window: &str) -> Result<bool> {
    let output = Command::new("tmux")
        .args(&["list-windows", "-t", session, "-F", "#{window_name}"])
        .output()?;

    if !output.status.success() {
        // Session doesn't exist
        return Ok(false);
    }

    let windows = String::from_utf8_lossy(&output.stdout);
    Ok(windows.lines().any(|w| w == window))
}

fn load_saved_sessions() -> Result<Vec<SavedSession>> {
    let sessions_file = sessions_file();

    if !sessions_file.exists() {
        return Ok(Vec::new());
    }

    let file = File::open(&sessions_file).context("Failed to open sessions file")?;
    let reader = BufReader::new(file);

    let mut sessions = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        if let Ok(session) = serde_json::from_str::<SavedSession>(&line) {
            sessions.push(session);
        }
    }

    Ok(sessions)
}

fn is_session_saved(session: &str, window: &str) -> Result<bool> {
    let saved = load_saved_sessions()?;
    Ok(saved
        .iter()
        .any(|s| s.session == session && s.window == window))
}

fn write_session_to_file(session: &SavedSession) -> Result<()> {
    let sessions_file = sessions_file();
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&sessions_file)
        .context("Failed to open sessions file")?;

    let json = serde_json::to_string(session)?;
    writeln!(file, "{}", json)?;

    Ok(())
}

fn extract_session_id_from_resume_command(cmd: &str) -> Option<String> {
    let re = Regex::new(r"codex\s+resume\s+([0-9a-fA-F-]{36})").ok()?;
    re.captures(cmd)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
}

fn write_or_update_session(session: &SavedSession) -> Result<()> {
    let sessions_path = sessions_file();
    let sid = extract_session_id_from_resume_command(&session.resume_command)
        .ok_or_else(|| anyhow::anyhow!("resume command missing session id"))?;

    // If file doesn't exist, just write a fresh line
    if !sessions_path.exists() {
        return write_session_to_file(session);
    }

    let mut lines: Vec<String> = {
        let f = File::open(&sessions_path).context("open sessions file")?;
        let r = BufReader::new(f);
        r.lines().collect::<std::io::Result<Vec<_>>>()?
    };

    // Find an existing line with the same session id and update it in place
    let mut updated = false;
    for line in &mut lines {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(mut existing) = serde_json::from_str::<SavedSession>(line) {
            if let Some(existing_sid) =
                extract_session_id_from_resume_command(&existing.resume_command)
            {
                if existing_sid == sid {
                    // Update the mapping to the new tmux session/window (and directory)
                    existing.session = session.session.clone();
                    existing.window = session.window.clone();
                    existing.directory = session.directory.clone();
                    *line = serde_json::to_string(&existing)?;
                    updated = true;
                }
            }
        }
    }

    if updated {
        // Overwrite the file with updated contents
        let mut f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&sessions_path)
            .context("rewrite sessions file")?;
        for l in lines {
            f.write_all(l.as_bytes())?;
            f.write_all(b"\n")?;
        }
        Ok(())
    } else {
        // Append as a new entry
        write_session_to_file(session)
    }
}

fn run_tmux_command(args: &[&str]) -> Result<String> {
    let output = Command::new("tmux")
        .args(args)
        .output()
        .context("Failed to run tmux command")?;

    if !output.status.success() {
        bail!(
            "tmux command failed: {:?}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(String::from_utf8(output.stdout)?)
}

fn extract_session_id_from_path(path: &str) -> Option<String> {
    let re = Regex::new(r"rollout-[0-9T:-]+-([0-9a-fA-F-]{36})\.jsonl").ok()?;
    re.captures(path)
        .and_then(|caps| caps.get(1).map(|m| m.as_str().to_string()))
}

fn find_session_id_from_pid(pid: u32) -> Result<Option<String>> {
    let output = Command::new("lsof")
        .args(&["-p", &pid.to_string(), "-Fn"])
        .output()
        .context("Failed to run lsof")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("lsof failed for pid {}: {}", pid, stderr.trim());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Some(path) = line.strip_prefix('n') {
            if let Some(session_id) = extract_session_id_from_path(path) {
                return Ok(Some(session_id));
            }
        }
    }

    Ok(None)
}

fn resolve_pane_pid(window: &TmuxWindow) -> Result<Option<u32>> {
    if let Some(pid) = window.pid {
        return Ok(Some(pid));
    }

    let target = format!("{}:{}", window.session, window.window);
    let output = Command::new("tmux")
        .args(&["display-message", "-p", "-t", &target, "#{pane_pid}"])
        .output()
        .context("Failed to query pane pid")?;

    if !output.status.success() {
        return Ok(None);
    }

    let pid_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if pid_str.is_empty() {
        return Ok(None);
    }

    Ok(pid_str.parse::<u32>().ok())
}

fn find_codex_process(pane_pid: u32) -> Option<u32> {
    let mut system = System::new();
    system.refresh_processes();

    let start = Pid::from_u32(pane_pid);
    let mut queue = VecDeque::new();
    let mut visited = HashSet::new();
    queue.push_back(start);

    while let Some(pid) = queue.pop_front() {
        if !visited.insert(pid) {
            continue;
        }

        let process = match system.process(pid) {
            Some(proc) => proc,
            None => continue,
        };

        let exe_matches = process
            .exe()
            .and_then(|path| path.file_name().and_then(|s| s.to_str()))
            .map(|s| s.contains("codex"))
            .unwrap_or(false);

        let cmd_matches = process.cmd().iter().any(|arg| arg.contains("codex"));

        let name_matches = process.name().contains("codex");

        if exe_matches || cmd_matches || name_matches {
            return Some(pid.as_u32());
        }

        for (child_pid, child_proc) in system.processes() {
            if child_proc.parent() == Some(pid) {
                queue.push_back(*child_pid);
            }
        }
    }

    None
}

fn resolve_codex_pid(window: &TmuxWindow) -> Result<Option<u32>> {
    let pane_pid = match resolve_pane_pid(window)? {
        Some(pid) => pid,
        None => return Ok(None),
    };

    Ok(find_codex_process(pane_pid))
}

fn list_codex_windows(
    target_session: Option<&str>,
    target_window: Option<&str>,
) -> Result<Vec<TmuxWindow>> {
    // Get all sessions
    let sessions_output = run_tmux_command(&["list-sessions", "-F", "#{session_name}"])?;
    let sessions: Vec<&str> = sessions_output.lines().collect();

    let mut codex_windows = Vec::new();

    for session in sessions {
        // Skip if we have a target session and this isn't it
        if let Some(target) = target_session {
            if session != target {
                continue;
            }
        }

        // List windows for this session
        let windows_output = run_tmux_command(&[
            "list-windows",
            "-t",
            session,
            "-F",
            "#{window_name}|#{pane_current_command}|#{pane_current_path}|#{pane_pid}",
        ])?;

        for line in windows_output.lines() {
            let parts: Vec<&str> = line.split('|').collect();
            if parts.len() != 4 {
                continue;
            }

            let window = parts[0];
            let command = parts[1];
            let directory = parts[2];
            let pid = parts[3].parse::<u32>().ok();

            // Skip if we have a target window and this isn't it
            if let Some(target) = target_window {
                if window != target {
                    continue;
                }
            }

            // Only include codex windows
            if command == "codex" {
                codex_windows.push(TmuxWindow {
                    session: session.to_string(),
                    window: window.to_string(),
                    command: command.to_string(),
                    directory: directory.to_string(),
                    pid,
                });
            }
        }
    }

    Ok(codex_windows)
}

fn extract_resume_command(scrollback: &str) -> Option<String> {
    let re =
        Regex::new(r"codex resume ([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})")
            .unwrap();

    // Search from the end of the scrollback
    for line in scrollback.lines().rev() {
        if let Some(captures) = re.captures(line) {
            if let Some(uuid) = captures.get(1) {
                return Some(format!("codex resume {}", uuid.as_str()));
            }
        }
    }

    None
}

fn save_window(window: &TmuxWindow) -> Result<Option<SavedSession>> {
    println!(
        "{} Saving {} in session {}",
        "→".blue(),
        window.window.yellow(),
        window.session.cyan()
    );

    let codex_pid = match resolve_codex_pid(window)? {
        Some(pid) => pid,
        None => {
            println!(
                "  {} No codex child process found for this pane (try running a command first)",
                "✗".red()
            );
            return Ok(None);
        }
    };

    match find_session_id_from_pid(codex_pid) {
        Ok(Some(session_id)) => {
            let resume_command = format!("codex resume {}", session_id);
            println!(
                "  {} PID {} is recording {}",
                "✓".green(),
                codex_pid,
                session_id.dimmed()
            );

            Ok(Some(SavedSession {
                session: window.session.clone(),
                window: window.window.clone(),
                directory: window.directory.clone(),
                resume_command,
            }))
        }
        Ok(None) => {
            println!(
                "  {} No open rollout file found for PID {}. Try `sudo lsof -p {}`.",
                "✗".red(),
                codex_pid,
                codex_pid
            );
            Ok(None)
        }
        Err(err) => {
            println!("  {} {}", "✗".red(), err);
            Ok(None)
        }
    }
}

fn save_sessions(target: Option<String>) -> Result<()> {
    // Parse target into session and optional window
    let (target_session, target_window) = if let Some(target) = target.as_ref() {
        if let Some((session, window)) = target.split_once(':') {
            (Some(session), Some(window))
        } else {
            (Some(target.as_str()), None)
        }
    } else {
        (None, None)
    };

    // Find all codex windows
    let windows = list_codex_windows(target_session, target_window)?;

    if windows.is_empty() {
        println!("{}", "No codex windows found.".yellow());
        return Ok(());
    }

    println!("Found {} codex window(s)\n", windows.len());

    // Save each window
    let mut saved_count = 0;

    for window in &windows {
        if let Some(saved) = save_window(window)? {
            write_or_update_session(&saved)?;
            saved_count += 1;
        }
        println!();
    }

    println!(
        "{} Saved {} session(s) to {}",
        "✓".green().bold(),
        saved_count,
        sessions_file().display().to_string().dimmed()
    );

    Ok(())
}

fn save_terminated_sessions(target: Option<String>) -> Result<()> {
    // Parse target into session and optional window
    let (target_session, target_window) = if let Some(target) = target.as_ref() {
        if let Some((session, window)) = target.split_once(':') {
            (Some(session), Some(window))
        } else {
            (Some(target.as_str()), None)
        }
    } else {
        (None, None)
    };

    // Get all sessions
    let sessions_output = run_tmux_command(&["list-sessions", "-F", "#{session_name}"])?;
    let sessions: Vec<&str> = sessions_output.lines().collect();

    let mut found_windows = Vec::new();

    for session in sessions {
        // Skip if we have a target session and this isn't it
        if let Some(target) = target_session {
            if session != target {
                continue;
            }
        }

        // List windows for this session
        let windows_output = run_tmux_command(&[
            "list-windows",
            "-t",
            session,
            "-F",
            "#{window_name}|#{pane_current_command}|#{pane_current_path}",
        ])?;

        for line in windows_output.lines() {
            let parts: Vec<&str> = line.split('|').collect();
            if parts.len() != 3 {
                continue;
            }

            let window = parts[0];
            let command = parts[1];
            let directory = parts[2];

            // Skip if we have a target window and this isn't it
            if let Some(target) = target_window {
                if window != target {
                    continue;
                }
            }

            // Only look at terminated sessions (NOT running codex)
            if command != "codex" {
                found_windows.push((
                    session.to_string(),
                    window.to_string(),
                    directory.to_string(),
                ));
            }
        }
    }

    if found_windows.is_empty() {
        println!("{}", "No terminated sessions found.".yellow());
        return Ok(());
    }

    println!("Found {} terminated window(s)\n", found_windows.len());

    let mut saved_count = 0;
    let mut skipped_count = 0;

    for (session, window, directory) in found_windows {
        // Check if already saved
        if is_session_saved(&session, &window)? {
            println!(
                "{} {} in session {} (already saved)",
                "⊘".dimmed(),
                window.dimmed(),
                session.dimmed()
            );
            skipped_count += 1;
            continue;
        }

        println!(
            "{} Checking {} in session {}",
            "→".blue(),
            window.yellow(),
            session.cyan()
        );

        // Capture scrollback
        let target = format!("{}:{}", session, window);
        let scrollback = run_tmux_command(&["capture-pane", "-p", "-t", &target, "-S", "-100"])?;

        // Extract resume command
        if let Some(resume_command) = extract_resume_command(&scrollback) {
            println!("  {} Found: {}", "✓".green(), resume_command.dimmed());

            let saved = SavedSession {
                session: session.clone(),
                window: window.clone(),
                directory: directory.clone(),
                resume_command,
            };

            write_or_update_session(&saved)?;
            saved_count += 1;
        } else {
            println!("  {} No resume command found", "✗".red());
        }

        println!();
    }

    println!(
        "{} Saved {} session(s), skipped {} (already saved)",
        "✓".green().bold(),
        saved_count,
        skipped_count
    );

    Ok(())
}

fn list_saved_sessions() -> Result<()> {
    let sessions_file = sessions_file();

    if !sessions_file.exists() {
        println!("{}", "No saved sessions found.".yellow());
        return Ok(());
    }

    let file = File::open(&sessions_file).context("Failed to open sessions file")?;
    let reader = BufReader::new(file);

    println!("{}\n", "Saved sessions:".bold());

    for (i, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        match serde_json::from_str::<SavedSession>(&line) {
            Ok(session) => {
                println!(
                    "{:3}. {} {} {}",
                    (i + 1).to_string().dimmed(),
                    session.window.yellow().bold(),
                    format!("({}:{})", session.session, session.window).dimmed(),
                    session.directory.cyan()
                );
                println!("     {}", session.resume_command.dimmed());
            }
            Err(_) => {
                println!(
                    "{:3}. {}",
                    (i + 1).to_string().dimmed(),
                    "[Invalid entry]".red()
                );
            }
        }
    }

    Ok(())
}

fn discover_live_sessions(target: Option<String>) -> Result<()> {
    // Parse target into session and optional window
    let (target_session, target_window) = if let Some(target) = target.as_ref() {
        if let Some((session, window)) = target.split_once(':') {
            (Some(session), Some(window))
        } else {
            (Some(target.as_str()), None)
        }
    } else {
        (None, None)
    };

    let windows = list_codex_windows(target_session, target_window)?;

    if windows.is_empty() {
        println!("{}", "No running codex sessions found.".yellow());
        return Ok(());
    }

    println!("{}\n", "Running codex sessions:".bold());

    for (i, window) in windows.iter().enumerate() {
        let entry_prefix = format!("{:3}.", (i + 1).to_string().dimmed());
        let header = format!(
            "{} {} {}",
            window.window.yellow().bold(),
            format!("({}:{})", window.session, window.window).dimmed(),
            window.directory.cyan()
        );

        let pane_status = match resolve_pane_pid(window)? {
            Some(pane_pid) => {
                let pane_part = format!("pane {}", pane_pid.to_string().cyan());
                if let Some(codex_pid) = find_codex_process(pane_pid) {
                    let codex_part = format!("codex {}", codex_pid.to_string().green());
                    match find_session_id_from_pid(codex_pid) {
                        Ok(Some(session_id)) => format!(
                            "{pane_part} → {codex_part} • session {}",
                            session_id.yellow()
                        ),
                        Ok(None) => format!(
                            "{pane_part} → {codex_part} • {}",
                            "rollout file not visible (try sudo)".dimmed()
                        ),
                        Err(err) => {
                            format!("{pane_part} → {codex_part} • {}", err.to_string().red())
                        }
                    }
                } else {
                    format!(
                        "{pane_part} • {}",
                        "no codex child process detected (idle shell?)".dimmed()
                    )
                }
            }
            None => "Unable to determine pane PID (tmux may be older than 3.2)".to_string(),
        };

        println!("{entry_prefix} {header}");
        println!("     {}", pane_status);
    }

    println!(
        "\n{}: {}",
        "Tip".blue(),
        "Use 'session-saver save <session>:<window>' to save a specific session".dimmed()
    );

    Ok(())
}

fn resume_session_single(saved: &SavedSession, target_session: Option<String>) -> Result<()> {
    let target_session = target_session.unwrap_or_else(|| saved.session.clone());

    println!(
        "{} Resuming {} in session {}",
        "→".blue(),
        saved.window.yellow(),
        target_session.cyan()
    );

    // Check if session exists
    let session_exists = Command::new("tmux")
        .args(&["has-session", "-t", &target_session])
        .output()?
        .status
        .success();

    let window_id = if !session_exists {
        // Create session with the first window already named to avoid default "zsh" window
        println!(
            "  Creating session {} with window {}",
            target_session.cyan(),
            saved.window.yellow()
        );
        let output = Command::new("tmux")
            .args(&[
                "new-session",
                "-d",
                "-s",
                &target_session,
                "-n",
                &saved.window,
                "-c",
                &saved.directory,
                "-P", // Print session:window.pane info
            ])
            .output()
            .context("Failed to create tmux session")?;

        if !output.status.success() {
            bail!("Failed to create session");
        }

        // Parse output like "0:1.1" to get window index
        let window_info = String::from_utf8_lossy(&output.stdout);
        window_info.trim().to_string()
    } else {
        // Session exists, create new window
        // Use "session:" format to avoid confusion with window indices
        let session_target = format!("{}:", target_session);
        println!("  Creating window {}", saved.window.yellow());
        let output = Command::new("tmux")
            .args(&[
                "new-window",
                "-t",
                &session_target,
                "-n",
                &saved.window,
                "-c",
                &saved.directory,
                "-P", // Print session:window.pane info
            ])
            .output()
            .context("Failed to create window")?;

        if !output.status.success() {
            bail!("Failed to create window");
        }

        // Parse output like "0:2.1" to get window index
        let window_info = String::from_utf8_lossy(&output.stdout);
        window_info.trim().to_string()
    };

    // Send the resume command
    // Use the exact window ID we got from creating the window
    println!("  Running: {}", saved.resume_command.dimmed());
    Command::new("tmux")
        .args(&[
            "send-keys",
            "-t",
            &window_id,
            &saved.resume_command,
            "Enter",
        ])
        .output()
        .context("Failed to send resume command")?;

    println!("{} Session resumed", "✓".green().bold());

    Ok(())
}

fn resume_sessions(identifier: Option<String>, target_session: Option<String>) -> Result<()> {
    let saved_sessions = load_saved_sessions()?;

    if saved_sessions.is_empty() {
        println!("{}", "No saved sessions found.".yellow());
        return Ok(());
    }

    let sessions_to_resume = if let Some(id) = identifier {
        // Find matching sessions
        let matches: Vec<SavedSession> = saved_sessions
            .into_iter()
            .filter(|s| s.window.contains(&id))
            .collect();

        if matches.is_empty() {
            bail!("No saved session matching '{}'", id);
        }

        if matches.len() > 1 {
            println!("{} Multiple matches found:", "!".yellow());
            for (i, session) in matches.iter().enumerate() {
                println!(
                    "  {}. {} ({}:{})",
                    i + 1,
                    session.window.yellow(),
                    session.session,
                    session.window
                );
            }
            bail!("Please be more specific");
        }

        matches
    } else {
        // Resume all sessions
        saved_sessions
    };

    println!("Resuming {} session(s)\n", sessions_to_resume.len());

    let mut resumed_count = 0;
    let mut skipped_count = 0;

    for session in &sessions_to_resume {
        // Check if window already exists before trying to resume
        let already_exists = window_exists(
            &target_session.as_ref().unwrap_or(&session.session),
            &session.window,
        )
        .unwrap_or(false);

        if already_exists {
            println!(
                "{} {} in session {} (already running)",
                "⊘".dimmed(),
                session.window.dimmed(),
                target_session.as_ref().unwrap_or(&session.session).dimmed()
            );
            skipped_count += 1;
        } else {
            match resume_session_single(session, target_session.clone()) {
                Ok(_) => resumed_count += 1,
                Err(e) => {
                    println!("  {} Failed to resume {}: {}", "✗".red(), session.window, e);
                }
            }
        }
        println!();
    }

    println!(
        "{} Resumed {} session(s), skipped {} (already running)",
        "✓".green().bold(),
        resumed_count,
        skipped_count
    );

    if resumed_count > 0 {
        println!(
            "\n{}: {}",
            "Tip".blue(),
            "Use 'tmux ls' to see all sessions".dimmed()
        );
    }

    Ok(())
}

fn main() -> Result<()> {
    let args = Args::parse();

    match args.command {
        Commands::Save { target } => save_sessions(target),
        Commands::SaveTerminated { target } => save_terminated_sessions(target),
        Commands::List => list_saved_sessions(),
        Commands::Discover { target } => discover_live_sessions(target),
        Commands::Resume {
            identifier,
            session,
        } => resume_sessions(identifier, session),
    }
}
