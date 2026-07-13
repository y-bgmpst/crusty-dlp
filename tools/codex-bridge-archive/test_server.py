import json
import os
import tempfile
import unittest
from pathlib import Path

from server import ArchiveStore, NotionSync, redact


class ArchiveTests(unittest.TestCase):
    def test_redacts_common_secrets(self):
        value = redact("Authorization: Bearer secret Cookie: sid=abc sk-test_1234567890123456 gho_12345678901234567890")
        self.assertNotIn("secret", value)
        self.assertNotIn("sid=abc", value)
        self.assertNotIn("gho_12345678901234567890", value)

    def test_atomic_archive_and_index(self):
        with tempfile.TemporaryDirectory() as directory:
            os.environ["CODEX_BRIDGE_ARCHIVE_DIR"] = directory
            store = ArchiveStore()
            result = store.save("Codex Bridge", "session-1", "user", "hello")
            self.assertTrue(Path(result["jsonl"]).is_file())
            self.assertTrue(Path(result["markdown"]).is_file())
            entries = store.list("Codex Bridge")
            self.assertEqual(len(entries), 1)
            record = json.loads(Path(result["jsonl"]).read_text().splitlines()[0])
            self.assertEqual(record["content"], "hello")
        os.environ.pop("CODEX_BRIDGE_ARCHIVE_DIR", None)

    def test_notion_is_opt_in(self):
        with tempfile.TemporaryDirectory() as directory:
            os.environ["CODEX_BRIDGE_ARCHIVE_DIR"] = directory
            store = ArchiveStore()
            result = NotionSync(store).append("Codex Bridge", "session-1", "user", "hello")
            self.assertFalse(result["enabled"])
        os.environ.pop("CODEX_BRIDGE_ARCHIVE_DIR", None)

    def test_notion_sync_uses_redacted_chunks(self):
        with tempfile.TemporaryDirectory() as directory:
            os.environ["CODEX_BRIDGE_ARCHIVE_DIR"] = directory
            store = ArchiveStore()
            store.save("Codex Bridge", "session-1", "user", "seed")
            sync = NotionSync(store)
            sync.token = "token"
            sync.parent_page_id = "parent"
            requests = []

            def fake_request(method, path, payload):
                requests.append((method, path, payload))
                return {"id": "page-1"} if method == "POST" else {}

            sync._request = fake_request
            result = sync.append("Codex Bridge", "session-1", "assistant", "gho_12345678901234567890")
            self.assertTrue(result["synced"])
            self.assertEqual(requests[0][1], "/pages")
            self.assertEqual(requests[1][1], "/blocks/page-1/children")
            self.assertIn("[REDACTED]", requests[1][2]["children"][0]["paragraph"]["rich_text"][0]["text"]["content"])
        os.environ.pop("CODEX_BRIDGE_ARCHIVE_DIR", None)

    def test_consult_gemini_archiving(self):
        from unittest.mock import patch, MagicMock
        with tempfile.TemporaryDirectory() as directory:
            os.environ["CODEX_BRIDGE_ARCHIVE_DIR"] = directory
            with patch("shutil.which", return_value="/mock/agy"), \
                 patch("subprocess.run") as mock_run:
                mock_run.return_value = MagicMock(returncode=0, stdout="Mock Gemini Output", stderr="")
                from server import consult_gemini
                response = consult_gemini("hi", directory)
                self.assertEqual(response, "Mock Gemini Output")
                mock_run.assert_called_once_with(
                    ["/mock/agy", "--print", "hi"],
                    cwd=directory,
                    capture_output=True,
                    text=True,
                    timeout=90,
                    shell=False,
                )
        os.environ.pop("CODEX_BRIDGE_ARCHIVE_DIR", None)



if __name__ == "__main__":
    unittest.main()
