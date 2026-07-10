# codex-bridge-archive

Small local MCP bridge for `crusty-dlp` development. It invokes the installed
Codex CLI without a shell and archives bridge exchanges locally:

```text
~/.local/share/codex-bridge/projects/Codex Bridge/
├── <timestamp>-<session>.jsonl
├── <timestamp>-<session>.md
└── index.json
```

The archive is local-only. Content is redacted before it is written; API keys,
bearer tokens, cookies, and common GitHub/OpenAI token formats are replaced by
`[REDACTED]`. Writes use a temporary file, `fsync`, and `os.replace`.

Run it with the same Python environment that provides `mcp`:

```console
python tools/codex-bridge-archive/server.py
```

Configuration:

- `CODEX_BRIDGE_ARCHIVE_DIR`: override the archive root
- `CODEX_BRIDGE_PROJECT`: default project name
- `CODEX_BRIDGE_SESSION_ID`: stable session identifier
- `CODEX_BRIDGE_MAX_SESSION_BYTES`: default `5242880`
- `CODEX_BRIDGE_MAX_TOTAL_BYTES`: default `104857600`
- `CODEX_BRIDGE_MAX_FILES`: default `200`

The server exposes `consult_codex`, `save_transcript`, and `list_transcripts`.
It does not access ChatGPT account sessions or upload anything to a ChatGPT
Project.

## Optional Notion sync

Create a Notion internal integration, share a parent page with it, and set:

```fish
set -x NOTION_TOKEN "secret_..."
set -x NOTION_PARENT_PAGE_ID "..."
```

When both values are present, each locally saved entry is also appended to a
session page below the shared parent page. The local write happens first and is
always authoritative. A Notion outage is reported in the tool result but does
not discard or roll back the local archive. Tokens are read only from the
environment and are never written to JSONL, Markdown, or `index.json`.
