# Lyon is the only vector renderer; no renderer trait

Status: accepted. Supersedes the renderer-swap part of [ADR 0001](0001-rust-stack-and-monorepo-layout.md).

ADR 0001 put vector rendering "behind a `VectorRenderer` trait" so that a later benchmark could swap renderers. The code no longer works that way. `CanvasRenderer` holds the concrete `LyonRenderer` and calls its inherent methods for runs, chunks and the tessellation cache; the trait covered only the backdrop. Swapping in another renderer would have meant rewriting `canvas.rs`, so the trait gave readers a false picture of the design.

**Decision:** `LyonRenderer` is the only vector renderer, and `CanvasRenderer` uses it directly. The `VectorRenderer` trait and its re-export are removed. Lyon tessellation, 4x MSAA and stencil Clip from ADR 0001 stand.

If a second renderer (such as `vello_gpu`) is ever benchmarked, design a trait then, from what both renderers actually need, and record it in a new ADR.
