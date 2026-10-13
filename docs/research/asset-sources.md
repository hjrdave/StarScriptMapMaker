# Research: open-licensed fantasy map asset sources

Resolves issue #5 (parent map #2). Question: which CC0 or permissively licensed asset packs could be bundled with a possibly commercial fantasy map maker, and which Wonderdraft-style categories stay uncovered.

## Status: mostly complete, read this first

Two sessions contributed. The first could reach GitHub only, so it verified game-icons.net and Google Fonts from their git repositories. The second (2026-10-13) had web access through an egress proxy. It read the licence pages for Kenney, Poly Haven, Quaternius, ambientCG (home page and API; its separate licence page was blocked), OpenGameArt entries, the game-icons.net about page, and the Creative Commons CC BY 3.0 deed and legal code and the CC0 1.0 deed. It also downloaded and inspected the Kenney Cartography Pack zip, including its `License.txt`.

Still blocked by the proxy, with a 403 or DNS failure, so these remain **unverified**: every `*.itch.io` creator page, kmalexander.com, cartographyassets.com, deviantart.com, and docs.ambientcg.com. Search-engine snippets about those hosts are listed under "Unverified candidates" as leads only. They are not findings.

## Verified findings

### 1. Game-icons.net icon set (SVG): CC BY 3.0, attribution required

- Source: https://github.com/game-icons/icons at commit `82d948812bfe3f269ef8f731dcdb07b08160edc4` (2026-04-23).
- `license.txt` (repo root): "Icons provided under the Creative Commons 3.0 BY or CC0 if mentioned below." It then lists contributors, one sub-folder each. Only Viscious Speed and Zeromancer are marked CC0.
- `CONTRIBUTING.md`: "Most of them are released under the Creative Commons BY 3.0, others as Public Domain (CC0)."
- `README.md` licence section says only "CC-BY".
- Format: 4239 SVG files, 512x512 viewBox, single-path, white shape on a black square background. The repo ships `colorize-svgs.sh` (recolour, transparent background) and `rasterize-svgs.sh` (PNG).
- Style: monochrome glyph icons, varied by contributor (Lorc, Delapouite and others). Consistent as a UI/icon set, not as top-down or side-view map art.
- Map-relevant examples present: `delapouite/castle.svg`, `village.svg`, `huts-village.svg`, `castle-ruins.svg`, `camping-tent.svg`, `palm-tree.svg`, `mountain-cave.svg`, `fruit-tree.svg`, `bonsai-tree.svg`; `badges/mountain.svg`.
- https://game-icons.net/about.html (read 2026-10-13): "provided under the terms of the Creative Commons 3.0 BY license … you can use them freely as long as you credit the original author in your creation". It suggests the credit line "Icons made by {author}. Available on https://game-icons.net".
- CC BY 3.0 terms, read from https://creativecommons.org/licenses/by/3.0/ (deed) and `/legalcode`:
  - The deed allows sharing and adapting "for any purpose, even commercially".
  - "You must give appropriate credit, provide a link to the license, and indicate if changes were made", with no endorsement implied.
  - "You may not apply legal terms or technological measures that legally restrict others."
  - Legal code 4(a): "You must include a copy of, or the Uniform Resource Identifier (URI) for, this License with every copy of the Work You Distribute".
  - Legal code 4(b): credit "may be implemented in any reasonable manner". For an Adaptation or Collection, credit must appear "at a minimum … as part of these credits and in a manner at least as prominent as the credits for the other contributing authors".
- Obligation: credit each icon's author, ship the licence URI, and say when an icon was recoloured or modified. An in-app credits screen plus per-asset metadata (author, source, licence, modified flag) meets this. The "no technological measures" clause means encrypted or DRM-locked asset bundles would conflict for CC BY icons. CC0-marked contributor folders need no attribution.
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

### 3. Kenney Cartography Pack: CC0, the best stamp set found

- Sources: https://opengameart.org/content/cartography-pack (licence field "CC0", author "Kenney"), https://kenney.nl/assets/cartography-pack (licence "Creative Commons CC0"), and `License.txt` inside the downloaded `cartographypack.zip` (4.2 MB, created 16-06-2020): "License: (Creative Commons Zero, CC0) … free to use in personal, educational and commercial projects. Support us by crediting Kenney or www.kenney.nl (this is not mandatory)".
- Kenney-wide policy, from https://kenney.nl/support: "all game assets on the asset pages are public domain licensed (CC0) … free to use them, even in commercial projects … Attribution is not required". The same page also says: "Do not use our logo, as it is reserved for official projects by our studio."
- Contents, from inspecting the zip:
  - 83 PNG stamps in `PNG/Default` (64x64) and `PNG/Retina` (128x128), plus a spritesheet with XML atlases.
  - One combined SVG (`Vector/cartographyPack_vector.svg`, 162 paths) and an SWF.
  - Five 1024x1024 parchment textures: `parchmentBasic`, `Ancient`, `Folded`, `Crinkled`, `FoldedCrinkled`.
- Map-relevant stamps:
  - Mountains and rocks: `rocksMountain`, `rocks`, `rocksA`, `rocksB`, `rocksTall`, `vulcano`.
  - Trees: `treePine*` (several), `treeTall`, `treePines`, `palm`, `palmLarge`, `bush`, `cactus`.
  - Buildings: `castle*`, `house*`, `houses`, `church*`, `tower*`, `watchtower`, `mill`, `waterWheel`, `lighthouse`, `stable`, `mine`, `well`, `dock`, `bridge*`, `gate`, `wall`, `graveyard`, `pyramid`, `runis` (sic), `tent`, `tipi`, `campfire`, `ship`.
  - Other: path and arrow pieces, `compass`, `banner`, `flag`, and `lake` and texture marks.
- Style: one consistent hand-drawn dark-ink line style (stroke `#2A2A2A`) meant for parchment (see the pack's `Sample.png`). This is the classic Wonderdraft/Inkarnate "ink on paper" look.
- Caveats:
  - The stamps are outlines only. Every SVG path is `fill="none"` and PNG interiors are transparent, so overlapping stamps show lines through each other. Wonderdraft-style mountain ranges and forests need an opaque fill or a mask. That can be generated (flood-fill the interior with the paper colour) but it is work.
  - The SVG is a single sheet with no `viewBox`, so it has to be split into per-stamp SVGs.
  - Resolution is low (128 px max) for PNG. Use the vectors for zoom.
  - Variety is thin for a full tool: about 6 mountain/rock and about 10 tree variants, with no hills or grassland marks.
- Fit: a strong CC0 starter set for terrain, trees, buildings and parchment in one style. Using it as the default set would let original art extend it in the same ink style, not replace it.

### 4. Poly Haven: CC0 textures (photoreal, poor fit)

- Source: https://polyhaven.com/license: "Our assets are all licensed as CC0 … You can use our assets for any purpose … including commercial work … You do not need to give credit … You can redistribute them" including in "a product you sell". Logos, example renders and site text are excluded and remain protected.
- Format/style: photoreal PBR textures, HDRIs and 3D models (per the licence page). Usable as raw material for a parchment or ocean backdrop after processing, but not stylised map art.

### 5. ambientCG: CC0 statement on the home page; licence page blocked

- Source: https://ambientcg.com/ home page: "All assets are released under the Creative Commons CC0 license, making them free to use without attribution … even in commercial circumstances." The full licence page (https://docs.ambientcg.com/license/, redirected from ambientcg.com/license) was blocked by the proxy and was **not read**. Read it before shipping in case it adds conditions (for example on reselling the assets themselves).
- Formats: the public API (`/api/v2/full_json?category=Paper`) lists 6 paper materials (`Paper001` to `Paper006`), each as PBR material zips in 1K/2K/4K JPG or PNG.
- Fit: photographic paper textures as optional backdrops. The Kenney parchments match the stamp style better.

### 6. Quaternius: own licence (QAL v1.0), not CC0, 3D only; do not use

- Source: https://quaternius.com/license.html (Quaternius Asset License v1.0):
  - Allows use "in personal, educational, and commercial games", with "No attribution is required".
  - Forbids anyone to "extract, repackage, sublicense, sell, or otherwise redistribute the Assets" as standalone items.
- https://quaternius.com/ lists 3D packs (rigged, animated). There are no 2D assets.
- Fit: none. The packs are 3D, the licence is scoped to "games", and a stamp library that users can export from could read as repackaging.

### 7. OpenGameArt: per-entry licences, checked entries

- "Fantasy World Map" by yd (https://opengameart.org/node/23938) is licensed **CC0**. It is a single 16.2 MB GIMP `.xcf` with brushes the author says were made from CC0 British Library Flickr scans of old maps. It is not stamps, so the brushes would need extracting from the XCF. The style is vintage engraving, which clashes with Kenney ink. Usable, but low priority.
- "B&W Wilderness Map Icons" by Mophs (https://opengameart.org/node/31488) has licences listed as CC-BY 3.0, CC-BY-SA 3.0, GPL 3.0, GPL 2.0 and OGA-BY 3.0. It is one PNG sheet plus a PSD of rocks, trees and mountains. Under CC-BY 3.0 it is usable with attribution, as for game-icons. It is a small set.
- OpenGameArt licences vary per entry, so each asset still needs its own check.

## Unverified candidates (licence not read; research only)

These hosts were blocked by the proxy. The snippets below came from search-engine results, not primary pages. They are leads, not findings.

| Candidate | Wanted for | Search-snippet claim (unverified) | Read before use |
| --- | --- | --- | --- |
| K.M. Alexander fantasy map brushes (kmalexander.com) | mountains, hills, forests, settlements | "CC0", free for commercial use; Photoshop brush sets | site licence text, plus whether brushes can be converted to PNG/SVG stamps |
| Zarkonnen "Extracted 1688 map images" (itch.io) | hills, mountains, trees, towns | public domain, extracted from a 1688 map of Germany | pack page and source scan provenance |
| Wuggy's Classic Cartography Set (itch.io) | mountains, trees | CC BY 4.0, hand-drawn ABR brushes and images | pack page |
| under_score_lab "cartography bigpack" (itch.io) | settlements, mountains, hills, trees | CC BY 4.0, based on historical engravings | pack page |
| Fantasy Map Design's Simple Stamps (cartographyassets.com) | mountains, trees, symbols | CC BY 4.0 | asset page |
| SPOICAT "Map Legend Icons 2" (itch.io) | map legend icons | listed under itch.io's CC0 tag; contents unknown | pack page |
| ambientCG full licence page | textures | see finding 5 | https://docs.ambientcg.com/license/ |

## Coverage against Wonderdraft-style categories

| Category (from the map's v1 scope) | Verified source | Status |
| --- | --- | --- |
| Labels / fonts | OFL families (finding 2) | covered |
| POI icons (towns, castles, camps) | Kenney Cartography (CC0) and game-icons.net (CC BY) | covered. Kenney matches the terrain style; game-icons is a glyph style and needs attribution |
| Mountains | Kenney Cartography (about 6 variants, CC0) | partly covered: style-consistent but few variants, outline only (needs fill/mask); expand with original art |
| Hills | none verified | uncovered; original art (or K.M. Alexander once verified) |
| Trees / forests | Kenney Cartography (about 10 variants, CC0) | partly covered: same caveats as mountains; forest clusters need scattering logic or original art |
| Buildings / settlements (map-scale) | Kenney Cartography (castles, houses, church, towers, mill, etc., CC0) | covered for a starter set |
| Decorations (compass, banners, paths) | Kenney Cartography (CC0) | covered for a starter set; no cartouches, frames or sea monsters |
| Parchment texture | Kenney Cartography parchments (5, 1024 px, CC0); ambientCG paper (CC0 per home page) | covered |
| Ocean backdrop | none map-styled; Poly Haven / ambientCG photo textures (CC0) | uncovered as styled art; procedural or original |
| Lakes, coastline styling | n/a (procedural); Kenney has a `lake` stamp | no asset needed |

## Recommendations for the map

1. Treat fonts as settled: bundle OFL families with their `OFL.txt` files.
2. Adopt the **Kenney Cartography Pack (CC0)** as the default terrain, building and parchment set. It fixes the house style ("dark ink outline on parchment"), so original art should match that style rather than compete with it. Budget work to split the SVG sheet into per-stamp SVGs and to add opaque interior fills or masks so stamps can overlap.
3. Plan original art (in the Kenney ink style) for hills, more mountain and tree variants, forest clusters, ocean styling, cartouches and frames. These are the gaps.
4. Use game-icons.net (CC BY 3.0) only as an optional marker or icon layer, with a credits screen listing each author, the licence URI and a note when icons were recoloured. Do not lock asset bundles with DRM, because CC BY forbids technological restrictions.
5. Skip Quaternius (3D, QAL licence with redistribution limits). Poly Haven and ambientCG are CC0 raw material for backdrops only.
6. Asset metadata should carry `author`, `source_url`, `licence` and `modified` per asset so mixed CC0 and CC BY content stays auditable.
7. Before bundling anything from the unverified table, read its page from an unblocked network. K.M. Alexander (claimed CC0, with hills and settlements) is the most valuable lead to confirm.
