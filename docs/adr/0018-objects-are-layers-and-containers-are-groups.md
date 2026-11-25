# Every Object is a Layer, and containers are Groups

Status: accepted (amends ADR 0014; builds on ADR 0004 and ADR 0005)

ADR 0014 put every Object inside a Layer, and made a Layer when the Map had none. Drawing one Landmass on a new Map then shows two rows, one of which the user never asked for. Photoshop and Figma let an object sit at the top of the list.

## Decision

- **Terms.** A Layer is one row in the Layers panel, and every Object is its own Layer. A Group is a container that holds Objects and other Groups. Object stays the term for the thing placed on the Map.
- **Objects can sit at the top level**, beside Groups in one mixed list, which is the Stacking order. A new Map starts with nothing in the list, and tools never create a Group. This replaces "If the Map has no Layer, one is made" in ADR 0014. When a Group is active, new Objects still go into it.
- **Settings.** An Object at the top level has only its own settings. Attach to Layer Below works on any row, not only a Group.
- **No active Group:** Select All, the Stamp Eraser and Paste act on the top level.
- **Water at the top level.** Inside a Group, Lakes and Rivers draw above its other content. At the top level they draw above the other Objects of their run, the Objects with no Group between them, so water still lies over the land beside it and Groups keep their place in the order.
- **File format.** An Object row with a NULL `parent` is format 1.20, a minor bump with no migration. Older files open unchanged. An older app can't open a Map that has a top-level Object.

## Consequences

- The work is staged (#640): core allows top-level Objects; Commands and render; tools and new Maps; the Layers panel and UI text; then renaming the container from Layer to Group in code. Until that last stage, `Node::Layer` and names built from it mean a Group.
