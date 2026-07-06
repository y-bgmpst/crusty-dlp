# crusty-dlp

`crusty-dlp` is a small terminal interface for managing a sequential `yt-dlp`
download queue on Arch Linux and CachyOS. It invokes programs directly with
argument arrays; URLs, paths, and custom formats are never evaluated by a shell.
See [`COMPATIBILITY.md`](COMPATIBILITY.md) for the supported approach to sites
that require JavaScript, browser sessions, or TLS impersonation.

## Requirements

- `yt-dlp` (required)
- `ffmpeg` (optional, required for audio extraction/MP3 and commonly needed when
  merging the best video and audio streams)
- `python-curl_cffi` (optional, provides browser impersonation and is required
  for BoyfriendTV downloads)
- `deno` (recommended for full YouTube JavaScript challenge support)
- A normal terminal at least 70 columns by 22 rows

Install system dependencies:

```console
sudo pacman -S yt-dlp ffmpeg python-curl_cffi deno
```

Install the Rust toolchain when building from source:

```console
sudo pacman -S rust
```

## Build and install

```console
cargo build --release
./target/release/crusty-dlp
```

To install for the current user:

```console
install -Dm755 target/release/crusty-dlp ~/.local/bin/crusty-dlp
```

Both bash and fish can run the resulting binary; no shell-specific integration
is required. The included `PKGBUILD` is a draft for release packaging. Replace
its placeholder project URL and checksum before using it to publish a package.

### Windows

GitHub Releases provide two Windows assets:

- `crusty-dlp.exe` for users who already have `yt-dlp.exe` in `PATH` or beside
  the application.
- `crusty-dlp-windows-x86_64.zip`, containing crusty-dlp and a checksum-verified
  official `yt-dlp.exe`. Extract both files into one folder and run
  `crusty-dlp.exe` from Windows Terminal.

Install `ffmpeg` separately when using conversion or stream merging. Full
YouTube support also needs Deno; install it with:

```powershell
winget install DenoLand.Deno
```

Every push is tested natively on Linux and Windows. Version tags matching `v*`
build and attach the EXE and ZIP to a GitHub Release automatically.

## Usage

Start with an empty queue:

```console
crusty-dlp
```

Preload one or more URLs:

```console
crusty-dlp 'https://example.com/video/1' 'https://example.com/video/2'
```

Inspect the precise executable and argument boundaries without launching
`yt-dlp`. Each argument is quoted separately and the command is printed without
starting the TUI, so this output can be redirected:

```console
crusty-dlp --dry-run 'https://example.com/video/1'
```

Use `--debug` to show diagnostic context such as the active configuration path.
Run `crusty-dlp --help` for CLI details.

## Keyboard shortcuts

| Key | Action |
| --- | --- |
| `q` | Quit and cancel the active child safely |
| `a` | Add one or more whitespace-separated URLs |
| `d` | Start or continue the queue |
| `c` | Cancel the active download |
| `b` | Cycle the browser used for session cookies |
| `Tab` | Switch panels |
| `Enter` / `Space` | Edit or change the selected panel |
| `Esc` | Cancel editing |
| `?` | Show help |

Downloads run sequentially. After a successful item, the next waiting item
starts automatically. Failed or cancelled entries remain visible in the queue.

### Browser impersonation

Use `Tab` to select the **Impersonation** panel and `Enter` or `Space` to cycle
through targets reported by `yt-dlp --list-impersonate-targets`. `None` is the
fastest and most stable default. `Any available` lets yt-dlp choose a target.
Forcing impersonation can reduce download speed or stability, so enable it only
for sites that need it.

Press `b` to optionally let yt-dlp read cookies from a locally installed
browser. This helps with sites that require the same authorized session used in
the browser, including some YouTube bot checks and anti-bot protected sites.
crusty-dlp stores only the browser name, never cookie contents. Close the browser
first if its cookie database is locked.

If no target is installed, crusty-dlp displays the correct Arch/CachyOS command:

```console
sudo pacman -S python-curl_cffi
```

### BoyfriendTV

BoyfriendTV video-page URLs are recognized directly. When impersonation is set
to `None`, crusty-dlp automatically asks yt-dlp's generic extractor to use any
available target for that URL. A bundled yt-dlp extractor plugin reads the
page's public media source list and supports direct files and HLS manifests.
This requires `python-curl_cffi`; the application shows an actionable error if
it is unavailable. Support is independently implemented and does not copy code
from the unlicensed third-party userscript.

## Configuration

Configuration follows the Linux XDG base directory convention and normally
lives at:

```text
~/.config/crusty-dlp/config.toml
```

If `XDG_CONFIG_HOME` is set, it is used instead. The app creates the file after
changing and saving a configurable value. See [`config.example.toml`](config.example.toml).

```toml
output_dir = "/home/alice/Downloads"
default_mode = "video"
custom_format = "bestvideo+bestaudio/best"
impersonation = "none"
cookies_browser = "none"
```

## Troubleshooting

**`yt-dlp was not found in PATH`** — install it with `pacman -S yt-dlp`, or
ensure the directory containing the executable is present in `PATH` before
starting the app.

**`ffmpeg is required`** — install it with `pacman -S ffmpeg`. MP3 and audio
extraction require it.

**No impersonation targets are available** — install support with
`sudo pacman -S python-curl_cffi`, then restart crusty-dlp.

**A site reports an authentication or extractor error** — first update the
system package (`sudo pacman -Syu`). This app deliberately does not handle
passwords, exported cookie files, DRM, or access-control bypasses. For content
you can already access in a local browser session, press `b` to select that
browser and retry.

**The terminal looks corrupted after an abnormal termination** — run `reset`.
Normal errors and exits restore the terminal automatically.

## Development

```console
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

The architecture keeps state/rendering, keyboard input, configuration, and
process management in separate modules. A Tokio task owns the active child;
channels carry progress and cancellation events back to the UI loop.

## Legal and responsible use

Download only content you own or have permission and a legal right to download.
You are responsible for following applicable laws and the terms of the services
you use. This project does not bypass DRM or platform access controls.

## License

MIT. See [`LICENSE`](LICENSE).
