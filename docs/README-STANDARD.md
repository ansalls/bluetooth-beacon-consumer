# README Standard

The shared README format used across all projects indexed in
[claude-projects-list](https://gitlab.epic.com/tsalls/claude-projects-list).
The dashboard's Tomes page renders the **Summary** and **Capabilities** sections directly,
so those two sections are *required* and have a *fixed shape*. Everything else is a recommendation.

## Goals

1. Give every repo a consistent first impression — a reader can find purpose, capabilities,
   and setup in the same place across projects.
2. Make the headline content machine-parseable so the Tomes overview can surface a project
   without bespoke tooling per repo.
3. Stay close to widely-used SoTA conventions (GitHub's standard README structure,
   makeareadme.com) so contributors aren't surprised by anything project-specific.

## File location

- One `README.md` at the repository root.
- UTF-8, LF or CRLF, no BOM.
- Use GitHub-flavored Markdown.

## Required structure

```markdown
# <Project Name>

> One-sentence elevator pitch. Optional but recommended; lives on a single line as a Markdown blockquote.

## Summary

<1–3 short paragraphs of plain prose. Describes what the project is, who it's for, and why it exists.>
<The first paragraph MUST stand alone — the Tomes overview shows it as the project preview.>
<No code blocks, no headings, no images, no tables inside this section.>

## Capabilities

- **<Capability name>** — <one-line description of what it does today>.
- **<Capability name>** — <one-line description>.
- ...

## Status           (optional)
## Prerequisites    (optional)
## Setup            (recommended)
## Usage            (recommended)
## Tools            (optional — for tool/library projects with many entry points)
## Architecture     (optional)
## Configuration    (optional)
## Development      (optional)
## Testing          (optional)
## Roadmap          (optional)
## License          (optional)
## Acknowledgements (optional)
## See Also         (optional)
```

### Section ordering

`# Title` → optional `> tagline` → `## Summary` → `## Capabilities` → recommended/optional sections.

The two anchor sections (`Summary`, `Capabilities`) MUST appear in that order, before any
other `##` section, so a parser can locate them by walking forward from the top.

## Section rules

### `## Summary`

- Plain prose, 1–3 paragraphs.
- The **first paragraph** is the canonical short description; keep it under ~60 words and
  self-contained (no "see below" references).
- No headings, no fenced code blocks, no images, no tables. Inline links and inline code
  spans are fine.
- Avoid roadmap content. Describe what the project *is* and *does today*.

### `## Capabilities`

- A flat unordered Markdown list (`-` or `*`) — no nested lists, no sub-headings.
- 3–8 bullets is the sweet spot. Hard cap: 12.
- Each bullet ≤ ~140 characters.
- Pattern: `- **<Subject>** — <verb phrase>.` The bolded subject is the capability name;
  the dash + sentence is what it does. The bold prefix is optional but encouraged.
- No roadmap items, no aspirational features. Only things that work today.
- Synonyms (`## Features`, `## What it does`) are accepted by the parser as fallbacks
  but `## Capabilities` is preferred for new/updated READMEs.
- **Tool-heavy projects** (8+ MCP tools, HTTP endpoints, CLI subcommands, etc.):
  keep Capabilities at 3–8 *high-level* bullets that group the entry points by theme,
  then list the full inventory in a `## Tools` (or `## HTTP API` / `## Subcommands`)
  section later in the file. See the two-tier pattern below.
- **Content / curation repos** (prompt libraries, research corpora, dataset collections,
  documentation sites): use Capabilities to name the *collections or content categories*
  the repo offers — e.g. `- **Code review prompts** — patterns for PR review and PQA.`
  Treat each collection as the "thing the repo provides" instead of a code feature. If
  the repo is a single curated dataset, list its 3–8 most useful entry points or facets.

### `## Status` (optional)

A single short paragraph or one-liner conveying maturity: `planning`, `alpha`, `beta`,
`stable`, `maintenance`, `deprecated`, etc. Useful when the master-table `State` column
is too coarse, when there are platform/runtime requirements ("Requires Windows 11"),
or when the project is in a notable phase (e.g. "Alpha. Phase 0 — discovery only").
Plain prose; one sentence is fine.

`## Status` is for *maturity and runtime constraints*, not progress tracking. If you
want to surface "what's done and what's next", use `## Roadmap`. Don't conflate the two
in one section.

### `## Prerequisites` (optional)

External dependencies, accounts, hardware, or platform requirements that must be in
place before Setup makes sense (e.g. "Requires a registered Azure AD app with
Mail.Send delegated scope" or "Requires GT.M / YottaDB ≥ 7.x"). Keep it short — link
out to provider docs rather than reproducing them.

### `## Roadmap` (optional)

What's done, what's in progress, and what's planned. Tables and progress checklists
belong here — *not* in `## Status` or `## Capabilities`. Aspirational items go here,
never in Capabilities.

### `## Setup` (recommended)

Concrete install/build steps. Code blocks welcome.

### `## Usage` (recommended)

Smallest useful example. Link to fuller docs rather than inlining a manual.

### `## Configuration` (optional)

Environment variables, config-file keys, and persistent runtime flags. Distinct from
`## Setup` (install/build steps) and from `## Usage` (per-run invocation). If your repo
has more than a couple of env vars or YAML/TOML keys, give them their own section here
rather than burying them in Setup.

### `## Tools` / `## HTTP API` / `## Subcommands` (optional)

For projects that expose many discrete entry points (MCP tools, HTTP routes, CLI verbs),
list them here as a Markdown table or per-tool subheadings. This is the *detailed
inventory* — it complements, rather than replaces, the high-level `## Capabilities`
bullets above. Pick the heading that best names what's exposed.

### `## Acknowledgements` / `## Credits` (optional)

Attribution for upstream projects, prior art, vendored libraries, or individuals whose
work this repo builds on. Use whichever heading reads more naturally; the parser
treats them as the same slot.

### `## See Also` / `## Related Projects` (optional)

Links to alternative implementations, sister repos, or projects this one composes with.
Use whichever heading reads more naturally.

### Other sections

`## Architecture`, `## Development`, `## License`, `## Contributing`, `## Testing` —
use as needed in the recommended order above.

## Two-tier pattern for tool-heavy projects

When a repo exposes many discrete tools or endpoints, don't try to flatten the entire
inventory into the `## Capabilities` list — split:

```markdown
## Capabilities

- **Mail and calendar tools** — read/send mail, manage events, manage room bookings.
- **OneDrive and SharePoint** — list, fetch, upload, and search files.
- **Tasks and notes** — manage Microsoft To Do tasks and OneNote pages.
- **Directory lookup** — resolve users, contacts, and meeting rooms.

## Tools

| Category | Tool                       | Description                          |
| -------- | -------------------------- | ------------------------------------ |
| Mail     | `mail.send`                | Send a new mail message.             |
| Mail     | `mail.list`                | List recent messages.                |
| Calendar | `calendar.create_event`    | Create a calendar event.             |
| ...      | ...                        | ...                                  |
```

This keeps the Tomes overview readable (4 high-level bullets) while preserving the full
reference catalog for users who need it.

## Adopting the standard in an existing repo

Most existing READMEs already contain the right *content* — they just need the right
*headings*. Prefer renaming over rewriting:

- If your README opens with prose paragraphs, retitle that block `## Summary`. The parser
  picks up whatever appears under that heading; you don't need to rewrite the prose.
- If the file's first non-title line is a single-sentence pitch, prefix it with `>` to
  promote it to the tagline slot. Don't introduce a new line for it.
- If you have a `## Features` (or `## Key Features`, `## What it does`) bullet list, the
  parser treats it as Capabilities. Rename to `## Capabilities` next time you touch the
  file, but it works today.
- If you see `## Current Status` or `## Current State`, rename to `## Status` (for
  maturity/platform notes) or `## Roadmap` (for progress tables and what's-next lists).
  Pick the one that matches the section's actual content.
- If a status/maturity callout is floating in the intro or under Setup, move it under
  `## Status`.
- If you have a long table of MCP tools, HTTP endpoints, or CLI subcommands inline in the
  intro, move it under `## Tools` (or `## HTTP API` / `## Subcommands`) and add 3–8
  high-level grouping bullets above under `## Capabilities`.

## Style

- Title case for `##` headings.
- One blank line between sections.
- Wrap prose around 100 characters where reasonable; don't reflow other people's pasted output.
- Don't start a section with the section name as a sentence (write `Toasty exposes 6 tools…`,
  not `## Tools — Tools are exposed…`).

## Drop-in scaffold

A copy-pasteable starter is available at [`README-template.md`](./README-template.md).

## Tomes integration

The dashboard's `readmeParser` extracts:

- `summary` — text content of the `## Summary` section, joined paragraphs.
- `capabilities` — array of bullet strings from the `## Capabilities` (or `## Features`) section.

Both are surfaced on the Tomes page when a project has a local clone with a conforming README.
Projects without these sections still render — they just fall back to the one-line description
from the master table in [`README.md`](../README.md).

## Maintenance

When you create, fork, or significantly refactor a project, ensure its README still conforms.
The repo-local [`AGENTS.md`](../AGENTS.md) instructs Claude/Codex agents to keep the README
aligned with this standard.
