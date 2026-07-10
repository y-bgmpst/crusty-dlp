#!/usr/bin/env python3
"""Local-only MCP bridge with bounded, redacted transcript storage."""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path
from typing import Any

from mcp.server.fastmcp import FastMCP


mcp = FastMCP("codex-bridge-archive")

_SECRET_PATTERNS = (
    re.compile(r"(?i)(authorization\s*:\s*bearer\s+)[^\s,;]+"),
    re.compile(r"(?i)(cookie\s*:\s*)[^\r\n]+"),
    re.compile(r"\b(?:sk|sk-proj)-[A-Za-z0-9_-]{16,}\b"),
    re.compile(r"\bgh[pousr]_[A-Za-z0-9_]{20,}\b"),
    re.compile(r"\bgithub_pat_[A-Za-z0-9_]{20,}\b"),
    re.compile(r"\bxox[baprs]-[A-Za-z0-9-]{16,}\b"),
)
_SECRET_REPLACEMENT = r"\1[REDACTED]"


def redact(value: str) -> str:
    result = value
    for pattern in _SECRET_PATTERNS:
        if pattern.groups == 0:
            result = pattern.sub("[REDACTED]", result)
        else:
            result = pattern.sub(_SECRET_REPLACEMENT, result)
    return result


def _limit(name: str, default: int) -> int:
    try:
        return max(1, int(os.environ.get(name, str(default))))
    except ValueError:
        return default


def _safe_name(value: str, fallback: str) -> str:
    cleaned = re.sub(r"[^A-Za-z0-9._ -]+", "_", value).strip(" .")
    return (cleaned or fallback)[:80]


class ArchiveStore:
    def __init__(self) -> None:
        configured = os.environ.get("CODEX_BRIDGE_ARCHIVE_DIR")
        self.root = Path(configured).expanduser() if configured else (
            Path.home() / ".local" / "share" / "codex-bridge" / "projects"
        )
        self.default_project = os.environ.get("CODEX_BRIDGE_PROJECT", "Codex Bridge")
        self.default_session = os.environ.get("CODEX_BRIDGE_SESSION_ID", str(uuid.uuid4()))
        self.lock = threading.RLock()

    def _project_dir(self, project: str) -> Path:
        return self.root / _safe_name(project, "Codex Bridge")

    def save(self, project: str, session: str, role: str, content: str) -> dict[str, Any]:
        project_name = _safe_name(project, "Codex Bridge")
        session_id = _safe_name(session, str(uuid.uuid4()))
        role_name = _safe_name(role, "note")
        now = time.time()
        record = {"timestamp": now, "session_id": session_id, "role": role_name, "content": redact(content)}
        with self.lock:
            directory = self._project_dir(project_name)
            directory.mkdir(mode=0o700, parents=True, exist_ok=True)
            stem = self._existing_stem(directory, project_name, session_id)
            if stem is None:
                stem = f"{time.strftime('%Y-%m-%dT%H-%M-%SZ', time.gmtime(now))}-{session_id}"
            jsonl_path = directory / f"{stem}.jsonl"
            markdown_path = directory / f"{stem}.md"
            self._atomic_append(jsonl_path, record)
            markdown = markdown_path.read_text(encoding="utf-8") if markdown_path.exists() else f"# {project_name}\n\n- Session: `{session_id}`\n"
            self._atomic_write(markdown_path, f"{markdown}\n## {role_name}\n\n{record['content']}\n")
            self._update_index(directory, project_name, session_id, stem, now)
            self._rotate(directory, jsonl_path)
        return {"project": project_name, "session_id": session_id, "jsonl": str(jsonl_path), "markdown": str(markdown_path)}

    def _existing_stem(self, directory: Path, project: str, session: str) -> str | None:
        for entry in self.list(project):
            if entry.get("session_id") == session and isinstance(entry.get("stem"), str):
                candidate = directory / f"{entry['stem']}.jsonl"
                if candidate.is_file():
                    return entry["stem"]
        return None

    def list(self, project: str) -> list[dict[str, Any]]:
        directory = self._project_dir(project)
        index = directory / "index.json"
        if not index.is_file():
            return []
        try:
            value = json.loads(index.read_text(encoding="utf-8"))
            return value if isinstance(value, list) else []
        except (OSError, json.JSONDecodeError):
            return []

    def _atomic_append(self, path: Path, record: dict[str, Any]) -> None:
        existing = path.read_text(encoding="utf-8") if path.exists() else ""
        payload = existing + json.dumps(record, ensure_ascii=False, sort_keys=True) + "\n"
        if len(payload.encode()) > _limit("CODEX_BRIDGE_MAX_SESSION_BYTES", 5 * 1024 * 1024):
            raise ValueError("transcript session size limit reached")
        self._atomic_write(path, payload)

    @staticmethod
    def _atomic_write(path: Path, content: str) -> None:
        path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
        try:
            with os.fdopen(fd, "w", encoding="utf-8") as handle:
                handle.write(content)
                handle.flush()
                os.fsync(handle.fileno())
            os.replace(temporary, path)
            try:
                directory_fd = os.open(path.parent, os.O_RDONLY)
                try:
                    os.fsync(directory_fd)
                finally:
                    os.close(directory_fd)
            except OSError:
                pass
        finally:
            if os.path.exists(temporary):
                os.unlink(temporary)

    def _update_index(self, directory: Path, project: str, session: str, stem: str, now: float) -> None:
        entries = self.list(project)
        entries = [entry for entry in entries if entry.get("stem") != stem]
        entries.append({"project": project, "session_id": session, "stem": stem, "updated": now})
        entries.sort(key=lambda entry: float(entry.get("updated", 0)), reverse=True)
        self._atomic_write(directory / "index.json", json.dumps(entries[: _limit("CODEX_BRIDGE_MAX_FILES", 200)], indent=2) + "\n")

    def _rotate(self, directory: Path, current: Path) -> None:
        limit = _limit("CODEX_BRIDGE_MAX_TOTAL_BYTES", 100 * 1024 * 1024)
        files = [path for path in directory.glob("*") if path.is_file() and path.name != "index.json"]
        files.sort(key=lambda path: path.stat().st_mtime)
        total = sum(path.stat().st_size for path in files)
        for path in files:
            if total <= limit or path == current:
                continue
            try:
                total -= path.stat().st_size
                path.unlink()
            except OSError:
                pass

    def attach_notion_page(self, project: str, session: str, page_id: str) -> None:
        directory = self._project_dir(project)
        entries = self.list(project)
        changed = False
        for entry in entries:
            if entry.get("session_id") == session:
                entry["notion_page_id"] = page_id
                changed = True
        if changed:
            self._atomic_write(directory / "index.json", json.dumps(entries, indent=2) + "\n")

    def notion_page(self, project: str, session: str) -> str | None:
        for entry in self.list(project):
            if entry.get("session_id") == session:
                page_id = entry.get("notion_page_id")
                return page_id if isinstance(page_id, str) else None
        return None


class NotionSync:
    """Best-effort Notion writer. Local persistence always happens first."""

    endpoint = "https://api.notion.com/v1"
    api_version = "2026-03-11"

    def __init__(self, store: ArchiveStore) -> None:
        self.store = store
        self.token = os.environ.get("NOTION_TOKEN", "").strip()
        self.parent_page_id = os.environ.get("NOTION_PARENT_PAGE_ID", "").strip()

    @property
    def enabled(self) -> bool:
        return bool(self.token and self.parent_page_id)

    def append(self, project: str, session: str, role: str, content: str) -> dict[str, Any]:
        if not self.enabled:
            return {"enabled": False, "synced": False}
        try:
            page_id = self.store.notion_page(project, session)
            if not page_id:
                page_id = self._create_page(project, session)
                self.store.attach_notion_page(project, session, page_id)
            self._append_blocks(page_id, role, redact(content))
            return {"enabled": True, "synced": True, "page_id": page_id}
        except (OSError, ValueError, RuntimeError) as error:
            # The local archive is the source of truth; a cloud outage must not
            # make the MCP call fail after local persistence succeeded.
            return {"enabled": True, "synced": False, "error": str(error)}

    def _request(self, method: str, path: str, payload: dict[str, Any]) -> dict[str, Any]:
        body = json.dumps(payload).encode("utf-8")
        request = urllib.request.Request(
            f"{self.endpoint}{path}",
            data=body,
            method=method,
            headers={
                "Authorization": f"Bearer {self.token}",
                "Content-Type": "application/json",
                "Notion-Version": self.api_version,
            },
        )
        try:
            with urllib.request.urlopen(request, timeout=15) as response:
                parsed = json.loads(response.read().decode("utf-8"))
        except urllib.error.HTTPError as error:
            detail = error.read(512).decode("utf-8", errors="replace")
            raise RuntimeError(f"Notion API returned HTTP {error.code}: {detail}") from error
        except (urllib.error.URLError, TimeoutError, json.JSONDecodeError) as error:
            raise RuntimeError(f"Notion sync request failed: {error}") from error
        if not isinstance(parsed, dict):
            raise RuntimeError("Notion API returned an invalid response")
        return parsed

    def _create_page(self, project: str, session: str) -> str:
        response = self._request(
            "POST",
            "/pages",
            {
                "parent": {"page_id": self.parent_page_id},
                "properties": {
                    "title": {
                        "title": [{"text": {"content": f"{project} · {session}"}}]
                    }
                },
            },
        )
        page_id = response.get("id")
        if not isinstance(page_id, str) or not page_id:
            raise RuntimeError("Notion did not return a page id")
        return page_id

    def _append_blocks(self, page_id: str, role: str, content: str) -> None:
        chunks = [content[index : index + 1800] for index in range(0, len(content), 1800)] or [""]
        blocks = [
            {
                "object": "block",
                "type": "paragraph",
                "paragraph": {
                    "rich_text": [
                        {"type": "text", "text": {"content": f"[{role}] {chunk}"}}
                    ]
                },
            }
            for chunk in chunks
        ]
        self._request("PATCH", f"/blocks/{page_id}/children", {"children": blocks})


STORE = ArchiveStore()
NOTION = NotionSync(STORE)


def save_and_sync(content: str, role: str, project: str, session_id: str) -> dict[str, Any]:
    result = STORE.save(project, session_id, role, content)
    result["notion"] = NOTION.append(project, session_id, role, content)
    return result


@mcp.tool()
def save_transcript(content: str, role: str = "note", project: str = "", session_id: str = "") -> str:
    """Save one redacted local transcript entry and return its paths."""
    project = project or STORE.default_project
    session_id = session_id or STORE.default_session
    return json.dumps(save_and_sync(content, role, project, session_id), ensure_ascii=False)


@mcp.tool()
def list_transcripts(project: str = "") -> str:
    """List archived sessions for a local project."""
    return json.dumps(STORE.list(project or STORE.default_project), ensure_ascii=False)


@mcp.tool()
def consult_codex(query: str, directory: str, format: str = "text", timeout: int = 90) -> str:
    """Ask the local Codex CLI and archive both sides of the exchange."""
    if not Path(directory).is_dir():
        raise ValueError(f"directory does not exist: {directory}")
    codex = shutil.which("codex")
    if not codex:
        raise RuntimeError("Codex CLI not found in PATH")
    session = STORE.default_session
    save_and_sync(query, "user", STORE.default_project, session)
    result = subprocess.run(
        [codex, "exec", query],
        cwd=directory,
        capture_output=True,
        text=True,
        timeout=max(1, min(timeout, 900)),
        shell=False,
    )
    response = result.stdout if result.returncode == 0 else f"Codex CLI Error: {result.stderr.strip()}"
    save_and_sync(response, "assistant", STORE.default_project, session)
    return response if format == "text" else json.dumps({"status": "success" if result.returncode == 0 else "error", "response": response})


def main() -> None:
    mcp.run()


if __name__ == "__main__":
    main()
