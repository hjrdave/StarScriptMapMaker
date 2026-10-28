# Golden images

Compared by `crates/render/tests/golden.rs`, only on a software adapter (lavapipe, WARP).

Last rewritten with Mesa 25.2.8 lavapipe (Vulkan, LLVM 20.1.2), the newest Mesa in Ubuntu 24.04's
archive. Other Mesa versions differ in the antialiasing of edges and text, and some fail the
tolerance. To rewrite them after an intended visual change, or to move to a newer Mesa:

```sh
SSMAP_GOLDEN_UPDATE=1 cargo test -p ssmap-render --test golden
```

Look at the changed PNGs before committing them. Say the Mesa version in the commit message.
