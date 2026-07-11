# crusty-dlp context groups

Use these `@` references to keep reviews focused and reduce context size.

## Core / downloader

```text
@Cargo.toml
@src/lib.rs
@src/app.rs
@src/downloader.rs
@src/config.rs
@src/errors.rs
```

## GUI

```text
@src/bin/gui.rs
@src/search.rs
@src/config.rs
@src/downloader.rs
```

## TUI

```text
@src/main.rs
@src/input.rs
@src/ui.rs
@src/app.rs
```

## Security

```text
@src/downloader.rs
@src/config.rs
@src/app.rs
@src/bin/gui.rs
```

## Packaging / release

```text
@Cargo.toml
@build.rs
@PKGBUILD
@.github/workflows/ci.yml
@.github/workflows/release.yml
@packaging/verify-linux-packages.sh
@assets/crusty-dlp.desktop
```

## Minimal everyday context

```text
@Cargo.toml
@src/lib.rs
@src/app.rs
@src/downloader.rs
```

## Review guidance

- Add only the smallest relevant group to a task.
- Include `@src/bin/gui.rs` for egui, clipboard, thumbnails, diagnostics,
  queue rendering, or desktop integration changes.
- Include `@src/downloader.rs` for yt-dlp arguments, plugins, playlists,
  cancellation, process execution, or security reviews.
- Include `@src/config.rs` for persisted settings or migration changes.
- Include packaging files only for CI, desktop-entry, installer, or release work.
- Treat this file as a human-readable context manifest; it does not execute or
  load files automatically.
