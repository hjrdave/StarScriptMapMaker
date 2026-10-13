# Research: open-licensed fantasy map asset sources

Resolves issue #5 (parent map #2). Question: which CC0 or permissively licensed asset packs could be bundled with a possibly commercial fantasy map maker, and which Wonderdraft-style categories stay uncovered.

## Status: partial, read this first

Licences were verified only from the licence files in the sources' own git repositories (cloned directly, commit hashes below). The research sandbox could reach git hosting on GitHub only. Every asset-site host (kenney.nl, opengameart.org, game-icons.net, ambientcg.com, polyhaven.com, quaternius.com, itch.io, creativecommons.org, fonts.google.com) returned an egress-proxy 403. So the pack pages for Kenney, OpenGameArt, ambientCG, Poly Haven and Quaternius could **not** be read, and this file makes **no licence claims about them**. They appear only as an unverified to-do list. Do not bundle anything from that list until its page or licence file has been read.

## Verified findings

### 1. Game-icons.net icon set (SVG): CC BY 3.0, attribution required

- Source: https://github.com/game-icons/icons at commit `82d948812bfe3f269ef8f731dcdb07b08160edc4` (2026-04-23).
- `license.txt` (repo root): "Icons provided under the Creative Commons 3.0 BY or CC0 if mentioned below." It then lists contributors, one sub-folder each. Only Viscious Speed and Zeromancer are marked CC0.
- `CONTRIBUTING.md`: "Most of them are released under the Creative Commons BY 3.0, others as Public Domain (CC0)."
- `README.md` licence section says only "CC-BY".
- Format: 4239 SVG files, 512x512 viewBox, single-path, white shape on a black square background. The repo ships `colorize-svgs.sh` (recolour, transparent background) and `rasterize-svgs.sh` (PNG).
- Style: monochrome glyph icons, varied by contributor (Lorc, Delapouite and others). Consistent as a UI/icon set, not as top-down or side-view map art.
- Map-relevant examples present: `delapouite/castle.svg`, `village.svg`, `huts-village.svg`, `castle-ruins.svg`, `camping-tent.svg`, `palm-tree.svg`, `mountain-cave.svg`, `fruit-tree.svg`, `bonsai-tree.svg`; `badges/mountain.svg`.
- Obligation: CC BY 3.0 requires credit to the author per icon. Bundling means shipping an in-app credits screen and per-asset metadata (author, source, licence). Commercial use is allowed under CC BY 3.0, but I did not read the CC BY 3.0 legal text (creativecommons.org was blocked); confirm it before shipping. CC0-marked contributor folders need no attribution.
- Fit: good for POI/marker icons (towns, castles, camps). Poor for terrain stamps (mountains, forests), where it has only a few generic glyphs.

### 2. Google Fonts, selected display and serif faces: SIL OFL 1.1

- Source: https://github.com/google/fonts at commit `66928797f271fed5ee428544e7229685e2d90127` (2026-10-12), `ofl/<family>/OFL.txt` and `METADATA.pb` for each family below.
- Checked: Almendra, Cinzel, Germania One, IM Fell Double Pica, Macondo, Mate, MedievalSharp, Metamorphous, Uncial Antiqua. Each `OFL.txt` states "licensed under the SIL Open Font License, Version 1.1" and each carries a Reserved Font Name. Each family ships TTF (Cinzel as a variable `wght` font).
- OFL 1.1 terms (quoted from `ofl/cinzel/OFL.txt`):
  - Free to "use, study, copy, merge, embed, modify, redistribute, and sell modified and unmodified copies".
  - The font may not be sold by itself.
  - Bundling and selling with software is allowed "provided that each copy contains the above copyright notice and this license".
  - Modified versions may not use the Reserved Font Name.
  - The font must stay under OFL. This does not apply to documents made with it, so exported map PNGs are unaffected.
- Obligation: ship each font's `OFL.txt` and copyright line with the app. No in-map attribution.
- Fit: good for a default label set. A map maker that lets users export PNGs with these fonts is covered by the document exception above.
- Not checked: whether each family has the glyph coverage and kerning the label system needs. That belongs to the labels ticket.

## Unverified candidates (licence not read; research only)

These are sources worth checking in a session that can reach their sites. Each entry says what to read. Nothing here is a finding.

| Candidate | Wanted for | Read before use |
| --- | --- | --- |
| Kenney asset packs | tiles, icons, UI | each pack's page and the licence file inside its zip |
| OpenGameArt entries | terrain stamps, buildings | licence field on each entry; licences vary per entry (CC0, CC-BY, CC-BY-SA, GPL), so every asset needs its own check |
| ambientCG | parchment, paper, ground textures | site licence page |
| Poly Haven | textures | site licence page |
| Quaternius | 3D/low-poly props | pack page; likely wrong style for 2D maps |
| itch.io map-asset packs | mountains, trees, hills | each pack's own licence text; per-author, not standardised |

## Coverage against Wonderdraft-style categories

| Category (from the map's v1 scope) | Verified source | Status |
| --- | --- | --- |
| Labels / fonts | OFL families above | covered |
| POI icons (towns, castles, camps) | game-icons.net (CC BY) | covered with attribution; mismatched with any terrain art style |
| Mountains, hills, trees (consistent stamp set) | none verified | uncovered; likely original art |
| Buildings (map-scale stamps) | none verified | uncovered beyond generic icons |
| Lakes, coastline styling | n/a (procedural) | no asset needed |
| Ocean backdrop, parchment texture | none verified | uncovered; unverified candidates above, or procedural/original |

## Recommendations for the map

1. Treat fonts as settled: bundle OFL families with licence files.
2. Treat game-icons.net as usable for marker icons only, with an attribution screen and per-asset author metadata in the asset format.
3. Plan on original art (the map already says "mix of CC0 packs and the author's own art") for the core terrain stamp set (mountains, hills, trees, forests) and for a style-consistent backdrop, because no verified source covers them.
4. Re-run the unverified table from a session with web access before finalising the default asset list. Asset-format design should carry `author`, `source_url` and `licence` fields per asset so mixed licences stay auditable.
