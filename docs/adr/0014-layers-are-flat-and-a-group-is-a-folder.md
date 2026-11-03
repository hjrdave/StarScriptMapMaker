# Layers are one flat list, and a Group is a folder

Status: proposed (supersedes ADR 0013; builds on ADR 0004)

ADR 0013 gave every Layer a fixed category (Labels, Assets, Land, Backdrop) and refused an Object in a Layer of another kind. The code for it was built and then dropped: it made new Layers and pasting harder, moved a Map's Layers into a fixed order, and put a rule in core that most of the app's own tests had to work around. The Layers panel is a flat list instead, as in Photoshop or Figma. This record keeps what #531 and #522 still need from that design.

## Decision

- **Layers are not categorised.** A Layer holds any Objects. There is no `category`, no fixed order of Layers, and no rule about which Object may go into which Layer. `Map::accepts_children` stays as it is: the target must be an unlocked Layer.
- **The user orders Layers freely**, by dragging, in one list from the top of the Stacking order down.
- **Grouping makes a folder.** Selecting Layers (or Objects) and choosing Group puts them in a new Group, a Layer that holds Layers, which the panel shows as a folder that folds. This is the existing Group Command; it needs no change.
- **New Objects go in the active Layer.** If the Map has no Layer, one is made. The tools no longer pick a Layer by kind, so the template's default Layers (`DefaultLayers`) only decide where a new Map's first Objects go.
- **The Layers panel** (#531) is a flat tree of Layers and Groups with: one settings row above the list (Opacity, Hide, Lock, Pin) that acts on the selected Layer or Group; state icons on each row; and **Attach to Layer Below** in the right-click menu. Details are in #531.
- **Pin on a Layer.** Pin to Canvas (ADR 0004's move lock, shown as a pin) becomes a setting of a Layer as well as of an Object. A pinned Layer pins everything inside it, as a locked Layer locks everything inside. This adds an optional `pinned` on a Layer, so it is **format 1.12, a minor bump**: an older app keeps the field it does not read.
- **A Group's settings are inherited.** Hide, Lock and Pin on a Group apply to everything inside it, and Opacity multiplies. The Layers inside show the inherited state as the same icon, dashed and dimmed, and cannot undo it from the Layer.
- **Attach to Layer Below** clips a Layer to the Landmasses in the Layer below it in the list, within the same parent. A Layer that is attached shows Release instead. This changes the clip from one Landmass to a rule over the Layer below; how it is stored (a new optional field, replacing `clip`) is decided with the Command in #531's last stage. The item is disabled on the last Layer of a parent.

## Consequences

- Nothing from ADR 0013 is merged, so no Map needs migrating and format 1.11 stays the current format until Pin on a Layer lands.
- The Layers panel plus makes a plain Layer; "New Label Layer" and its kin are gone. #522 (an Object only goes into Layers of its kind) is closed as not planned.
- The panel work of #531 is staged: Pin on a Layer and inherited state in core, then the settings row and state icons, then Attach to Layer Below.
