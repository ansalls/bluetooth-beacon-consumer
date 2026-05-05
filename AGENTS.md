# Agent instructions

Instructions for Claude Code, Codex, and other coding agents working in this repo
(or any repo that adopts this file). Drop this file at the repository root so it
loads automatically.

## README format

This repo follows the cross-project README standard maintained at
[claude-projects-list](https://gitlab.epic.com/tsalls/claude-projects-list/-/blob/main/docs/README-STANDARD.md).
A seeded copy lives at [`docs/README-STANDARD.md`](./docs/README-STANDARD.md). The
dashboard's Tomes page parses the **Summary** and **Capabilities** sections directly,
so they are required and have a fixed shape.

When you create, edit, or review a README:

1. Confirm the file follows this exact top-of-file structure:

   ```
   # <Project Name>
   > optional tagline
   ## Summary
   ## Capabilities
   ```

2. `## Summary` — 1–3 short paragraphs of plain prose. First paragraph stands alone.
   No headings, code blocks, images, or tables inside.
3. `## Capabilities` — 3–8 flat bullets, each ≤ ~140 chars, ideally
   `- **<Subject>** — <verb phrase>.`. No nesting, no sub-headings.
   Today-features only — no roadmap items.
4. After the anchor sections, use any subset of `## Status`, `## Prerequisites`,
   `## Setup`, `## Usage`, `## Tools` (or `## HTTP API` / `## Subcommands`),
   `## Architecture`, `## Configuration`, `## Development`, `## Testing`,
   `## Roadmap`, `## License`, `## Contributing`, `## Acknowledgements`
   (or `## Credits`), `## See Also` (or `## Related Projects`), in that order.
5. If a README has the older `## Features` heading, the parser still picks it up —
   but rename to `## Capabilities` when you touch the file.
6. `## Status` is for maturity and runtime constraints. Progress lists, roadmaps,
   and "what's done so far" tables belong under `## Roadmap`. Don't conflate them.

### Adopting the standard in an existing repo

Prefer **renaming over rewriting** — most existing READMEs already have the right
content, just under different headings:

- If the file opens with untitled prose paragraphs, retitle that block `## Summary`.
  Don't restructure the prose unless it's wrong; the parser only needs the heading.
- If the file's first non-title line is a single-sentence pitch, prefix it with `>`
  to promote it to the tagline slot. Don't add a second line for it.
- If there's a `## Features` (or `## Key Features` / `## What it does` /
  `## What Works Today` / `## Working Features` / `## Current Features`) bullet list,
  rename it to `## Capabilities`. The parser falls back to the older synonyms, but
  new edits should use `## Capabilities`.
- If there's a `## Current Status` or `## Current State` heading, rename it to
  `## Status` (for maturity/platform notes) or `## Roadmap` (for progress tables and
  what's-next lists). Pick the one that matches the section's actual content.
- If there's a floating maturity/status line ("Alpha. Phase 0…", "Requires Windows 11"),
  move it under `## Status`.
- For content/curation repos (prompt libraries, research corpora, docs sites), use
  `## Capabilities` to name the collections or content categories rather than code
  features.
- For tool-heavy repos (8+ MCP tools, HTTP endpoints, or CLI subcommands), don't try
  to fit every entry point into Capabilities. Keep `## Capabilities` at 3–8 high-level
  themes, then list the full inventory under `## Tools` (or `## HTTP API` /
  `## Subcommands`). See the two-tier pattern in
  [`docs/README-STANDARD.md`](./docs/README-STANDARD.md#two-tier-pattern-for-tool-heavy-projects).

A copy-paste scaffold is in [`docs/README-template.md`](./docs/README-template.md).

## Conventions

- Edit existing files; don't add new top-level docs unless necessary.
- Keep commit messages concise and imperative ("Add Capabilities to Tomes pane").
- Don't restructure unrelated files in a single change.
