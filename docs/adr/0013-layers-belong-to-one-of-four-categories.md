# Layers belong to one of four fixed categories

Status: proposed (amends ADR 0002 and ADR 0005; builds on ADR 0004)

Today a Layer holds any Objects and Layers, and `Map::accepts_children` only checks that the target is an unlocked Layer. Issue #522 asks that an Object can only go into a Layer of its own kind, and #531 groups the Layers panel under categories that have their own Hide, Lock, Pin and Opacity. Both need the same model, so this record decides it once. It changes the file format, so it needs a migration; no code changes until this is accepted.

## Decision

- **A Map has four built-in categories**, always in this order from top to bottom: **Labels**, **Assets**, **Land**, **Backdrop**. They are fixed: users cannot add, remove, rename or reorder them. "The Layer below" a Layer follows the list across categories.
- **Each category is a container** with its own `visible`, `locked`, `pinned` and `opacity`. These apply to every Layer inside it. A Layer keeps its own settings too; the category's are inherited, and inherited state cannot be undone from the Layer. A category's opacity multiplies with each Layer's opacity.
- **Every top-level Layer belongs to exactly one category**, stored as `category` on the Layer. Layers nested inside a Group take the category of the Group. A Group's category is the category of what it holds, so a Group cannot hold Layers of two categories.
- **An Object may only go into a Layer of its category.** The rule lives in `crates/core`, so every way of moving, creating, pasting or importing an Object obeys it, and a Command that breaks it fails with a `CommandError`. The Layers panel only shows the refusal; it does not enforce it.

| Object kind | Category |
| --- | --- |
| Label | Labels |
| Stamp | Assets |
| Landmass, Lake, River | Land |
| (none yet) | Backdrop |

## Existing Maps

- A migration adds the categories and gives every Layer one. A Layer holding a single kind gets that kind's category. A **mixed Layer goes to Land**, and keeps its Objects, so nothing is lost and the stacking order inside it is unchanged. An empty Layer takes the category from its name when it is one of the starting names (Labels, Assets, Land, Backdrop), and Land otherwise.
- Moving the Layers into the fixed category order can change the Stacking order of a Map that interleaved kinds, for example Labels between Stamp Layers. The migration writes `<map>.v<old>.bak` first (ADR 0005). Stacking inside each category is kept.
- This is a **major** format bump: an older app would draw hidden or locked categories as visible and unlocked.

## Rules for the rest of the model

- **Templates** give each starting Layer a category.
- **Tools** put a new Object in the first Layer of its category in the Stacking order, and create a Layer of that category only when there is none. Paste and Import follow the same rule, per Object.
- **Links** may join Objects and Layers of different categories; the rule only restricts which Layer an Object sits in.
- **Clip** keeps its current meaning in this record: a Layer clips to one Landmass. How "Attach to Layer Below" clips to the Landmasses in the Layer below is decided with #531.
- **Pin** belongs to Objects today (ADR 0004's move lock, shown as Pin to Canvas). A category's pin pins every Object inside it.

## Open

- **Backdrop holds nothing yet.** The Map's background is a setting on the Canvas and Art boards, not an Object. Either Backdrop is a category with no Object kind until Backdrop images exist (it shows in the panel but cannot take a drop), or it is left out of the first release and the panel shows three categories. This needs an answer before the code starts.
- **Collapsed state** of each category in the panel is saved per Map with the other panel state; it is not part of the model.

## Considered options

- **Categories as normal group nodes:** rejected. Users could then rename, add and reorder them, which conflicts with a fixed "below" and with the locking rule.
- **A free category per Layer (`Option<Category>` with five kinds, one per Object kind):** rejected. It gives no place for the category's own Hide, Lock, Pin and Opacity, and Landmass, Lake and River share one panel category.
- **Splitting mixed Layers on load:** rejected. It reorders more of the Stacking order than moving them to Land.

## Consequences

- The Layers panel can group by category without guessing, and "a Landmass cannot go into a Stamp Layer" is one check in core.
- The category settings add inherited state to the render path (hidden, locked, pinned, opacity), which render must read the same way as it reads a Layer's.
- Older apps refuse the new major version with a clear message.
