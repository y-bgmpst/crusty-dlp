# AGENTS.md — crusty-dlp

## Goal

crusty-dlp is a local, resource-efficient TUI/GUI downloader built around
`yt-dlp`. Keep it secure, maintainable, Linux-native, and suitable for lawful
downloads without bypassing DRM or access controls.

## Architecture boundaries

- Rust owns core logic, configuration, validation, queue management, process
  control, the TUI, the egui/eframe GUI, and packaging integration.
- `yt-dlp`, `ffmpeg`, `aria2c`, and optional extractor plugins remain external
  processes/extensions. Never construct shell commands from input strings.
- Pass process arguments only with `Command::arg`/`args`.
- The TUI and GUI share the same core, downloader, and configuration logic.
- Playlist inspection, thumbnail downloads, and diagnostics must not block the
  UI thread.
- Do not load large media or unbounded process output into memory; use streams,
  explicit limits, and bounded queues.

## Security and privacy rules

- Treat URLs, paths, templates, extractor arguments, and configuration as
  untrusted input.
- Never invoke a shell or use `sh -c`, `cmd /C`, or string-concatenated
  commands for external processes.
- Production plugins may only come from trusted installation paths; repository
  plugins are allowed only in debug builds.
- Resolve executables through absolute, trusted paths; reject relative or empty
  `PATH` entries.
- Do not log cookies, tokens, or complete URL query parameters.
- Do not bypass DRM, access controls, or store credentials.
- Downloads must not modify existing files without an explicit user option.
  Overwrite behavior, metadata writing, and playlist subfolders must remain
  opt-in settings.
- Treat plugin and thumbnail downloads as untrusted external data; keep byte,
  decode, and network limits in place.
- Cancellation must terminate the complete process tree and leave no orphaned
  `yt-dlp`, `ffmpeg`, or `aria2c` processes.

## Agent tooling and permissions (updated)

- Any automated agent must list the tools it intends to use and request explicit permission before using network-capable tools or system-level commands that can modify the host system (for example: `sudo`, package managers that install system packages, or any command that changes CI/workflows or system configuration).
- Allowed without prior permission (subject to the constraints below):
  - Git read operations: `clone`, `fetch`, `log`, `status`, `diff`. Local git operations inside the repository (creating branches, local commits) are allowed for drafting; pushing, opening PRs, or modifying remote branches still require explicit human authorization.
  - NodeJS tooling limited to the project: running `npx`, executing project-local npm scripts, and installing packages locally within the repository (e.g., `npm ci`, `npm install` into the project workspace) — provided a lockfile (`package-lock.json` / `yarn.lock` / `pnpm-lock.yaml`) is present and used.
  - Rust tooling limited to the project: `cargo build`, `cargo check`, `cargo test`, and other local cargo commands that operate on the repository.
- Additional safety constraints for allowed operations:
  - No global/system package installs (no `-g`, no system-wide npm/pip installs) and no modification of system PATH or system package databases without explicit permission.
  - Prefer lockfile-driven installs (`npm ci`, `cargo build --locked`), offline caches, and verified registries. If a lockfile is missing or network use would fetch unverified code, the agent must request permission.
  - Disallow running package-install lifecycle scripts or arbitrary remote install-and-execute steps unless explicitly authorized; prefer `--ignore-scripts` or other mitigations when feasible.
  - Any network fetch that brings external code or binaries into the repo must include integrity verification (lockfile, checksum, or signature) and must be reported in the preflight tool list.
- Explicit-permission actions (must ask before proceeding):
  - Any command requiring elevated privileges (`sudo`, package managers that modify system packages like `pacman`, `apt`, `dnf`, `zypper`), publishing packages to external registries, adding system services, or changing CI/workflow files or secrets.
  - Any operation that will publish or distribute artifacts (e.g., `npm publish`, `cargo publish`, uploading releases) or modify remote repositories (push/merge) without prior maintainer approval.
- Reporting requirement: The agent's preflight tool list must clearly name intended commands (e.g., `git clone`, `npm ci`, `cargo build`), why they are needed, whether they access the network, and what files/configs (lockfiles, build scripts) they rely on.

## Error handling

- Represent expected failures with `Result`/`Option` and surface clear messages
  in the UI/TUI; do not use `unwrap`/`expect` to handle user input.
- Bound error output, prioritize useful `ERROR:` lines, and avoid secrets.
- Malformed URLs, configuration, playlist data, and extractor output must not
  crash the application.

## Context groups

For token-efficient reviews, load only the smallest relevant group:

```text
Core:     @Cargo.toml @src/lib.rs @src/app.rs @src/downloader.rs @src/config.rs @src/errors.rs
GUI:      @src/bin/gui.rs @src/search.rs @src/config.rs @src/downloader.rs
TUI:      @src/main.rs @src/input.rs @src/ui.rs @src/app.rs
Security: @src/downloader.rs @src/config.rs @src/app.rs @src/bin/gui.rs
Release:  @Cargo.toml @build.rs @PKGBUILD @.github/workflows/ci.yml @.github/workflows/release.yml @packaging/verify-linux-packages.sh @assets/crusty-dlp.desktop
```

## Quality gates

Run these before declaring a change complete:

```fish
cargo fmt --check
cargo check --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --locked
python -m compileall -q plugins
cargo build --release --locked
```

For packaging or release changes, also run the Linux package verification and
check the affected GitHub Actions workflow. New validators, argument builders,
playlist parsers, process-cancellation paths, configuration fields, and UI
states require unit or integration tests.

## Current feature set

- TUI and egui/eframe GUI with queue management, parallel downloads,
  pause/resume/cancel, dry-run, and debug output.
- Shared yt-dlp argument construction for formats, output templates,
  rate-limits, aria, cookies, impersonation, network retries, extractor
  arguments, and metadata.
- Supported playlist expansion with deduplication, titles/thumbnails, and safe
  playlist subfolders.
- Asynchronous diagnostics for yt-dlp, ffmpeg, yt-dlp-ejs, JavaScript runtimes,
  and plugins.
- Persistent thumbnail cache with size and image-decode limits.
- Linux, Windows, and macOS builds plus DEB/RPM/Arch/Gentoo/Nix packaging paths.

## Development workflow

1. Identify the architecture boundary and relevant context group before editing.
2. Make small, reversible changes with tests and understandable errors.
3. Do not add silent fallbacks for supported playlists or security checks.
4. Run local quality gates, then verify CI and packaging when relevant.
5. Write commit messages and README changes in terms of user impact.

## Recommended next improvements

1. Replace unbounded playlist/config reads with bounded streaming reads.
2. Add thumbnail SSRF protection with redirect and private-IP checks.
3. Redact URLs and sensitive values in logs and debug commands.
4. Use stable queue IDs and further virtualization for very large playlists.
5. Expand package smoke tests for desktop files, plugin paths, and window IDs.

---

## Included from: https://github.com/actionbook/rust-skills/blob/main/AGENTS.md

# Rust Skills - Agent Instructions

> For OpenAI Codex and compatible agents

## Default Project Settings

When creating Rust projects or Cargo.toml files, ALWAYS use:

```toml
[package]
edition = "2024"
rust-version = "1.85"

[lints.rust]
unsafe_code = "warn"

[lints.clippy]
all = "warn"
pedantic = "warn"
```

## Core Capabilities

### 1. Question Routing
Route Rust questions to appropriate skills:
- Ownership/borrowing → m01-ownership
- Smart pointers → m02-resource
- Error handling → m06-error-handling
- Concurrency → m07-concurrency
- Unsafe code → unsafe-checker

### 2. Code Style
Follow Rust coding guidelines:
- Use snake_case for variables and functions
- Use PascalCase for types and traits
- Use SCREAMING_SNAKE_CASE for constants
- Max line length: 100 characters
- Use `?` operator instead of `unwrap()` in library code

### 3. Error Handling
```rust
// Good: Use Result with context
fn read_config() -> Result<Config, ConfigError> {
    let content = std::fs::read_to_string("config.toml")
        .map_err(|e| ConfigError::Io(e))?;
    toml::from_str(&content)
        .map_err(|e| ConfigError::Parse(e))
}

// Avoid: unwrap() in library code
fn read_config() -> Config {
    let content = std::fs::read_to_string("config.toml").unwrap(); // Bad
    toml::from_str(&content).unwrap() // Bad
}
```

### 4. Unsafe Code
Every `unsafe` block MUST have a `// SAFETY:` comment:
```rust
// SAFETY: We checked that index < len above, so this is in bounds
unsafe { slice.get_unchecked(index) }
```

### 5. Common Error Fixes

| Error | Cause | Fix |
|-------|-------|-----|
| E0382 | Use of moved value | Clone, borrow, or use reference |
| E0597 | Lifetime too short | Extend lifetime or restructure |
| E0502 | Borrow conflict | Split borrows or use RefCell |
| E0499 | Multiple mut borrows | Restructure to single mut borrow |
| E0277 | Missing trait impl | Add trait bound or implement trait |

## Quick Reference

### Ownership
- Each value has one owner
- Borrowing: `&T` (shared) or `&mut T` (exclusive)
- Lifetimes: `'a` annotations for references

### Smart Pointers
- `Box<T>`: Heap allocation
- `Rc<T>`: Reference counting (single-threaded)
- `Arc<T>`: Atomic reference counting (thread-safe)
- `RefCell<T>`: Interior mutability

### Concurrency
- `Send`: Safe to transfer between threads
- `Sync`: Safe to share references between threads
- `Mutex<T>`: Mutual exclusion
- `RwLock<T>`: Reader-writer lock

### Async
```rust
#[tokio::main]
async fn main() {
    let handle = tokio::spawn(async {
        // async work
    });
    handle.await.unwrap();
}
```

## Skill Files

For detailed guidance, see:
- `skills/rust-router/SKILL.md` - Question routing
- `skills/coding-guidelines/SKILL.md` - Code style rules
- `skills/unsafe-checker/SKILL.md` - Unsafe code review
- `skills/m01-ownership/SKILL.md` - Ownership concepts
- `skills/m06-error-handling/SKILL.md` - Error patterns
- `skills/m07-concurrency/SKILL.md` - Concurrency patterns
