---
name: memory
description: Two-layer memory system with grep-based recall.
always: true
---

# Memory

## Structure

- `memory/MEMORY.md` - Long-term facts (preferences, project context, relationships). Always loaded into context.
- `memory/HISTORY.md` - Append-only event log. Not loaded into context. Search it with grep.

## Search Past Events

```bash
grep -i "keyword" memory/HISTORY.md
```

Use the `exec` tool to run grep. Combine patterns:

```bash
grep -iE "meeting|deadline" memory/HISTORY.md
```

## When to Update MEMORY.md

Write important facts immediately using `edit_file` or `write_file`:
- User preferences ("I prefer dark mode")
- Project context ("The API uses OAuth2")
- Relationships ("Alice is the project lead")

## Auto-consolidation

Old conversations are automatically summarized into `HISTORY.md` when sessions grow large. New long-term facts are extracted to `MEMORY.md`.
