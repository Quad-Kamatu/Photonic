#!/usr/bin/env python3
"""Generate docs/mcp-api.md from the MCP tool-list JSON.

Reads the JSON produced by the `dump_tools` binary on stdin and emits a
Markdown reference on stdout, one section per tool with a parameter table
derived from each tool's `inputSchema`.

Usage:
    cargo run -p photonic-mcp --bin dump_tools | python3 tools/gen-mcp-docs.py > docs/mcp-api.md
"""
import json
import sys


def esc(text: str) -> str:
    """Escape Markdown table-breaking characters in a cell."""
    return str(text).replace("|", "\\|").replace("\n", " ").strip()


def type_of(schema: dict) -> str:
    """Render a parameter's type from its JSON-schema fragment."""
    t = schema.get("type")
    if isinstance(t, list):
        t = " | ".join(t)
    if t == "array":
        items = schema.get("items", {})
        inner = type_of(items) if isinstance(items, dict) else "any"
        return f"array<{inner}>"
    if "enum" in schema:
        vals = ", ".join(f"`{v}`" for v in schema["enum"])
        return f"enum ({vals})"
    return t or "any"


def render_tool(tool: dict) -> str:
    name = tool.get("name", "?")
    desc = tool.get("description", "").strip()
    out = [f"## `{name}`", ""]
    if desc:
        out += [desc, ""]

    schema = tool.get("inputSchema", {}) or {}
    props = schema.get("properties", {}) or {}
    required = set(schema.get("required", []) or [])

    if props:
        out += [
            "| Parameter | Type | Required | Description |",
            "| --- | --- | --- | --- |",
        ]
        # Required first, then alphabetical, for readability.
        for key in sorted(props, key=lambda k: (k not in required, k)):
            p = props[key] or {}
            req = "yes" if key in required else "no"
            out.append(
                f"| `{esc(key)}` | {esc(type_of(p))} | {req} | "
                f"{esc(p.get('description', ''))} |"
            )
        out.append("")
    else:
        out += ["_No parameters._", ""]
    return "\n".join(out)


def main() -> None:
    tools = json.load(sys.stdin)
    tools.sort(key=lambda t: t.get("name", ""))

    lines = [
        "# Photonic MCP API Reference",
        "",
        "<!-- GENERATED FILE — do not edit by hand. -->",
        "<!-- Regenerate with: "
        "cargo run -p photonic-mcp --bin dump_tools | python3 tools/gen-mcp-docs.py > docs/mcp-api.md -->",
        "",
        f"This document lists all **{len(tools)}** MCP tools exposed by "
        "`photonic-mcp`, generated directly from `server::tool_list()` so it "
        "cannot drift from the implementation.",
        "",
        "## Tools",
        "",
        ", ".join(f"[`{t['name']}`](#{t['name'].replace('_', '-')})" for t in tools),
        "",
        "---",
        "",
    ]
    lines += [render_tool(t) for t in tools]
    sys.stdout.write("\n".join(lines).rstrip() + "\n")


if __name__ == "__main__":
    main()
