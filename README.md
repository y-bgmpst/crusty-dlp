# crusty-dlp

`crusty-dlp` is a small terminal interface for managing a sequential `yt-dlp`
download queue on Arch Linux and CachyOS. It invokes programs directly with
argument arrays; URLs, paths, and custom formats are never evaluated by a shell.

## Requirements

- `yt-dlp` (required)
- `ffmpeg` (optional, required for audio extraction/MP3 and commonly needed when
  merging the best video and audio streams)
- A normal terminal at least 70 columns by 22 rows

Install system dependencies:

```console
sudo pacman -S yt-dlp ffmpeg
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
| `Tab` | Switch panels |
| `Enter` / `Space` | Edit or change the selected panel |
| `Esc` | Cancel editing |
| `?` | Show help |

Downloads run sequentially. After a successful item, the next waiting item
starts automatically. Failed or cancelled entries remain visible in the queue.

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
```

## Troubleshooting

**`yt-dlp was not found in PATH`** — install it with `pacman -S yt-dlp`, or
ensure the directory containing the executable is present in `PATH` before
starting the app.

**`ffmpeg is required`** — install it with `pacman -S ffmpeg`. MP3 and audio
extraction require it.

**A site reports an authentication or extractor error** — first update the
system package (`sudo pacman -Syu`). This app deliberately does not handle
credentials, cookies, DRM, or access-control bypasses.

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
