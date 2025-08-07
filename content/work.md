+++
title = "Work"
description = "Open source projects and tools I'm building"
+++

## Projects

### surfacer
**Comprehensive Rust crate analysis tool**

Surfaces API documentation, examples, and real-world usage patterns from the Rust ecosystem without building any code. Unlike rustdoc which shows what's possible, surfacer reveals what's practical by analyzing how crates are actually used in reverse dependencies.

- [GitHub Repository](https://github.com/tekacs/surfacer)
- [Installation](https://github.com/tekacs/surfacer#installation) - `cargo install --path .`

---

### librp
**LLM-powered roleplaying library**

A Rust library that brings LLM roleplaying capabilities to any application. Features character card parsing, context management, multi-provider support via genai, and an optional TUI interface for interactive sessions.

- [GitHub Repository](https://github.com/tekacs/librp)
- [Documentation](https://docs.rs/librp) (coming soon)

---

### additive
**CRDT exploration project** *(early stage)*

Experimental project exploring Conflict-free Replicated Data Types (CRDTs) with both Automerge and Y-CRDT implementations. Currently in early development stage.

- GitHub Repository (coming soon)
- Status: Research phase

---

### thing
**Fly.io-style CLI for Kubernetes**

Brings the simplicity of Fly's Machines API to Kubernetes. Provides imperative, single-command operations for managing containers with automatic scaling, volumes, and networking - all using native Kubernetes primitives under the hood.

- [GitHub Repository](https://github.com/tekacs/thing)
- [Architecture Plans](https://github.com/tekacs/thing/blob/main/plan.revised.md)

---

### factor
**Full-stack Clojure(Script) framework**

Opinionated full-stack framework combining Clojure backend (Integrant, Pathom3, XTDB) with ClojureScript frontend (Shadow-cljs, React). Includes example client/server setup with hot reloading and REPL-driven development.

- [GitHub Repository](https://github.com/tekacs/factor.git)
- [Quick Start](https://github.com/tekacs/factor.git#example)

---

### bevy-wasm-tasks
**Async task integration for Bevy**

A Bevy plugin that enables running futures (including `!Send` futures) in Bevy applications. Essential for WASM compatibility where traditional threading isn't available. Originally based on bevy-tokio-tasks but heavily adapted.

- [GitHub Repository](https://github.com/tekacs/bevy-wasm-tasks.git)
- [crates.io](https://crates.io/crates/bevy-wasm-tasks)
- [Documentation](https://docs.rs/bevy-wasm-tasks/latest/bevy_wasm_tasks/)

---

### casual-ai
**Zero-configuration AI library** *(in development)*

Makes adding AI capabilities to any project trivially easy. Automatically selects the best available model, handles provider fallback, and abstracts away API complexity. Planned support for Rust, Python, Swift, Kotlin, TypeScript, and Ruby via UniFFI.

- GitHub Repository (coming soon)
- [Project Plan](https://github.com/tekacs/casual-ai/blob/main/plans/00-project-summary.md)

---

### scriptr
**Fast Rust script launcher**

Cuts startup time for Rust single-file packages from 200ms to 5ms through intelligent caching. Perfect for command-line tools where startup latency matters. Works seamlessly with cargo's `-Zscript` feature while adding a smart caching layer.

- [GitHub Repository](https://github.com/tekacs/scriptr)
- [crates.io](https://crates.io/crates/scriptr) - `cargo install scriptr`

---

### public-scripts
**Collection of fast CLI tools**

Fast-launching command-line utilities written in Rust using scriptr. Currently features an enhanced Zellij session manager with hash-based identification for quick switching. All tools start in ~5ms for instant responsiveness.

- [GitHub Repository](https://github.com/tekacs/public-scripts)
- [Install Scripts](https://github.com/tekacs/public-scripts#installation)