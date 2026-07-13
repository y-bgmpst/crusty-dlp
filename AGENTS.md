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

## Agent tooling and permissions (policy-driven)

Statt wiederholter Einzelgenehmigungen wird ein repository‑lokales Policy‑Dokument verwendet: `.agent-policy.yaml`. Dieses Dokument definiert vertrauenswürdige Agenten, erlaubte Toolklassen und zulässige Branch‑Muster. Agents, die in der Policy gelistet sind und ein gültiges Preflight‑Manifest vorlegen, dürfen bestimmte, im Policy‑Scope definierte Aktionen ohne weitere interaktive Bestätigung durchführen.

Kurzfassung der Regeln

- Policy‑Datei: `.agent-policy.yaml` im Repo (von Maintainer:innen verwaltet). Sie listet vertrauenswürdige Agenten (ID, Name, public_key), erlaubte Aktionen (z. B. `git_read`, `cargo_build`, `node_local_install`) und erlaubte Branchmuster (z. B. `agent/*`).
- Vorgelegte Manifeste: Jeder Agent muss vor Ausführung ein Preflight‑Manifest (Tools, Versionen, Zweck, Timestamp) vorlegen. Das Manifest muss signiert sein und wird gegen `.agent-policy.yaml` validiert.
- Erlaubte, nicht-interaktive Aktionen (wenn durch Policy abgedeckt):
  - Git‑Leseoperationen: `clone`, `fetch`, `log`, `status`, `diff`.
  - Lokale Builds/Tests innerhalb des Repos: `cargo build`, `cargo check`, `cargo test`, `npm ci` (nur mit Lockfile).
  - Pushes in erlaubte Namensräume, z. B. `agent/*` (nur wenn explizit in der Policy erlaubt).
- Immer genehmigungspflichtig (müssen menschlich freigegeben werden):
  - Pushes auf geschützte Branches (`main`, `release`, `packaging/*`) oder Änderungen an CI/Workflows/Secrets.
  - Veröffentlichung/Publish (z. B. `npm publish`, `cargo publish`).
  - Befehle mit erhöhten Rechten (`sudo`) oder Änderungen an Systempaketen/Hostkonfiguration.
- Audit & Revocation:
  - Alle Agent‑Aktionen werden in einem Audit‑Log dokumentiert (wer, was, wann, Manifest). Policy‑Einträge haben Ablaufdaten (TTL) und können jederzeit per Commit/PR entzogen werden.

Praktische Umsetzungsempfehlungen

- Beginne mit einer kleinen Policy, die nur vertrauenswürdige Agents und das Branch‑Pattern `agent/*` enthält. Erlaube diesen Agents pushes in `agent/*`; humans reviewen PRs gegen `main`.
- Validierung: Agent‑Runner oder Integration prüft Preflight‑Manifest und Signatur vor Ausführung.
- Branch‑Protection: Konfiguriere Branch Protection Rules für `main`/`release` so, dass nur gemergte PRs Änderungen vornehmen können.

Beispiel‑Policy (Beispiel, anpassbar):

```yaml
version: 1
trusted_agents:
  - id: "build-bot"
    name: "build-bot@ci"
    public_key: "ssh-ed25519 AAAA..."
    allowed:
      - git_read
      - cargo_build
      - cargo_check
    allowed_branches:
      - "agent/*"
    expires: "2026-12-31T23:59:59Z"

  - id: "npm-helper"
    name: "npm-helper@runner"
    public_key: "ssh-ed25519 AAAA..."
    allowed:
      - git_read
      - node_local_install   # only with lockfile
    allowed_branches:
      - "agent/*"
    restrictions:
      require_lockfile: true
    expires: "2026-12-31T23:59:59Z"

global_rules:
  require_signed_preflight: true
  audit_log_path: "audit/agent-actions.log"
  protected_branches:
    - "main"
    - "release"
```

Diese Policy reduziert wiederholte Bestätigungen, behält aber Kontrolle über risikoreiche Aktionen und liefert Audit‑Nachweise.

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
