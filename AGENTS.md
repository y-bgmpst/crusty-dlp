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
  rate-limits, aria2, cookies, impersonation, network retries, extractor
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
