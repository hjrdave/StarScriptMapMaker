# Graphify evaluation (issue #355)

Status: **not run yet.** This note holds everything that can be prepared without running Graphify, so the evaluation takes one sitting and ends in a recorded keep-or-drop decision. Nothing below is a measured result for Graphify.

Why it has not been run: the session that wrote this could not reach `pypi.org`, `files.pythonhosted.org` or `graphify.com` (its network policy blocks them), so it could not install Graphify or read its documentation. The claims about Graphify come from the issue text only. Measuring it also needs interactive Claude Code sessions with the `/graphify` skill and MCP server, which an unattended run cannot do.

## Are the prerequisites met?

The issue says to wait for #352, #349 and #350.

- #352 (split `apps/starscript/src/lib.rs`): done.
- #349 (split the oversized files): done, closed. Every non-test source file is under about 800 lines.
- #350 (group app modules into folders): the app crate is grouped (`app/`, `workspace/`, `canvas/`, `panels/`, `chrome/`, `dialogs/`, `export/`, `packs/`), as `CLAUDE.md` describes. The issue is already closed.

So the tree has settled, and a graph built now will not be thrown away by the next refactor.

## What the graph would cover

Rust, counted on `master` after #448:

| Crate | Files | Lines |
| --- | --- | --- |
| `crates/core` | 78 | 19,105 |
| `crates/render` | 34 | 11,154 |
| `crates/fonts` | 1 | 173 |
| `apps/starscript` | 129 | 36,811 |
| Total | 242 | about 67,400 (about 2.3 MB) |

At roughly 4 bytes a token, reading all the Rust once is on the order of 580,000 tokens. That is the cost a graph has to beat for tasks that otherwise mean reading many files.

## Privacy: what would leave the machine

The issue says Graphify parses code locally and sends docs, PDFs and images to the model API. This repo has:

- `docs/`: 54 files, 2.5 MB. 20 Markdown (ADRs, the spec, research notes), 13 HTML and 13 PNG design handoff files, a few YAML, JSON, JS and patch files.
- `assets/`: 37 MB. 299 PNG, 85 TTF, plus manifests, licence text and two SVG and two `.abr` files. Nothing here is code, and the images would be the costly and pointless part to process.

Recommended first run, which sends nothing to an API: **code only.** Point Graphify at the Rust and `Cargo.toml` files and exclude `docs/`, `assets/`, `target/` and `docs/design/`. Decide separately whether `CONTEXT.md` and `docs/adr/` (plain Markdown, no images) are worth adding, since the domain terms and ADRs are what a "how does X connect to Y?" question usually needs. That decision is the owner's.

## Tasks to compare

Run each in a fresh Claude Code session twice, with and without Graphify, and record the answer's correctness, the tool calls and the tokens. Each question has a mechanical baseline from `grep`.

| # | Task | Why it is a fair test | `grep` baseline |
| --- | --- | --- | --- |
| 1 | List everything that executes a Command through `History` | Callers across crates | 3 call sites outside tests; 15 files including tests |
| 2 | Trace a Label edit from the Properties panel to pixels on screen | A long chain: panel, `Editor`, Command, Ops, `Map`, scene, render | None; needs reading the path |
| 3 | Where is Roughness computed and cached, and who invalidates it? | Spread over `core` and `render` | 23 files mention `RoughnessCache` or `roughen(` |
| 4 | What breaks if `Op::SetDefaults` gains a field? | Impact analysis | 16 mentions in 8 files |
| 5 | Which modules depend on `ssmap_core::land`? | Dependency query | 10 files, 6 outside tests |

Tasks 2 and 3 are where a graph should help most. Tasks 1, 4 and 5 are where `grep` or rust-analyzer (issue #354) may already be enough, so they show whether Graphify adds anything over those cheaper tools.

## Protocol

1. In a scratch clone: `pip install graphifyy`, `graphify install`, then `/graphify .` with the exclusions above. Record the build time and the size of `graphify-out/`.
2. Run the five tasks with and without it. Note wrong or missing answers, not only tokens: the issue warns that claimed savings range from about 5x to 71x.
3. Rebuild after a trivial edit and note how long it takes and whether the graph noticed the change. This shows how fast it goes stale.
4. Repeat task 1, 4 and 5 with rust-analyzer (#354) and compare.

## Decisions to record

- **Commit `graphify-out/` or rebuild locally?** Committing makes every refactor a noisy diff. The issue suggests rebuilding in the SessionStart hook (`.claude/hooks/session-start.sh`) if it is kept, which costs session start time.
- **Scope of the graph:** code only, or code plus `CONTEXT.md` and `docs/adr/`.
- **Keep or drop**, with the reasoning, in the issue or an ADR.

## Result

Not filled in yet. When the evaluation is run, add here: the build time, the task results, the keep-or-drop decision and the reasoning, then close #355 (or update `CLAUDE.md` if it is kept).
