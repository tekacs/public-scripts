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
use anyhow::{Result, Context};
use rayon::prelude::*;

#[derive(Parser)]
#[command(about = "Enhanced zellij session manager")]
struct Args {
    /// Session name or hash prefix to attach to
    session: Option<String>,
    
    /// Output completion options (hidden flag)
    #[arg(long, hide = true)]
    completions: bool,
}

#[derive(Debug)]
struct SessionInfo {
    name: String,
    is_current: bool,
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

fn list_sessions() -> Result<Vec<SessionInfo>> {
    let output = cmd!("zellij", "list-sessions")
        .read()
        .context("Failed to list zellij sessions")?;
    
    let current_session = get_current_session();
    
    let sessions: Vec<SessionInfo> = output
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.contains("EXITED"))
        .map(|line| {
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
            SessionInfo { name, is_current, hash_prefix }
        })
        .filter(|s| !s.name.is_empty())
        .collect();
    
    Ok(sessions)
}

fn parse_session_tabs(session_name: &str) -> Result<Vec<TabInfo>> {
    // Get the layout dump
    let layout = cmd!("zellij", "-s", session_name, "action", "dump-layout")
        .stderr_null()
        .read()
        .context("Failed to dump layout")?;
    
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

fn attach_session(name: &str, sessions: &[SessionInfo]) -> Result<()> {
    // Check if we're already in a zellij session
    if let Some(current) = get_current_session() {
        eprintln!("{}: Already in zellij session: {}", "Error".red().bold(), current.yellow());
        eprintln!("Use '{}' or exit first", "zellij action switch-session".green());
        std::process::exit(1);
    }
    
    // Find session by name or hash prefix
    let session = sessions.iter()
        .find(|s| s.name == name || s.hash_prefix.starts_with(name))
        .context("No session found matching that name or hash prefix")?;
    
    // Attach to the session
    cmd!("zellij", "attach", &session.name)
        .run()
        .context("Failed to attach to session")?;
    
    Ok(())
}

fn main() -> Result<()> {
    let args = Args::parse();
    let sessions = list_sessions()?;
    
    if args.completions {
        // Output just session names for completion
        for session in &sessions {
            println!("{}", session.name);
        }
        return Ok(());
    }
    
    match args.session {
        Some(session_name) => {
            attach_session(&session_name, &sessions)?;
        }
        None => {
            // Fetch tab information in parallel
            let sessions_with_tabs: Vec<(SessionInfo, Result<Vec<TabInfo>>)> = sessions
                .into_par_iter()
                .map(|session| {
                    let tabs = parse_session_tabs(&session.name);
                    (session, tabs)
                })
                .collect();
                
            display_sessions_with_tabs(sessions_with_tabs)?;
        }
    }
    
    Ok(())
}