//! Per-stage frame timing (issue #309). The render path marks its stages with [`stage`]; with
//! the `profiling` feature each mark is a `tracing` span and a [`Recorder`] adds up the time
//! spent in each stage. Without the feature [`stage`] returns an empty guard and nothing is
//! compiled in, so release builds pay nothing.
//!
//! Stages nest (roughness runs inside the Map update, tessellation inside upload). A stage's
//! time excludes the stages inside it, so the stage times of a frame add up to its length.

use std::ops::{Index, IndexMut};
use std::time::Duration;

/// The parts of a frame that are timed apart.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    /// Applying the frame's input: brush geometry, Commands, picking.
    Input,
    /// Rebuilding the scene from the Map, apart from roughness.
    Update,
    /// Generating rough outlines.
    Roughness,
    /// Tessellating Objects into triangles.
    Tessellation,
    /// Preparing and uploading geometry, Stamps and uniforms to the GPU.
    Upload,
    /// Recording the draw, submitting it and (offscreen) waiting for the GPU.
    Draw,
}

impl Stage {
    pub const ALL: [Stage; 6] = [
        Stage::Input,
        Stage::Update,
        Stage::Roughness,
        Stage::Tessellation,
        Stage::Upload,
        Stage::Draw,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Stage::Input => "input",
            Stage::Update => "update",
            Stage::Roughness => "roughness",
            Stage::Tessellation => "tessellation",
            Stage::Upload => "upload",
            Stage::Draw => "draw",
        }
    }
}

/// Time spent in each [`Stage`].
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct StageTimes([Duration; Stage::ALL.len()]);

impl StageTimes {
    /// All the stages together.
    pub fn total(&self) -> Duration {
        self.0.iter().sum()
    }
}

impl Index<Stage> for StageTimes {
    type Output = Duration;
    fn index(&self, stage: Stage) -> &Duration {
        &self.0[stage as usize]
    }
}

impl IndexMut<Stage> for StageTimes {
    fn index_mut(&mut self, stage: Stage) -> &mut Duration {
        &mut self.0[stage as usize]
    }
}

/// Adds up stage times from enter and exit events. `at` is the time since any fixed start.
#[derive(Default)]
pub struct StageClock {
    /// Open stages: the stage, when it was entered and the time of the stages inside it so far.
    open: Vec<(Stage, Duration, Duration)>,
    times: StageTimes,
}

impl StageClock {
    pub fn enter(&mut self, stage: Stage, at: Duration) {
        self.open.push((stage, at, Duration::ZERO));
    }

    /// Ends the innermost open stage; does nothing if none is open.
    pub fn exit(&mut self, at: Duration) {
        let Some((stage, began, inner)) = self.open.pop() else {
            return;
        };
        let whole = at.saturating_sub(began);
        self.times[stage] += whole.saturating_sub(inner);
        if let Some(parent) = self.open.last_mut() {
            parent.2 += whole;
        }
    }

    /// The times of the stages that have ended since the last call, which starts over.
    pub fn take(&mut self) -> StageTimes {
        std::mem::take(&mut self.times)
    }
}

/// Keeps a stage open until dropped.
#[must_use = "the stage ends when the guard is dropped"]
pub struct Guard {
    #[cfg(feature = "profiling")]
    _span: tracing::span::EnteredSpan,
}

/// Marks the rest of the scope as `stage`. With the `profiling` feature and a [`Recorder`]
/// installed it is timed; otherwise it does nothing.
#[cfg(feature = "profiling")]
pub fn stage(stage: Stage) -> Guard {
    let span = match stage {
        Stage::Input => tracing::trace_span!(target: "ssmap_stage", "input"),
        Stage::Update => tracing::trace_span!(target: "ssmap_stage", "update"),
        Stage::Roughness => tracing::trace_span!(target: "ssmap_stage", "roughness"),
        Stage::Tessellation => tracing::trace_span!(target: "ssmap_stage", "tessellation"),
        Stage::Upload => tracing::trace_span!(target: "ssmap_stage", "upload"),
        Stage::Draw => tracing::trace_span!(target: "ssmap_stage", "draw"),
    };
    Guard {
        _span: span.entered(),
    }
}

#[cfg(not(feature = "profiling"))]
#[inline(always)]
pub fn stage(_stage: Stage) -> Guard {
    Guard {}
}

#[cfg(feature = "profiling")]
pub use recorder::Recorder;

#[cfg(feature = "profiling")]
mod recorder {
    use std::sync::{Arc, Mutex, MutexGuard};
    use std::thread::{self, ThreadId};
    use std::time::Instant;

    use tracing::span::{Attributes, Id, Record};
    use tracing::{Event, Metadata, Subscriber};

    use super::{Stage, StageClock, StageTimes};

    /// A `tracing` subscriber that only times the stage spans of the thread that installed it.
    pub struct Recorder {
        origin: Instant,
        owner: ThreadId,
        clock: Mutex<StageClock>,
    }

    impl Recorder {
        /// Makes a recorder the process-wide subscriber and returns it. `None` if some other
        /// subscriber is already set.
        pub fn install() -> Option<Arc<Recorder>> {
            let recorder = Arc::new(Recorder {
                origin: Instant::now(),
                owner: thread::current().id(),
                clock: Mutex::new(StageClock::default()),
            });
            tracing::subscriber::set_global_default(recorder.clone()).ok()?;
            Some(recorder)
        }

        /// The stage times since the last call.
        pub fn take(&self) -> StageTimes {
            self.clock().take()
        }

        fn clock(&self) -> MutexGuard<'_, StageClock> {
            self.clock.lock().unwrap_or_else(|e| e.into_inner())
        }
    }

    impl Subscriber for Recorder {
        fn enabled(&self, metadata: &Metadata<'_>) -> bool {
            metadata.target() == "ssmap_stage"
        }

        // The span id says which stage it is, so no per-span state is kept.
        fn new_span(&self, span: &Attributes<'_>) -> Id {
            let stage = Stage::ALL
                .into_iter()
                .position(|stage| stage.name() == span.metadata().name())
                .unwrap_or(0);
            Id::from_u64(stage as u64 + 1)
        }

        fn record(&self, _: &Id, _: &Record<'_>) {}

        fn record_follows_from(&self, _: &Id, _: &Id) {}

        fn event(&self, _: &Event<'_>) {}

        fn enter(&self, id: &Id) {
            if thread::current().id() == self.owner {
                let stage = Stage::ALL[id.into_u64() as usize - 1];
                self.clock().enter(stage, self.origin.elapsed());
            }
        }

        fn exit(&self, _: &Id) {
            if thread::current().id() == self.owner {
                self.clock().exit(self.origin.elapsed());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn nested_stages_exclude_what_is_inside_them() {
        let mut clock = StageClock::default();
        clock.enter(Stage::Update, ms(0));
        clock.enter(Stage::Roughness, ms(2));
        clock.exit(ms(7));
        clock.exit(ms(10));
        clock.enter(Stage::Draw, ms(10));
        clock.exit(ms(14));
        let times = clock.take();
        assert_eq!(times[Stage::Update], ms(5));
        assert_eq!(times[Stage::Roughness], ms(5));
        assert_eq!(times[Stage::Draw], ms(4));
        assert_eq!(times.total(), ms(14));
        assert_eq!(clock.take(), StageTimes::default());
    }

    #[test]
    fn a_stray_exit_is_ignored() {
        let mut clock = StageClock::default();
        clock.exit(ms(3));
        assert_eq!(clock.take().total(), Duration::ZERO);
    }

    #[cfg(feature = "profiling")]
    #[test]
    fn spans_reach_the_recorder() {
        let recorder = Recorder::install().expect("no other subscriber in this test binary");
        {
            let _outer = stage(Stage::Upload);
            let _inner = stage(Stage::Tessellation);
            std::thread::sleep(ms(2));
        }
        let times = recorder.take();
        assert!(times[Stage::Tessellation] >= ms(2));
        assert!(times[Stage::Upload] < ms(2));
        assert_eq!(times[Stage::Draw], Duration::ZERO);
    }
}
