# Performance: the frame-time budget and how to measure it

Editing must feel smooth: no visible lag while painting land, dragging objects, moving sliders,
panning or zooming, even on large maps with high shoreline roughness (issue #245).

## The budget

Agreed by the owner on issue #245:

- **60 fps is required:** a frame under **16.6 ms at p95**.
- **120 fps is a stretch goal** for 120 Hz displays: under **8.3 ms at p95**.
- **No run of slow frames:** no frame over about **33 ms** during a continuous gesture. A single
  dropped frame is acceptable, a run of them isn't.
- **Input latency:** the preview follows the mouse with no extra frame of lag.
- Reference hardware: an Apple-silicon Mac, on the generated large Map (100k Stamps, about 1M
  coastline vertices).

Heavy work may run on a background thread, and a live preview may show the previous result for a
frame or two until the new one is ready.

## Running the measurements

The measurement is the app binary's `--frame-time` mode. It has no window: it builds the large
Map, renders a fixed script offscreen, prints the statistics and exits.

```sh
# One script, on the real GPU (build with --release for numbers that mean anything)
cargo run --release -p app -- --frame-time --script paint

# Every script, one after the other
cargo run --release -p app -- --frame-time --script all

# A small Map on the software adapter, as CI does
cargo run -p app -- --frame-time --software-adapter --scale 0.01 --frames 2 --script paint
```

| Option | Meaning |
| --- | --- |
| `--script NAME` | Which script to run: `all`, or one of the names below. The default is `pan-zoom`. |
| `--scale F` | Shrinks the Map to a fraction `F` (above 0, at most 1). |
| `--frames N` | Stops each script after `N` frames, for quick runs. |
| `--software-adapter` | Forces the platform's software adapter. |

### The scripts

Each script is a fixed sequence of input, one step per frame, so runs are comparable. (Only the
ids of new objects are random.) The editing scripts run on the generated Map with a high
Map-wide Roughness (amount 60, scale 200) and frame one landmass at about 4x zoom.

| Script | One frame is |
| --- | --- |
| `pan-zoom` | A step of the zoom in, pan and zoom out path (issue #94). No roughness. 180 frames. |
| `paint` | A land brush segment merged into a rough landmass, along its coast. |
| `erase` | An erase brush segment cut out of the landmasses beneath it. |
| `drag-landmass` | A drag step of one rough landmass (`Translate`). |
| `drag-stamps` | A drag step of 2000 Stamps (`SetStamp` each). |
| `scatter` | A scatter stroke of the Stamp tool at density 40. |
| `slider-amount` | A new roughness Amount, ramping from 25% to 175% of the base value. |
| `slider-scale` | A new roughness Scale, ramping from 50% to 200%. |
| `slider-shore` | A new Shore width, ripple spacing and shading width. |
| `marquee` | A marquee rectangle growing from the middle to the whole Canvas, on the whole-Canvas view. |

The editing scripts run 90 frames. A first frame is rendered before measuring, to upload the Map
and roughen every outline once; it isn't counted.

Not measured yet: resizing the Canvas with many objects (issue #239), because there is no
resize Command yet. Add a script when there is.

## Reading the output

```
adapter: Apple M1
backend: Metal
...

script: paint
frames: 90
mean: 6.10 ms
p50: 5.80 ms
p95: 9.40 ms
p99: 14.20 ms
worst: 18.90 ms
budget: 60 fps (p95 under 16.6 ms) met, 120 fps (p95 under 8.3 ms) missed, 0 frames over 33 ms
stages (ms)      p50      p95      p99    worst
  edit             0.90     1.60     2.10     2.40
  scene            2.30     4.10     6.00     9.00
  render           2.50     3.30     5.00     6.00
```

- **Judge by p95, p99 and the worst frame, not the mean.** Stutter is what people notice.
- The **stages** say where a regression comes from. They are timed apart, so each is a share of
  the frame:
  - `edit`: applying the input to the Map: brush geometry, the Command, picking.
  - `scene`: rebuilding the scene from the Map: roughness generation and tessellation.
    An edit that changes Objects in place (a property set, an outline replaced) redraws only
    those Objects and keeps the rest (`Map::changes_since`), as does the landing of their shore
    rings. Adding, removing or moving Objects, Layer and Map-wide edits, and Maps with clipped
    Layers rebuild it all.
    The rings shore ripples and shading follow are computed on a thread of their own, as in
    the editor, so an edit that changes an outline shows its old rings for a frame or a few
    until the new ones land. Their cost does not appear here; export waits for them.
  - `render`: upload and draw, until the GPU reports it is done.
- For a finer breakdown build with the `profiling` feature (next section).
- The frame includes waiting for the GPU to finish. The editor overlaps the CPU and the GPU, so a
  real frame can be shorter than the sum, but the measurement is the same each time.
- Numbers from the software adapter only show that the scripts run, and big relative changes.
  Record real-GPU numbers per OS in the PR description when a change is meant to affect speed.
- Compare a change against `master` on the same machine, with nothing else running.

## Per-stage timing (the `profiling` feature, issue #309)

The render path marks six stages with `tracing` spans. They exist only in a build with the
`profiling` feature, so a normal build, release included, has no spans and pays nothing
(`scripts/check-profiling-off.sh` checks that; add `--binary` to also build the release binary
and search it for the spans).

```sh
cargo run --release -p app --features profiling -- --frame-time --script paint
```

The `stages` table then has these rows in place of `edit`, `scene` and `render`:

| Stage | What it covers |
| --- | --- |
| `input` | Applying the frame's input: brush geometry, Commands, picking. |
| `update` | Rebuilding the scene from the Map, apart from roughness. |
| `roughness` | Generating rough outlines (a cached outline costs almost nothing). |
| `tessellation` | Turning Objects into triangles (only the Objects that changed). |
| `upload` | Merging triangles, uploading buffers and Stamps, the view uniform. |
| `draw` | Recording the pass, submitting it and waiting for the GPU. |

Reading it:

- A stage's time leaves out the stages inside it (roughness inside `update`, tessellation inside
  `upload`), so the rows add up to the frame. The last line, `stages add up to N% of the frame
  time`, checks that: it should be close to 100%. A low number means time is spent outside the
  marked stages.
- Start with the row that is largest at p95, then compare its p99 and worst: a stage whose worst
  is far above its p95 is the one that causes hitches.
- Under the editing scripts `roughness` and `tessellation` show the cost of an edit, since only
  changed Objects are rebuilt. A large `tessellation` p50 on `pan-zoom` means the cache is not
  being reused.

### The overlay

A build with the `profiling` feature, or a debug build, can show the frame time, frames per
second and (with the feature) the six stages of the last frame in the corner of the editor. It is
off by default; press **F3** to toggle it. The frame time turns red above 16.6 ms. It measures
the time between frames while the editor is repainting, so it reads the real frame time during a
drag, a stroke or a zoom, not while the editor is idle. Release builds without the feature have
no overlay.

## The regression check (issue #338)

`docs/perf-baseline.json` holds the p50, p95 and p99 frame time of every script, plus the same
for the `edit`, `scene` and `render` stages, measured with the *reduced* run: the software
adapter, `--scale 0.01` and 10 frames per script. The check runs that again and compares:

```sh
cargo run --release -p app -- --check-baseline
```

It prints every script against its baseline and exits with 3 if any frame percentile regressed,
0 otherwise (1 for a missing or unreadable baseline, 2 for no adapter). A number regresses only
when it is both more than 3 times the baseline *and* more than 5 ms above it. The margin is wide
on purpose: CI runners are noisy and software rendering is slow, so the check catches a script
getting several times slower, not drift. The stage rows are shown for scripts that regressed, to
point at the cause. `--baseline-file PATH` reads a different file.

The nightly job runs the check from `docs/perf-nightly.patch`; apply it with
`git apply docs/perf-nightly.patch` (it edits `.github/workflows/nightly.yml`, which only the
owner changes).

### Refreshing the baseline

Refresh it when a change is meant to alter the numbers (a fix that makes a script faster, or a
feature that costs time on purpose), or when the nightly check fails with no code change because
the runner got slower:

```sh
cargo run --release -p app -- --write-baseline
```

This rewrites `docs/perf-baseline.json` from a fresh reduced run. Commit it with the change and
say why in the PR. Run it on a quiet machine, ideally the same kind as the nightly runner
(Linux x86_64), since the check compares against these numbers; run it twice and keep the
slower result if the two differ much.

## Still to come (issue #245)

The fixes the numbers point to (step 4).
