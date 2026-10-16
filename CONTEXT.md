# Fantasy Map Maker

A native, cross-platform desktop application for drawing fantasy maps. Its defining idea is Figma-style layers: everything on a map stays editable, nothing is baked into pixels.

## Language

**Map**:
One document the user creates and edits, made of a Canvas and a tree of Layers.
_Avoid_: Project, file, document

**Canvas**:
The bounded drawing area of a Map, with a fixed size and a real-world scale chosen when the Map is created.
_Avoid_: Board, artboard, page

**Object**:
Any single thing placed on a Map, such as a coastline, a placed asset or a label.
_Avoid_: Element, item, shape, node

**Layer**:
A container in the Map's ordered tree that holds Objects and other Layers, and has a name, visibility, lock and opacity.
_Avoid_: Folder, frame

**Group**:
A Layer the user creates to move, hide or clip several Objects together. Layers nest, so a Group is just a nested Layer.
_Avoid_: Folder, collection

**Link**:
A relationship that makes Layers and Objects move and transform together while each keeps its own place in the tree, even in different Groups. A locked member blocks the move for the whole set.
_Avoid_: Bind, attach, constraint

**Stacking order**:
The single top-to-bottom order of the Map's tree that decides what draws above what.
_Avoid_: Z-order, depth

**Clip**:
A relationship that restricts a Layer to the shape of a Landmass, so its contents show only where there is land and not where a Lake covers it.
_Avoid_: Mask

### Land and water

**Landmass**:
An Object that is a filled region of land, made of one or more parts, each with an outer boundary and optional holes that show ocean.
_Avoid_: Island, continent, land shape

**Coastline**:
The boundary of a Landmass. It is not an Object of its own; it is what shore styling and Roughness act on.
_Avoid_: Shore path, outline

**Lake**:
An Object that is a closed region of fresh water drawn above land, separate from any Landmass.
_Avoid_: Pond, hole

**Roughness**:
Generated fractal detail added to a Coastline from an amount, a scale and a seed, so a smooth stored outline renders as a jagged coast.
_Avoid_: Noise, jitter, fractalisation

**Shore style**:
The set of effects drawn along a Coastline: outline, ripple lines and inner shading. A Map has a default, and each Landmass or Lake may override it.
_Avoid_: Coast effect, border style

### Assets

**Asset**:
A reusable piece of map art, such as a mountain or a castle, defined in an Asset pack.
_Avoid_: Symbol, sprite, prop, icon

**Asset pack**:
A named bundle of Assets with a manifest that records each Asset's metadata, author and licence.
_Avoid_: Library, set, collection

**Stamp**:
An Object that places one Asset on a Map, with its own position, scale, rotation, flip, tint, fill colour and opacity.
_Avoid_: Instance, symbol, placed asset

**Anchor**:
The point of an Asset that sits on the ground, such as a mountain's base centre. Stamps are positioned, rotated, scaled and depth-sorted by it.
_Avoid_: Origin, pivot, hotspot

**Variant group**:
A set of interchangeable Assets, such as six mountain peaks, from which brushes pick at random.
_Avoid_: Variation, family

**Silhouette**:
The inside area of an Asset, filled with a colour beneath its line art so that overlapping Stamps hide one another.
_Avoid_: Mask, backing

**Depth sort**:
A Layer setting that draws the Stamps directly inside it by their Anchor's height on the Map, lower in front, instead of by tree order.
_Avoid_: Y-sort, z-sort

### Editing

**Command**:
One named edit to a Map, such as placing Stamps or uniting two Landmasses. Every change to a Map, whether from the user or an agent, is a Command.
_Avoid_: Action, operation, edit, mutation

**Undo step**:
The group of Commands that one gesture or one agent action produced, undone and redone together.
_Avoid_: History entry, transaction

**Lock**:
A setting on a Layer or Object, inherited by everything inside a locked Layer, that makes it refuse all Commands until it is unlocked.
_Avoid_: Freeze, protect

**Selection**:
The Objects the user currently has picked in the editor. It is part of the editor, not of the Map, and Commands never refer to it.
_Avoid_: Focus, active objects

### Tabs

**Tab**:
One open Map in the window, with its own editor, unsaved-changes state and view. Asset packs, Recent files and the export dialog are shared by all tabs and act on the active one.
_Avoid_: Document, window, project

**Home tab**:
The tab that shows the map browser. It is not a Map and has no Canvas.
_Avoid_: Start page, launcher

### Saving

**Map file**:
The single `.ssmap` file that stores one Map together with every Asset and font it uses, so it opens identically anywhere.
_Avoid_: Project file, save file, document

**Recovery journal**:
The record of edits made since the last save, kept beside a Map file so that work survives a crash.
_Avoid_: Autosave file, backup, swap file

### Labels

**Label**:
An Object that places text on a Map, sized in Canvas units so it is part of the picture.
_Avoid_: Text, caption, annotation

**Label style**:
A named set of typography and colour settings stored in a Map, such as "Realm" or "City". Each Label uses one and may override any of its settings.
_Avoid_: Text style, font preset, class
