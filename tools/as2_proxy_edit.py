#!/usr/bin/env python3
"""AS-2 "Short film" acceptance story — proxy-edit slice.

Drives a running Photonic MCP server over HTTP JSON-RPC (the same `POST
/mcp` endpoint `crates/photonic-mcp/src/server.rs` exposes), exercising the
P3 story slice `docs/specs/video-editor/00-overview.md`'s phase table calls
"AS-2: proxy edit": import a heavy asset, generate proxies for it, toggle
the session proxy policy, and edit normally regardless.

Full AS-2 ("import several 4K clips -> generate proxies -> multi-track edit
with cross-dissolves -> ...", 00-overview.md §2) needs capabilities that
land in P6-P8 (transitions, grade, mixer) — this script tracks only the
slice available now, per `11-testing-phasing.md` §3.4/§6's incremental
per-phase MCP-script policy.

No real 4K fixture exists in the committed test-media corpus
(`crates/photonic-video/tests/fixtures/README.md` — everything tops out at
320x180 to stay inside the 5 MB corpus budget). This script uses
`beep_flash.mp4`, the corpus's longest/heaviest real asset (60s), as a
stand-in for "a big imported clip" — the resolution doesn't matter for what
this slice actually exercises: `generate_proxies` currently reports
NotSupportedV1 regardless of source size (see `s_generate_proxies` below),
and `set_proxy_mode`/editing behave identically at any resolution.

Usage
-----
    # 1. Launch a headless Photonic MCP server (no GUI window):
    cargo run -p photonic-app -- --headless

    # 2. Run this script against it (default: http://127.0.0.1:7842/mcp):
    python3 tools/as2_proxy_edit.py

Exit code is 0 iff every step passed or was skipped (skips are reserved for
environment gaps — no GPU adapter — this script can't control); any FAIL
exits nonzero, so this is CI-runnable once a GPU runner is available.

Stdlib-only by design — no `requests` dependency, so this runs anywhere a
plain `python3` does. The HTTP client / step-runner below is intentionally
duplicated from `tools/as1_arrange_cut.py` rather than shared through a
local module — each story script stays a single self-contained file, same
convention as `tools/gen-mcp-docs.py` / `tools/gen-test-fixtures.py`.
"""
from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any, Optional

REPO_ROOT = Path(__file__).resolve().parent.parent
FIXTURES_DIR = REPO_ROOT / "crates" / "photonic-video" / "tests" / "fixtures"

# crates/photonic-core/src/timeline/time.rs — exact ticks-per-second, chosen
# so every common frame rate (23.976 .. 120fps) divides it cleanly.
TICKS_PER_SECOND = 705_600_000

DEFAULT_URL = os.environ.get("PHOTONIC_MCP_URL", "http://127.0.0.1:7842/mcp")


def ticks(seconds: float) -> int:
    """Seconds -> exact ticks. Safe for this script's 30fps sequence (30
    divides TICKS_PER_SECOND with no remainder)."""
    return round(seconds * TICKS_PER_SECOND)


# ─── Minimal HTTP JSON-RPC client ──────────────────────────────────────────


class RpcError(RuntimeError):
    pass


class ToolResult:
    """Decodes an MCP `tools/call` result (content[] + isError) per
    `crates/photonic-mcp/src/protocol/args/c.rs::ToolResult`. Every
    `.with_data(...)` call appends a second `content` text block that is
    itself pretty-printed JSON (`ContentItem::json`) — this parses whichever
    blocks decode as a JSON object, later blocks winning on key collision."""

    def __init__(self, tool_name: str, raw: dict):
        self.tool_name = tool_name
        self.raw = raw
        self.is_error = bool(raw.get("isError"))
        texts = [c.get("text", "") for c in raw.get("content", []) if c.get("type") == "text"]
        self.message = texts[0] if texts else ""
        self.data: dict[str, Any] = {}
        for t in texts[1:]:
            try:
                parsed = json.loads(t)
            except (json.JSONDecodeError, TypeError):
                continue
            if isinstance(parsed, dict):
                self.data.update(parsed)

    @property
    def error_code(self) -> Optional[str]:
        return self.data.get("error_code")

    def __repr__(self) -> str:
        return (
            f"ToolResult({self.tool_name}, is_error={self.is_error}, "
            f"message={self.message!r}, data={self.data!r})"
        )


class Client:
    """Talks JSON-RPC 2.0 to the Photonic MCP server's single `POST /mcp`
    endpoint (server.rs::build_router)."""

    def __init__(self, url: str):
        self.url = url
        self._next_id = 1

    def call(self, method: str, params: Optional[dict] = None) -> dict:
        req_id = self._next_id
        self._next_id += 1
        body = json.dumps(
            {"jsonrpc": "2.0", "id": req_id, "method": method, "params": params or {}}
        ).encode("utf-8")
        req = urllib.request.Request(
            self.url, data=body, headers={"Content-Type": "application/json"}, method="POST"
        )
        try:
            with urllib.request.urlopen(req, timeout=60) as resp:
                payload = json.loads(resp.read().decode("utf-8"))
        except urllib.error.URLError as e:
            raise RpcError(
                f"cannot reach Photonic MCP server at {self.url} ({e}) — is it running? "
                "See this script's module docstring for how to launch one."
            ) from e
        if "error" in payload:
            err = payload["error"]
            raise RpcError(f"{method}: JSON-RPC error {err.get('code')}: {err.get('message')}")
        return payload["result"]

    def tool(self, name: str, arguments: Optional[dict] = None) -> ToolResult:
        result = self.call("tools/call", {"name": name, "arguments": arguments or {}})
        return ToolResult(name, result)


# ─── Step runner ────────────────────────────────────────────────────────────


class Skip(Exception):
    """Raise from a step to mark it skipped (environment gap, not a bug)."""


class Runner:
    def __init__(self):
        self.total = 0
        self.failed = 0
        self.skipped = 0

    def step(self, name: str, fn) -> Any:
        """Run one step, print its PASS/FAIL/SKIP line, and return whatever
        `fn` returns (so later steps can chain off earlier results) —
        `None` on failure/skip."""
        self.total += 1
        try:
            result = fn()
        except Skip as e:
            self.skipped += 1
            print(f"[SKIP] {name}: {e}")
            return None
        except (AssertionError, RpcError) as e:
            self.failed += 1
            print(f"[FAIL] {name}: {e}")
            return None
        except Exception as e:  # noqa: BLE001 — last-resort net so one bad step doesn't kill the report
            self.failed += 1
            print(f"[FAIL] {name}: unexpected {type(e).__name__}: {e}")
            return None
        print(f"[PASS] {name}")
        return result

    def summary(self) -> int:
        passed = self.total - self.failed - self.skipped
        print(
            f"\n{passed}/{self.total} passed, {self.skipped} skipped, "
            f"{self.failed} failed"
        )
        return 1 if self.failed else 0


def expect_ok(r: ToolResult) -> ToolResult:
    if r.is_error:
        raise AssertionError(f"{r.tool_name} returned an error: {r.message!r} (data={r.data!r})")
    return r


def engine_available(client: Client) -> bool:
    """Mirrors `handlers/video.rs`'s own test-only `engine_available()` —
    engine-backed tools need a GPU adapter; skip cleanly if this machine
    doesn't have one rather than failing the whole run."""
    return not client.tool("get_engine_status", {}).is_error


# ─── Story steps ────────────────────────────────────────────────────────────


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default=DEFAULT_URL, help=f"MCP endpoint (default: {DEFAULT_URL})")
    args = parser.parse_args()

    client = Client(args.url)
    run = Runner()
    state: dict[str, Any] = {}

    def s_connect():
        result = client.call(
            "initialize",
            {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "as2_proxy_edit.py", "version": "1"},
            },
        )
        assert result.get("serverInfo", {}).get("name") == "photonic", result

    run.step("connect (initialize)", s_connect)

    def s_create_sequence():
        r = expect_ok(
            client.tool(
                "create_sequence",
                {
                    "name": "AS-2 Sequence",
                    "frame_rate": {"num": 30, "den": 1},
                    "formats": [{"name": "16:9", "width": 320, "height": 180}],
                },
            )
        )
        state["sequence_id"] = r.data["sequence_id"]

    run.step("create_sequence", s_create_sequence)

    def s_import_4k_ish():
        # See module docstring — beep_flash.mp4 stands in for "a big
        # imported clip"; no real 4K fixture exists in the corpus.
        path = str(FIXTURES_DIR / "beep_flash.mp4")
        assert Path(path).exists(), f"fixture missing: {path} — run tools/gen-test-fixtures.py"
        r = expect_ok(client.tool("import_media", {"paths": [path], "bin": "AS-2 4K-ish stand-in"}))
        assets = r.data["assets"]
        assert len(assets) == 1, r.data
        state["asset_id"] = assets[0]["asset_id"]

    run.step("import_media (4K-ish stand-in fixture)", s_import_4k_ish)

    def s_generate_proxies():
        # generate_proxies is NOT IMPLEMENTED in this build — the
        # engine/proxy module (02 §6, 05 §2.3) hasn't landed yet, so it
        # always reports NotSupportedV1 (handlers/video.rs::generate_proxies).
        # This step's PASS condition is that documented current-build
        # behavior, not success — update it once CAP-014 proxy generation
        # actually lands.
        r = client.tool("generate_proxies", {"asset_ids": [state["asset_id"]]})
        assert r.is_error, f"expected NotSupportedV1, got success: {r}"
        assert r.error_code == "NotSupportedV1", f"unexpected error_code: {r}"

    run.step("generate_proxies (expected NotSupportedV1)", s_generate_proxies)

    def s_toggle_proxy_mode():
        # `set_proxy_mode`'s response echoes the Rust enum's Debug form
        # (session.rs::ProxyMode) — PascalCase, no underscore.
        expected = {"force_proxy": "ForceProxy", "force_original": "ForceOriginal", "auto": "Auto"}
        for mode in ("force_proxy", "force_original", "auto"):
            r = expect_ok(client.tool("set_proxy_mode", {"mode": mode}))
            assert r.data["mode"] == expected[mode], r.data

    run.step("set_proxy_mode (toggle force_proxy -> force_original -> auto)", s_toggle_proxy_mode)

    def s_edit_under_proxy_mode():
        # Editing must work the same regardless of proxy mode (proxies are
        # never required for correctness, CAP-014) — insert, split, trim on
        # the imported asset while the session is still on whatever mode the
        # previous step left it in (auto).
        v1 = expect_ok(client.tool("add_track", {"sequence_id": state["sequence_id"], "kind": "video", "name": "V1"}))
        track_id = v1.data["track_id"]
        clip = expect_ok(
            client.tool(
                "insert_clip",
                {
                    "track_id": track_id,
                    "name": "beep_flash",
                    "start_ticks": ticks(0),
                    "source": {"kind": "asset", "asset_id": state["asset_id"]},
                    "duration_ticks": ticks(4),
                },
            )
        )
        clip_id = clip.data["clip_id"]
        split = expect_ok(client.tool("split_clip", {"clip_id": clip_id, "at_ticks": ticks(2.5)}))
        new_clip_id = split.data["new_clip_id"]
        expect_ok(client.tool("trim_clip", {"clip_id": clip_id, "edge": "out", "new_ticks": ticks(2)}))

        clips = {
            c["clip_id"]: c
            for c in expect_ok(
                client.tool("list_clips", {"sequence_id": state["sequence_id"]})
            ).data["clips"]
        }
        assert clips[clip_id]["duration_ticks"] == ticks(2), clips[clip_id]
        assert clips[new_clip_id]["start_ticks"] == ticks(2.5), clips[new_clip_id]

    run.step("edit (insert/split/trim while proxy mode toggled)", s_edit_under_proxy_mode)

    return run.summary()


if __name__ == "__main__":
    sys.exit(main())
