#!/usr/bin/env scriptr
---
[dependencies]
clap = { version = "4.5", features = ["derive"] }
colored = "2"
anyhow = "1"
dirs = "5"
---

use clap::Parser;
use colored::*;
use anyhow::{Result, Context, bail};
use std::path::{Path, PathBuf};
use std::fs;
use std::os::unix::fs::symlink;
use std::env;

#[derive(Parser)]
#[command(about = "Install scriptr scripts and shell completions")]
struct Args {
    /// Specific scripts to install (installs all if none specified)
    scripts: Vec<String>,
    
    /// Directory to symlink scripts into
    #[arg(short, long, default_value = "~/bin")]
    bin_dir: String,
    
    /// Shell to set up completions for (fish, bash, zsh)
    #[arg(short, long)]
    shell: Option<String>,
    
    /// Force overwrite existing symlinks
    #[arg(short, long)]
    force: bool,
    
    /// List what would be installed without doing it
    #[arg(long)]
    dry_run: bool,
}

fn expand_tilde(path: &str) -> PathBuf {
    if path.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            home.join(&path[2..])
        } else {
            PathBuf::from(path)
        }
    } else {
        PathBuf::from(path)
    }
}

fn detect_shell() -> Option<String> {
    // First try SHELL environment variable
    if let Ok(shell_path) = env::var("SHELL") {
        if let Some(shell_name) = Path::new(&shell_path).file_name() {
            return Some(shell_name.to_string_lossy().to_string());
        }
    }
    None
}

fn get_shell_completion_dir(shell: &str) -> Result<Option<PathBuf>> {
    match shell {
        "fish" => {
            // Fish looks in ~/.config/fish/completions/
            if let Some(home) = dirs::home_dir() {
                Ok(Some(home.join(".config").join("fish").join("completions")))
            } else {
                bail!("Could not determine home directory for fish")
            }
        }
        "bash" => {
            // Bash completions can go in several places
            // Try ~/.local/share/bash-completion/completions first
            if let Some(data) = dirs::data_local_dir() {
                Ok(Some(data.join("bash-completion").join("completions")))
            } else {
                bail!("Could not determine data directory for bash")
            }
        }
        "zsh" => {
            // Zsh looks in directories in $fpath
            // Common location is ~/.zsh/completions or ~/.local/share/zsh/site-functions
            if let Some(data) = dirs::data_local_dir() {
                Ok(Some(data.join("zsh").join("site-functions")))
            } else {
                bail!("Could not determine data directory for zsh")
            }
        }
        _ => Ok(None),
    }
}

fn find_scripts(repo_dir: &Path, filter: Option<&[String]>) -> Result<Vec<PathBuf>> {
    let mut scripts = Vec::new();
    
    if let Some(names) = filter {
        // Find specific scripts by name (with or without .rs extension)
        for name in names {
            // Try with .rs extension first
            let path_with_rs = repo_dir.join(format!("{}.rs", name));
            let path_without_rs = repo_dir.join(name);
            
            let path = if path_with_rs.exists() {
                path_with_rs
            } else if path_without_rs.exists() && path_without_rs.extension().map_or(false, |e| e == "rs") {
                path_without_rs
            } else {
                bail!("Script '{}' not found in {} (looked for {}.rs)", name, repo_dir.display(), name);
            };
            
            if !path.is_file() {
                bail!("'{}' is not a file", path.display());
            }
            // Check if it's executable
            if let Ok(metadata) = fs::metadata(&path) {
                use std::os::unix::fs::PermissionsExt;
                if metadata.permissions().mode() & 0o111 == 0 {
                    bail!("'{}' is not executable", path.display());
                }
            }
            scripts.push(path);
        }
    } else {
        // Find all executable .rs scripts and also check meta/ subdirectory
        let dirs_to_check = vec![repo_dir.to_path_buf(), repo_dir.join("meta")];
        
        for dir in dirs_to_check {
            if dir.exists() {
                for entry in fs::read_dir(&dir)? {
                    let entry = entry?;
                    let path = entry.path();
                    
                    // Only include .rs files that are executable
                    if path.is_file() && path.extension().map_or(false, |e| e == "rs") {
                        // Check if it's executable
                        if let Ok(metadata) = fs::metadata(&path) {
                            use std::os::unix::fs::PermissionsExt;
                            if metadata.permissions().mode() & 0o111 != 0 {
                                scripts.push(path);
                            }
                        }
                    }
                }
            }
        }
        scripts.sort();
    }
    
    Ok(scripts)
}

fn validate_existing_symlink(link_path: &Path, expected_target: &Path) -> Result<bool> {
    if !link_path.exists() {
        return Ok(true); // No conflict
    }
    
    if link_path.is_symlink() {
        let target = fs::read_link(link_path)?;
        let canonical_target = if target.is_relative() {
            link_path.parent().unwrap().join(&target).canonicalize()?
        } else {
            target.canonicalize()?
        };
        let canonical_expected = expected_target.canonicalize()?;
        
        Ok(canonical_target == canonical_expected)
    } else {
        Ok(false) // It's a regular file, not a symlink
    }
}

fn install_script(script: &Path, bin_dir: &Path, force: bool, dry_run: bool) -> Result<()> {
    let script_name_full = script.file_name().unwrap().to_string_lossy();
    // Remove .rs extension for the symlink name
    let link_name = if script_name_full.ends_with(".rs") {
        &script_name_full[..script_name_full.len() - 3]
    } else {
        &script_name_full
    };
    let link_path = bin_dir.join(link_name);
    
    // Check what's at the target location
    if link_path.is_symlink() {
        // It's a symlink - validate it points to the right place
        if let Ok(target) = fs::read_link(&link_path) {
            let canonical_target = if target.is_relative() {
                link_path.parent().unwrap().join(&target).canonicalize().ok()
            } else {
                target.canonicalize().ok()
            };
            let canonical_expected = script.canonicalize().ok();
            
            if canonical_target.is_some() && canonical_target == canonical_expected {
                // Symlink is correct
                println!("   {} {} {}", 
                    "✓".green().dimmed(), 
                    link_name.dimmed(),
                    "(already installed)".dimmed()
                );
                return Ok(());
            }
        }
        
        // Symlink is broken or points to wrong location, update it
        if !dry_run {
            fs::remove_file(&link_path)?;
        }
        println!("   {} {} {}", 
            "🔄".yellow(), 
            link_name.bold(),
            "(updating symlink)".dimmed()
        );
    } else if link_path.exists() {
        // It's a regular file or directory - can't overwrite
        bail!("Regular file exists at {}. Cannot create symlink. Use --force to overwrite.", 
            link_path.display());
    }
    
    // Create the symlink
    if !dry_run {
        symlink(script, &link_path)
            .with_context(|| format!("Failed to create symlink from {} to {}", 
                link_path.display(), script.display()))?;
    }
    
    if !link_path.is_symlink() || dry_run {
        println!("   {} {}", 
            if dry_run { "→" } else { "✓" }.green().bold(), 
            link_name.bold()
        );
    }
    
    Ok(())
}

fn install_completion(completion_file: &Path, shell: &str, completion_dir: &Path, dry_run: bool) -> Result<()> {
    let completion_name = completion_file.file_name().unwrap();
    let target_path = completion_dir.join(completion_name);
    
    // Check if already exists
    if target_path.exists() {
        if !dry_run {
            // Compare contents to see if update needed
            let source_content = fs::read_to_string(completion_file)?;
            let target_content = fs::read_to_string(&target_path)?;
            
            if source_content == target_content {
                // Extract script name for display
                let script_name = completion_name.to_string_lossy()
                    .trim_end_matches(&format!(".{}", shell))
                    .to_string();
                
                println!("   {} {} {}", 
                    "✓".green().dimmed(),
                    script_name.dimmed(),
                    "(already installed)".dimmed()
                );
                return Ok(());
            }
        } else {
            // In dry-run mode, just report it exists
            let script_name = completion_name.to_string_lossy()
                .trim_end_matches(&format!(".{}", shell))
                .to_string();
            
            println!("   {} {} {}", 
                "✓".green().dimmed(),
                script_name.dimmed(),
                "(already installed)".dimmed()
            );
            return Ok(());
        }
    }
    
    if !dry_run {
        // Create completion directory if it doesn't exist
        fs::create_dir_all(completion_dir)
            .with_context(|| format!("Failed to create completion directory: {}", completion_dir.display()))?;
        
        // Copy the completion file
        fs::copy(completion_file, &target_path)
            .with_context(|| format!("Failed to copy {} to {}", completion_file.display(), target_path.display()))?;
    }
    
    // Extract script name from completion filename (e.g., "z.fish" -> "z")
    let script_name = completion_name.to_string_lossy()
        .trim_end_matches(&format!(".{}", shell))
        .to_string();
    
    println!("   {} {}", 
        if dry_run { "→" } else { "✓" }.green().bold(),
        script_name.bold()
    );
    
    Ok(())
}

fn main() -> Result<()> {
    let args = Args::parse();
    
    // Get repo directory (current working directory)
    let repo_dir = env::current_dir()?;
    
    // Expand and create bin directory
    let bin_dir = expand_tilde(&args.bin_dir);
    if !args.dry_run {
        fs::create_dir_all(&bin_dir)
            .context("Failed to create bin directory")?;
    }
    
    // Determine shell
    let shell = args.shell.or_else(detect_shell);
    
    // Find scripts (filtered or all)
    let filter = if args.scripts.is_empty() {
        None
    } else {
        Some(args.scripts.as_slice())
    };
    let scripts = find_scripts(&repo_dir, filter)?;
    
    if args.dry_run {
        println!("{}", "──────────────────────────────────────".dimmed());
        println!("{}", "DRY RUN MODE".yellow().bold());
        println!("{}", "No changes will be made".yellow());
        println!("{}", "──────────────────────────────────────".dimmed());
        println!();
    }
    
    // Install scripts
    println!("{} {}", 
        "📦 Scripts".bold(), 
        format!("({} found)", scripts.len()).dimmed()
    );
    println!("   {} {}", 
        "Target:".dimmed(),
        bin_dir.display().to_string().cyan()
    );
    println!();
    
    for script in &scripts {
        install_script(script, &bin_dir, args.force, args.dry_run)?;
    }
    
    // Install completions if shell is specified
    if let Some(shell_name) = shell {
        println!();
        println!("{} {} {}", 
            "🐚 Completions".bold(),
            "for".dimmed(),
            shell_name.cyan()
        );
        
        if let Some(completion_dir) = get_shell_completion_dir(&shell_name)? {
            // Look for completion files
            let completions_dir = repo_dir.join("completions");
            if completions_dir.exists() {
                let mut found_completions = false;
                for entry in fs::read_dir(&completions_dir)? {
                    let entry = entry?;
                    let path = entry.path();
                    
                    // Match completion files for the specified shell
                    if path.is_file() {
                        let name = path.file_name().unwrap().to_string_lossy();
                        if name.ends_with(&format!(".{}", shell_name)) {
                            // Check if this completion is for a script we have (whether newly installed or not)
                            let script_name = name.trim_end_matches(&format!(".{}", shell_name));
                            let script_exists = scripts.iter().any(|s| {
                                s.file_name()
                                    .map(|n| {
                                        let name_str = n.to_string_lossy();
                                        // Match either exact name or name.rs
                                        name_str == script_name || 
                                        name_str == format!("{}.rs", script_name)
                                    })
                                    .unwrap_or(false)
                            });
                            
                            if script_exists {
                                install_completion(&path, &shell_name, &completion_dir, args.dry_run)?;
                                found_completions = true;
                            }
                        }
                    }
                }
                
                if !found_completions && !scripts.is_empty() {
                    println!("   {} No completions found for installed scripts", "ℹ️ ".dimmed());
                }
            }
            
            if shell_name == "fish" && !args.dry_run {
                println!();
                println!("   {} Run {} to reload completions", 
                    "💡".yellow(),
                    "source ~/.config/fish/config.fish".cyan()
                );
            }
        } else {
            println!("   {} Unknown shell: {}", "⚠️ ".yellow(), shell_name);
        }
    }
    
    if !args.dry_run {
        println!();
        println!("{}", "──────────────────────────────────────".dimmed());
    }
    println!();
    println!("{} {}", "✨", "Done!".green().bold());
    
    // Check if bin_dir is in PATH
    if let Ok(path_var) = env::var("PATH") {
        let bin_dir_str = bin_dir.to_string_lossy();
        if !path_var.split(':').any(|p| p == bin_dir_str) {
            println!();
            println!("{} {} {}", 
                "⚠️ ".yellow(),
                bin_dir_str.yellow(),
                "is not in your PATH".dimmed()
            );
            println!();
            println!("   Add to your shell configuration:");
            println!("   {}", format!("export PATH=\"{}:$PATH\"", bin_dir_str).cyan());
        }
    }
    
    Ok(())
}