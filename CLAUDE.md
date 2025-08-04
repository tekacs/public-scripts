# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Repository Purpose

This is a collection of personal utility scripts written in Rust using scriptr. All scripts are designed to be fast-launching command-line tools that solve specific problems.

## Development Guidelines

### Script Structure
- Scripts are written as single-file Rust programs using scriptr (https://github.com/tekacs/scriptr)
- Scriptr is a wrapper around `cargo -Zscript` that adds intelligent caching to eliminate the ~200ms startup overhead
- Scripts use Rust's new front-matter manifest syntax:
  ```rust
  #!/usr/bin/env scriptr
  ---
  [dependencies]
  clap = { version = "4.5", features = ["derive"] }
  colored = "2"
  ---
  
  // Rust code starts here
  ```
- Scripts use the shebang `#!/usr/bin/env scriptr`
- All scripts must be executable (`chmod +x scriptname`)
- Scripts live in the repository root, not in subdirectories
- Scripts can have any name (no .rs extension required)

### Shell Completions
- Completion files go in `completions/` directory
- Naming convention: `scriptname.shellname` (e.g., `z.fish`, `z.bash`, `z.zsh`)
- Completions are optional but encouraged for better UX

### The Install Script
The `meta/install` script is special - it manages installation of all other scripts:
- Symlinks scripts from this repo to a target directory (default: ~/bin)
- Installs shell completions to appropriate directories
- Validates existing symlinks point to this repo
- Supports selective installation: `./meta/install scriptname`
- Includes dry-run mode: `./meta/install --dry-run`

### Building and Testing
Scripts are compiled on-demand by scriptr, so there's no build process. To test a script:
```bash
./scriptname args
```

First run will be slower as dependencies are compiled. Subsequent runs are cached and fast (~5ms).

### Adding New Scripts
1. Create executable script file in repo root
2. Add scriptr shebang and dependencies front matter
3. Optionally create completion files in `completions/`
4. Run `./meta/install` to symlink to ~/bin