# public-scripts

Fast-launching command-line tools written in Rust using [scriptr](https://github.com/tekacs/scriptr).

These scripts start in ~5ms, making them feel as snappy as native shell commands.

This README is maintained by AI. I do review the scripts though.

## Installation

```bash
git clone https://github.com/amar/public-scripts
cd public-scripts
./meta/install
```

The install script will:
- Symlink scripts to `~/bin` (or custom directory via `-b`)
- Set up shell completions for your current shell
- Validate any existing symlinks

### Install Options

```bash
./meta/install --help                 # Show all options
./meta/install --dry-run              # Preview what would be installed
./meta/install z                      # Install only specific scripts
./meta/install --shell fish           # Override shell detection
./meta/install --bin-dir ~/.local/bin # Custom install directory
```

## Available Scripts

### `z` - Zellij Session Manager

Enhanced session switcher for [Zellij](https://zellij.dev/) with hash-based identification.

```bash
z              # List all sessions with tabs
z work         # Attach to session by name
z 3f2          # Attach by hash prefix
```

Features:
- 🚀 Instant session listing with tab information
- 🔑 Unique hash prefixes for quick switching
- 📁 Shows working directories and commands per tab
- 🎨 Color-coded current session indicator

## Adding Scripts

Scripts in this repo follow a simple pattern:

```rust
#!/usr/bin/env scriptr
---
[dependencies]
clap = { version = "4.5", features = ["derive"] }
---

use clap::Parser;

fn main() {
    // Your fast-launching CLI tool here
}
```

1. Create an executable script in the repo root
2. Add optional completions in `completions/scriptname.{fish,bash,zsh}`
3. Run `./meta/install` to symlink it

## License

MIT
