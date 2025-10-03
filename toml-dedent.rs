#!/usr/bin/env scriptr
---
[dependencies]
anyhow = "1.0"
clap = { version = "4.5", features = ["derive"] }
walkdir = "2.5"
toml_edit = "0.22"
unindent = "0.2"
similar = "2.6"
---
use anyhow::{Context, Result};
use clap::Parser;
use std::fs;
use std::path::{Path, PathBuf};
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, Value};
use walkdir::WalkDir;

#[derive(Parser, Debug)]
#[command(name = "toml-dedent", about = "Dedent TOML multi-line string values safely")]
struct Args {
    /// Files or directories to process
    #[arg(default_value = ".")]
    paths: Vec<PathBuf>,

    /// Write changes in-place (default is check-only)
    #[arg(long)]
    write: bool,

    /// Create .bak alongside modified files (only with --write)
    #[arg(long)]
    backup: bool,

    /// Also trim trailing spaces/tabs on each line (off by default)
    #[arg(long)]
    trim_trailing: bool,

    /// Strip leading/trailing completely blank lines
    #[arg(long)]
    strip_blank_edges: bool,

    /// Show unified diffs for changed files (check mode)
    #[arg(long)]
    diff: bool,

    /// Apply Markdown-aware cleanup inside strings (lists, headings, blank lines)
    #[arg(long)]
    normalize_markdown: bool,
}

#[derive(Clone, Copy)]
struct Options {
    trim_trailing: bool,
    strip_blank_edges: bool,
    normalize_markdown: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let opts = Options {
        trim_trailing: args.trim_trailing,
        strip_blank_edges: args.strip_blank_edges,
        normalize_markdown: args.normalize_markdown,
    };

    let mut changed = 0usize;
    let mut visited = 0usize;
    let mut errors = 0usize;

    let files = collect_toml_files(&args.paths)?;

    for path in files {
        visited += 1;
        match process_file(&path, opts, args.write, args.backup, args.diff) {
            Ok(did_change) => {
                if did_change {
                    changed += 1;
                }
            }
            Err(e) => {
                errors += 1;
                eprintln!("error: {}: {:#}", path.display(), e);
            }
        }
    }

    if !args.write && changed > 0 {
        eprintln!(
            "Would modify {} of {} TOML files ({} errors).",
            changed, visited, errors
        );
        std::process::exit(1);
    }

    eprintln!(
        "{} {} of {} TOML files ({} errors).",
        if args.write { "Modified" } else { "Checked" },
        changed,
        visited,
        errors
    );
    Ok(())
}

fn collect_toml_files(inputs: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for input in inputs {
        let md = fs::metadata(input)
            .with_context(|| format!("stat {}", input.display()))?;
        if md.is_file() {
            if is_toml(input) {
                out.push(input.clone());
            }
            continue;
        }
        for entry in WalkDir::new(input).follow_links(false) {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("warn: skipping entry: {}", e);
                    continue;
                }
            };
            if !entry.file_type().is_file() {
                continue;
            }
            let p = entry.into_path();
            if is_toml(&p) {
                out.push(p);
            }
        }
    }
    Ok(out)
}

fn is_toml(p: &Path) -> bool {
    p.extension().map(|e| e == "toml").unwrap_or(false)
}

fn process_file(path: &Path, opts: Options, write: bool, backup: bool, show_diff: bool) -> Result<bool> {
    let original = fs::read_to_string(path)
        .with_context(|| format!("read {}", path.display()))?;
    let mut doc = original.parse::<DocumentMut>()
        .with_context(|| format!("parse {}", path.display()))?;

    let mut mutated = false;
    visit_item(doc.as_item_mut(), opts, &mut mutated);

    if !mutated {
        return Ok(false);
    }

    let updated = doc.to_string();
    if !write {
        if show_diff {
            print_diff(path, &original, &updated);
        } else {
            println!("{}", path.display());
        }
        return Ok(true);
    }

    if backup {
        let mut bak = path.to_path_buf();
        let old_ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        bak.set_extension(format!("{}.bak", old_ext));
        fs::write(&bak, &original)
            .with_context(|| format!("write backup {}", bak.display()))?;
    }

    fs::write(path, updated)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(true)
}

fn visit_item(item: &mut Item, opts: Options, mutated: &mut bool) {
    match item {
        Item::Value(v) => visit_value(v, opts, mutated),
        Item::Table(t) => visit_table(t, opts, mutated),
        Item::ArrayOfTables(aot) => visit_aot(aot, opts, mutated),
        _ => {}
    }
}

fn visit_table(table: &mut Table, opts: Options, mutated: &mut bool) {
    for (_, it) in table.iter_mut() {
        visit_item(it, opts, mutated);
    }
}

fn visit_aot(aot: &mut ArrayOfTables, opts: Options, mutated: &mut bool) {
    for tbl in aot.iter_mut() {
        visit_table(tbl, opts, mutated);
    }
}

fn visit_value(value: &mut Value, opts: Options, mutated: &mut bool) {
    match value {
        Value::Array(arr) => {
            for v in arr.iter_mut() {
                visit_value(v, opts, mutated);
            }
        }
        Value::InlineTable(tbl) => {
            for (_, v) in tbl.iter_mut() {
                visit_value(v, opts, mutated);
            }
        }
        Value::String(_s) => {
            if let Some(old) = value.as_str() {
                if old.contains('\n') {
                    let new = transform_multiline(old, opts);
                    if new != old {
                        *value = Value::from(new);
                        *mutated = true;
                    }
                }
            }
        }
        _ => {}
    }
}

fn transform_multiline(input: &str, opts: Options) -> String {
    // Normalize line endings
    let mut s = input.replace("\r\n", "\n").replace('\r', "\n");

    // Dedent uniformly using unindent; preserves relative indentation
    s = unindent::unindent(&s);

    // Optional Markdown-aware normalization (outside fenced code blocks)
    if opts.normalize_markdown {
        s = normalize_markdown(&s);
    }

    // Optionally strip leading/trailing completely blank lines
    if opts.strip_blank_edges {
        s = strip_blank_edges(&s);
    }

    // Optionally trim trailing spaces/tabs per line
    if opts.trim_trailing {
        let mut out = String::with_capacity(s.len());
        for (i, line) in s.lines().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(line.trim_end_matches(|c| c == ' ' || c == '\t'));
        }
        s = out;
    }

    s
}

fn strip_blank_edges(s: &str) -> String {
    let mut lines: Vec<&str> = s.split('\n').collect();
    while lines.first().is_some_and(|l| l.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

fn print_diff(path: &Path, old: &str, new: &str) {
    use similar::{ChangeTag, TextDiff};

    let diff = TextDiff::from_lines(old, new);
    println!("--- {}", path.display());
    println!("+++ {}", path.display());
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Delete => print!("-"),
            ChangeTag::Insert => print!("+"),
            ChangeTag::Equal => print!(" "),
        }
        print!("{}", change);
    }
}

fn normalize_markdown(s: &str) -> String {
    // Walk lines, preserve fenced code blocks verbatim, and normalize
    // - list item spacing
    // - heading spacing
    // - collapse multiple blank lines to a single blank (outside fences)

    let mut out = String::with_capacity(s.len());
    let mut in_fence = false;
    let mut fence_marker: Option<&str> = None; // "```" or "~~~"
    let mut last_was_blank = false;

    for raw_line in s.split('\n') {
        let trimmed_start = raw_line.trim_start();
        let is_fence_opener = (trimmed_start.starts_with("```") || trimmed_start.starts_with("~~~"))
            && !in_fence;
        let is_fence_closer = in_fence
            && fence_marker
                .map(|m| trimmed_start.starts_with(m))
                .unwrap_or(false);

        if is_fence_opener {
            // Enter fence; record marker
            fence_marker = Some(if trimmed_start.starts_with("```") { "```" } else { "~~~" });
            in_fence = true;
            last_was_blank = false; // keep fences intact
            if !out.is_empty() { out.push('\n'); }
            out.push_str(raw_line);
            continue;
        }

        if is_fence_closer {
            in_fence = false;
            fence_marker = None;
            if !out.is_empty() { out.push('\n'); }
            out.push_str(raw_line);
            continue;
        }

        if in_fence {
            // Pass-through inside fences
            if !out.is_empty() { out.push('\n'); }
            out.push_str(raw_line);
            continue;
        }

        // Outside fences: normalize
        let mut line = raw_line.to_string();

        // Heading spacing: ensure one space after leading hashes
        if let Some(hash_prefix_len) = heading_hash_prefix(&line) {
            // Replace any run of spaces/tabs after hashes with a single space (if non-empty content)
            let (prefix, rest) = line.split_at(hash_prefix_len);
            let rest_trim = rest.trim_start_matches([' ', '\t']);
            if rest_trim.is_empty() {
                // Keep as is if no heading text
                line = prefix.to_string();
            } else {
                line = format!("{} {}", prefix, rest_trim);
            }
        }

        // List item normalization: bullets (- * +) and ordered (1. 1)
        // - collapse indent before marker to 0 (top-level) or 2 spaces (nested)
        // - ensure single space after marker
        if let Some((indent, marker, after)) = parse_list_marker(&line) {
            let text = after.trim_start_matches([' ', '\t']);
            // normalize indent: any non-zero indent -> two spaces
            let desired_indent = if indent.is_empty() { "" } else { "  " };
            line = if text.is_empty() {
                format!("{}{}", desired_indent, marker) // keep bare marker line
            } else {
                format!("{}{} {}", desired_indent, marker, text)
            };
        } else {
            // Non-list, non-heading lines: collapse 3+ internal spaces to a single space
            line = collapse_internal_3plus_spaces_preserve_eol(&line);
        }

        // Collapse multiple blank lines to single blank
        let is_blank = line.trim().is_empty();
        if is_blank {
            if last_was_blank {
                // skip extra blank
                continue;
            }
            last_was_blank = true;
        } else {
            last_was_blank = false;
        }

        if !out.is_empty() { out.push('\n'); }
        out.push_str(&line);
    }

    out
}

fn heading_hash_prefix(line: &str) -> Option<usize> {
    // return length of leading ###... up to 6, only if line starts with '#'
    let bytes = line.as_bytes();
    if bytes.first().copied() != Some(b'#') {
        return None;
    }
    let mut n = 0usize;
    for &b in bytes.iter() {
        if b == b'#' && n < 6 { n += 1; } else { break; }
    }
    Some(n)
}

fn parse_list_marker(line: &str) -> Option<(&str, &str, &str)> {
    // Matches leading indent + marker ("-", "*", "+", "1.", "1)") + following content
    let bytes = line.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') { i += 1; }
    let indent = &line[..i];
    if i >= bytes.len() { return None; }
    let b = bytes[i];
    if b == b'-' || b == b'*' || b == b'+' {
        let marker = &line[i..=i];
        let rest = &line[(i+1)..];
        return Some((indent, marker, rest));
    }
    // Ordered list: digits then '.' or ')'
    let mut j = i;
    let mut saw_digit = false;
    while j < bytes.len() && bytes[j].is_ascii_digit() { j += 1; saw_digit = true; }
    if saw_digit && j < bytes.len() && (bytes[j] == b'.' || bytes[j] == b')') {
        let marker = &line[i..=j];
        let rest = &line[(j+1)..];
        return Some((indent, marker, rest));
    }
    None
}

fn collapse_internal_3plus_spaces_preserve_eol(line: &str) -> String {
    // Preserve up to two trailing spaces (Markdown hard line break),
    // collapse runs of 3+ spaces inside the line (outside the trailing run).
    let mut end = line.len();
    let bytes = line.as_bytes();
    while end > 0 && (bytes[end - 1] == b' ' || bytes[end - 1] == b'\t') {
        end -= 1;
    }
    let head = &line[..end];
    let tail = &line[end..];

    let mut out = String::with_capacity(line.len());
    let mut space_run = 0usize;
    for ch in head.chars() {
        if ch == ' ' {
            space_run += 1;
        } else {
            if space_run > 0 {
                if space_run >= 3 {
                    out.push(' ');
                } else {
                    for _ in 0..space_run { out.push(' '); }
                }
                space_run = 0;
            }
            out.push(ch);
        }
    }
    if space_run > 0 {
        if space_run >= 3 {
            out.push(' ');
        } else {
            for _ in 0..space_run { out.push(' '); }
        }
    }
    out.push_str(tail);
    out
}
