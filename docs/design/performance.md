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

Not measured yet: resizing a board or Canvas with many objects (issue #239), because there is no
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
  - `render`: upload and draw, until the GPU reports it is done.
- The frame includes waiting for the GPU to finish. The editor overlaps the CPU and the GPU, so a
  real frame can be shorter than the sum, but the measurement is the same each time.
- Numbers from the software adapter only show that the scripts run, and big relative changes.
  Record real-GPU numbers per OS in the PR description when a change is meant to affect speed.
- Compare a change against `master` on the same machine, with nothing else running.

## Still to come (issue #245)

Stage timing with `tracing` spans and an optional on-screen overlay, a regression check against a
checked-in baseline (`docs/perf-baseline.json`) in the nightly job, and the fixes the numbers
point to. Changes under `.github/` need the owner, so the workflow change will be a patch file
under `docs/`.
