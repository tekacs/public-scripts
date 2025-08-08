#!/usr/bin/env scriptr
---
[dependencies]
duct = "0.13"
clap = { version = "4.5", features = ["derive"] }
colored = "2"
anyhow = "1"
blake3 = "1"
kdl = "4"
rayon = "1"
---

use clap::Parser;
use colored::*;
use duct::cmd;
use std::env;
use std::collections::HashMap;
use anyhow::{Result, Context, bail};
use rayon::prelude::*;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::fs;

#[derive(Parser)]
#[command(about = "Enhanced zellij session manager")]
struct Args {
    /// Session name or hash prefix to attach to
    session: Option<String>,
    
    /// Create a new session
    #[arg(short = 'n', long)]
    new: bool,
    
    /// Kill/delete a session
    #[arg(short = 'k', long)]
    kill: bool,
    
    /// List sessions (names only)
    #[arg(short = 'l', long)]
    list: bool,
    
    /// Rename a session (provide old and new names)
    #[arg(short = 'r', long)]
    rename: bool,
    
    /// Include exited sessions
    #[arg(short = 'x', long)]
    include_exited: bool,
    
    /// New name for rename operation (positional second argument)
    new_name: Option<String>,
    
    /// Output completion options (hidden flag)
    #[arg(long, hide = true)]
    completions: bool,
}

#[derive(Debug)]
struct SessionInfo {
    name: String,
    is_current: bool,
    is_exited: bool,
    hash_prefix: String,
}

impl AsRef<SessionInfo> for SessionInfo {
    fn as_ref(&self) -> &SessionInfo {
        self
    }
}

#[derive(Debug)]
struct TabInfo {
    name: String,
    command: Option<String>,
    cwd: Option<String>,
}

fn get_current_session() -> Option<String> {
    env::var("ZELLIJ_SESSION_NAME").ok()
}

fn get_zellij_version() -> Result<String> {
    let output = cmd!("zellij", "--version")
        .read()
        .context("Failed to get zellij version")?;
    
    // Parse "zellij 0.42.2" to get "0.42.2"
    let version = output
        .trim()
        .split_whitespace()
        .nth(1)
        .context("Failed to parse zellij version")?
        .to_string();
    
    Ok(version)
}

fn get_zellij_cache_dir() -> Result<PathBuf> {
    let version = get_zellij_version()?;
    
    let cache_base = if cfg!(target_os = "macos") {
        let home = env::var("HOME").context("HOME not set")?;
        PathBuf::from(home)
            .join("Library")
            .join("Caches")
            .join("org.Zellij-Contributors.Zellij")
            .join(&version)
    } else {
        // Linux and others
        let home = env::var("HOME").context("HOME not set")?;
        PathBuf::from(home)
            .join(".cache")
            .join("zellij")
            .join(&version)
    };
    
    Ok(cache_base)
}

fn load_cached_session_layout(session_name: &str) -> Result<String> {
    let cache_dir = get_zellij_cache_dir()?;
    let layout_path = cache_dir
        .join("session_info")
        .join(session_name)
        .join("session-layout.kdl");
    
    if layout_path.exists() {
        fs::read_to_string(&layout_path)
            .with_context(|| format!("Failed to read cached layout from {:?}", layout_path))
    } else {
        bail!("No cached layout found for session {}", session_name)
    }
}

fn compute_hash_prefix(name: &str) -> String {
    let hash = blake3::hash(name.as_bytes());
    hash.to_hex().chars().take(8).collect()
}

fn find_shortest_prefixes<T: AsRef<SessionInfo>>(sessions: &[T]) -> HashMap<String, String> {
    let mut prefixes = HashMap::new();
    
    for session in sessions {
        let session = session.as_ref();
        // Start with 1 character and increase until unique
        for len in 1..=8 {
            let prefix: String = session.hash_prefix.chars().take(len).collect();
            let is_unique = sessions.iter()
                .map(|s| s.as_ref())
                .filter(|s| s.name != session.name)
                .all(|s| !s.hash_prefix.starts_with(&prefix));
            
            if is_unique {
                prefixes.insert(session.name.clone(), prefix);
                break;
            }
        }
    }
    
    prefixes
}

fn list_sessions(include_exited: bool) -> Result<Vec<SessionInfo>> {
    let output = cmd!("zellij", "list-sessions")
        .read()
        .context("Failed to list zellij sessions")?;
    
    let current_session = get_current_session();
    
    let sessions: Vec<SessionInfo> = output
        .lines()
        .filter(|line| !line.trim().is_empty() && (include_exited || !line.contains("EXITED")))
        .map(|line| {
            let is_exited = line.contains("EXITED");
            
            // Extract session name from the colored output
            let name = if let Some(start) = line.find('\x1b') {
                if let Some(end_start) = line[start..].find("m") {
                    let name_start = start + end_start + 1;
                    if let Some(name_end) = line[name_start..].find('\x1b') {
                        line[name_start..name_start + name_end].trim().to_string()
                    } else {
                        line.split_whitespace().next().unwrap_or("").to_string()
                    }
                } else {
                    line.split_whitespace().next().unwrap_or("").to_string()
                }
            } else {
                line.split_whitespace().next().unwrap_or("").to_string()
            };
            
            let is_current = current_session.as_ref() == Some(&name);
            let hash_prefix = compute_hash_prefix(&name);
            SessionInfo { name, is_current, is_exited, hash_prefix }
        })
        .filter(|s| !s.name.is_empty())
        .collect();
    
    Ok(sessions)
}

fn parse_kdl_layout(layout: &str) -> Result<Vec<TabInfo>> {
    // Parse KDL
    let doc = layout.parse::<kdl::KdlDocument>()
        .context("Failed to parse KDL layout")?;
    
    let mut tabs = Vec::new();
    
    // Find the layout node first
    if let Some(layout_node) = doc.nodes().iter().find(|n| n.name().value() == "layout") {
        if let Some(layout_children) = layout_node.children() {
            // Now find all tab nodes within the layout
            for node in layout_children.nodes() {
                if node.name().value() == "tab" {
                    let mut tab_name = String::from("Tab");
                    let mut panes_info: Vec<(Option<String>, Option<String>)> = Vec::new();
                    
                    // Get tab name if present
                    if let Some(name_entry) = node.entries().iter().find(|e| e.name().map(|n| n.value()) == Some("name")) {
                        if let Some(name_val) = name_entry.value().as_string() {
                            tab_name = name_val.to_string();
                        }
                    }
                    
                    // Look through child nodes for panes
                    if let Some(children) = node.children() {
                        for child in children.nodes() {
                            if child.name().value() == "pane" {
                                let mut command = None;
                                let mut cwd = None;
                                
                                // Get command attribute
                                if let Some(cmd_entry) = child.entries().iter().find(|e| e.name().map(|n| n.value()) == Some("command")) {
                                    if let Some(cmd_val) = cmd_entry.value().as_string() {
                                        command = Some(cmd_val.to_string());
                                    }
                                }
                                
                                // Get cwd attribute
                                if let Some(cwd_entry) = child.entries().iter().find(|e| e.name().map(|n| n.value()) == Some("cwd")) {
                                    if let Some(cwd_val) = cwd_entry.value().as_string() {
                                        cwd = Some(cwd_val.to_string());
                                    }
                                }
                                
                                // Only add if it's not a plugin pane
                                if command.is_some() || cwd.is_some() {
                                    panes_info.push((command, cwd));
                                }
                            }
                        }
                    }
                    
                    // If we found panes, add a tab entry for each unique combination
                    if !panes_info.is_empty() {
                        // Group by command/cwd and take the first of each unique combination
                        let mut seen = std::collections::HashSet::new();
                        for (command, cwd) in panes_info {
                            let key = (command.clone(), cwd.clone());
                            if seen.insert(key) {
                                tabs.push(TabInfo {
                                    name: tab_name.clone(),
                                    command,
                                    cwd,
                                });
                            }
                        }
                    } else {
                        tabs.push(TabInfo {
                            name: tab_name,
                            command: None,
                            cwd: None,
                        });
                    }
                }
            }
        }
    }
    
    Ok(tabs)
}

fn parse_session_tabs(session: &SessionInfo) -> Result<Vec<TabInfo>> {
    if session.is_exited {
        // Try to load from cache for exited sessions
        match load_cached_session_layout(&session.name) {
            Ok(layout) => parse_kdl_layout(&layout),
            Err(_) => {
                // If we can't load cached layout, return empty
                Ok(Vec::new())
            }
        }
    } else {
        // Get the layout dump for live sessions
        let layout = cmd!("zellij", "-s", &session.name, "action", "dump-layout")
            .stderr_null()
            .read()
            .context("Failed to dump layout")?;
        
        parse_kdl_layout(&layout)
    }
}

fn check_dead_session(name: &str) -> Result<Option<SessionInfo>> {
    // List all sessions including exited ones
    let all_sessions = list_sessions(true)?;
    
    // Find a dead session with the given name
    Ok(all_sessions.into_iter()
        .find(|s| s.name == name && s.is_exited))
}

fn resurrect_dead_session(name: &str) -> Result<()> {
    println!("{}: Resurrecting dead session '{}'", "Info".blue(), name.green());
    
    // Try to attach to the dead session, which should resurrect it
    let result = cmd!("zellij", "attach", name)
        .run();
    
    match result {
        Ok(_) => Ok(()),
        Err(_) => {
            // The attach might fail in non-terminal environments but still resurrect the session
            // Check if the session is now active
            let active_sessions = list_sessions(false)?;
            if active_sessions.iter().any(|s| s.name == name && !s.is_exited) {
                // Session was successfully resurrected despite the error
                println!("{}: Session '{}' has been resurrected", "Success".green(), name.green());
                println!("Use '{}' to attach to it", format!("z {}", name).cyan());
                Ok(())
            } else {
                // Session is still dead, offer to delete and recreate
                println!("{}: Session appears to be corrupted.", "Warning".yellow());
                print!("Would you like to delete it and create a new one? [Y/n] ");
                io::stdout().flush()?;
                
                let mut response = String::new();
                io::stdin().read_line(&mut response)?;
                let response = response.trim().to_lowercase();
                
                if response.is_empty() || response == "y" || response == "yes" {
                    // Delete the dead session
                    println!("{}: Deleting dead session '{}'", "Info".blue(), name.yellow());
                    cmd!("zellij", "delete-session", name)
                        .run()
                        .context("Failed to delete dead session")?;
                    
                    // Create a new session
                    create_session(name)?;
                } else {
                    bail!("Session resurrection cancelled");
                }
                Ok(())
            }
        }
    }
}

fn display_sessions_with_tabs(sessions_with_tabs: Vec<(SessionInfo, Result<Vec<TabInfo>>)>) -> Result<()> {
    if sessions_with_tabs.is_empty() {
        println!("{}", "No active zellij sessions found.".dimmed());
        println!();
        println!("Start a new session with: {}", "zellij".green());
        println!("Start a named session with: {}", "zellij -s <name>".green());
        return Ok(());
    }
    
    let sessions: Vec<&SessionInfo> = sessions_with_tabs.iter().map(|(s, _)| s).collect();
    let prefixes = find_shortest_prefixes(&sessions);
    
    for (i, (session, tabs_result)) in sessions_with_tabs.iter().enumerate() {
        let prefix = prefixes.get(&session.name).unwrap();
        
        if session.is_current {
            println!("{} {} {} {}", 
                prefix.yellow().bold(),
                "*".green().bold(), 
                session.name.green().bold(), 
                "(current)".dimmed()
            );
        } else {
            println!("{} {}", 
                prefix.yellow().bold(),
                session.name.cyan()
            );
        }
        
        // Display tab information
        match tabs_result {
            Ok(tabs) => {
                for tab in tabs {
                    let cmd = tab.command.as_deref().unwrap_or("-");
                    let cwd = tab.cwd.as_deref().unwrap_or("-");
                    println!("    {} {} {}", 
                        tab.name.dimmed(),
                        cmd.blue().dimmed(),
                        cwd.dimmed()
                    );
                }
            }
            Err(_) => {
                println!("    {}", "[Unable to fetch tabs]".dimmed());
            }
        }
        
        // Only add blank line between sessions, not after the last one
        if i < sessions_with_tabs.len() - 1 {
            println!();
        }
    }
    
    println!("\n{}: {} or {} to attach", 
        "Usage".yellow(), 
        "z <session-name>".bold(),
        "z <hash-prefix>".bold()
    );
    Ok(())
}

fn attach_or_switch_session(name: &str, sessions: &[SessionInfo]) -> Result<()> {
    // Check if we're already in a zellij session
    if let Some(current) = get_current_session() {
        // Find session by name or hash prefix
        let session = sessions.iter()
            .find(|s| s.name == name || s.hash_prefix.starts_with(name));
        
        match session {
            Some(target) => {
                if target.name == current {
                    println!("{}: Already in session '{}'", "Info".blue(), current.yellow());
                } else {
                    // Switch to the target session
                    println!("{}: Switching from '{}' to '{}'", 
                        "Info".blue(), current.yellow(), target.name.green());
                    cmd!("zellij", "action", "switch-session", &target.name)
                        .run()
                        .context("Failed to switch session")?;
                }
            }
            None => {
                // Session doesn't exist, offer to create it
                offer_to_create_session(name)?;
            }
        }
    } else {
        // Not in a session, try to attach
        let session = sessions.iter()
            .find(|s| s.name == name || s.hash_prefix.starts_with(name));
        
        match session {
            Some(target) => {
                // Attach to the session
                cmd!("zellij", "attach", &target.name)
                    .run()
                    .context("Failed to attach to session")?;
            }
            None => {
                // Session doesn't exist, offer to create it
                offer_to_create_session(name)?;
            }
        }
    }
    
    Ok(())
}

fn offer_to_create_session(name: &str) -> Result<()> {
    // First check if there's a dead session with this name
    if let Some(_dead_session) = check_dead_session(name)? {
        println!("{}: Session '{}' exists but is dead.", "Info".yellow(), name.cyan());
        print!("Would you like to resurrect it? [Y/n] ");
        io::stdout().flush()?;
        
        let mut response = String::new();
        io::stdin().read_line(&mut response)?;
        let response = response.trim().to_lowercase();
        
        if response.is_empty() || response == "y" || response == "yes" {
            resurrect_dead_session(name)?;
        } else {
            println!("Session resurrection cancelled.");
        }
    } else {
        // No dead session found, offer to create a new one
        println!("{}: Session '{}' does not exist.", "Info".yellow(), name.cyan());
        print!("Would you like to create it? [Y/n] ");
        io::stdout().flush()?;
        
        let mut response = String::new();
        io::stdin().read_line(&mut response)?;
        let response = response.trim().to_lowercase();
        
        if response.is_empty() || response == "y" || response == "yes" {
            create_session(name)?;
        } else {
            println!("Session creation cancelled.");
        }
    }
    
    Ok(())
}

fn create_session(name: &str) -> Result<()> {
    println!("{}: Creating session '{}'", "Info".blue(), name.green());
    
    // Check if we're already in a session
    if get_current_session().is_some() {
        // Create detached session
        cmd!("zellij", "-s", name)
            .stderr_null()
            .stdout_null()
            .start()?;
        println!("Session '{}' created. Use '{}' to switch to it.", 
            name.green(), format!("z {}", name).cyan());
    } else {
        // Create and attach
        cmd!("zellij", "-s", name)
            .run()
            .context("Failed to create session")?;
    }
    
    Ok(())
}

fn kill_session(name: &str, sessions: &[SessionInfo]) -> Result<()> {
    // Find session by name or hash prefix
    let session = sessions.iter()
        .find(|s| s.name == name || s.hash_prefix.starts_with(name))
        .context("No session found matching that name or hash prefix")?;
    
    // Prevent killing current session
    if let Some(current) = get_current_session() {
        if session.name == current {
            bail!("Cannot kill the current session. Exit first or switch to another session.");
        }
    }
    
    println!("{}: Killing session '{}'", "Info".blue(), session.name.red());
    cmd!("zellij", "kill-session", &session.name)
        .run()
        .context("Failed to kill session")?;
    
    println!("Session '{}' killed.", session.name.red());
    Ok(())
}

fn rename_session(old_name: &str, new_name: &str, sessions: &[SessionInfo]) -> Result<()> {
    // Find session by name or hash prefix
    let session = sessions.iter()
        .find(|s| s.name == old_name || s.hash_prefix.starts_with(old_name))
        .context("No session found matching that name or hash prefix")?;
    
    // Check if new name already exists
    if sessions.iter().any(|s| s.name == new_name) {
        bail!("Session '{}' already exists", new_name);
    }
    
    println!("{}: Renaming session '{}' to '{}'", 
        "Info".blue(), session.name.yellow(), new_name.green());
    
    // Check if we're renaming the current session
    let in_current = get_current_session()
        .map(|current| current == session.name)
        .unwrap_or(false);
    
    if in_current {
        // Use action command when inside the session
        cmd!("zellij", "action", "rename-session", new_name)
            .run()
            .context("Failed to rename session")?;
    } else {
        // Use regular command when outside
        cmd!("zellij", "rename-session", &session.name, new_name)
            .run()
            .context("Failed to rename session")?;
    }
    
    println!("Session renamed successfully.");
    Ok(())
}

fn list_simple(sessions: &[SessionInfo]) -> Result<()> {
    for session in sessions {
        if session.is_current {
            println!("{} {}", session.name, "(current)".dimmed());
        } else {
            println!("{}", session.name);
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let args = Args::parse();
    let sessions = list_sessions(args.include_exited)?;
    
    if args.completions {
        // Output just session names for completion
        for session in &sessions {
            println!("{}", session.name);
        }
        return Ok(());
    }
    
    // Handle various operations
    if args.list {
        // Simple list mode
        list_simple(&sessions)?;
    } else if args.new {
        // Create new session
        let session_name = args.session
            .context("Session name required for --new flag")?;
        create_session(&session_name)?;
    } else if args.kill {
        // Kill session
        let session_name = args.session
            .context("Session name required for --kill flag")?;
        kill_session(&session_name, &sessions)?;
    } else if args.rename {
        // Rename session
        let old_name = args.session
            .context("Old session name required for --rename flag")?;
        let new_name = args.new_name
            .context("New session name required for --rename flag")?;
        rename_session(&old_name, &new_name, &sessions)?;
    } else {
        // Default behavior: attach/switch or display
        match args.session {
            Some(session_name) => {
                attach_or_switch_session(&session_name, &sessions)?;
            }
            None => {
                // Fetch tab information in parallel
                let sessions_with_tabs: Vec<(SessionInfo, Result<Vec<TabInfo>>)> = sessions
                    .into_par_iter()
                    .map(|session| {
                        let tabs = parse_session_tabs(&session);
                        (session, tabs)
                    })
                    .collect();
                    
                display_sessions_with_tabs(sessions_with_tabs)?;
            }
        }
    }
    
    Ok(())
}