//! The bake's frame budget (PLAN-v2 section 3.2, M7): what a `frame` call
//! does next while the host's `work-left` meter runs down, and the cost
//! window the mod logs.
//!
//! The host turns the granted microseconds into a soft fuel budget and reports
//! what is left of it as `work-left`, from [`WORK_SCALE`] down to 0. The bake
//! takes steps (read a batch of chunks, paint some rows) while more than
//! [`RESERVE`] is left, then uploads texels while more than
//! [`UPLOAD_RESERVE`] is left, and keeps the rest for drawing the corner box.
//! A step that starts just above a reserve may run past it, so the frame's
//! cost is at most the budget plus the dearest step past the reserve it
//! started above; the tests pin that.

/// `work-left` at the start of a frame call.
pub const WORK_SCALE: u64 = 1_000_000;
/// Work kept back for the upload and the corner box.
pub const RESERVE: u64 = 350_000;
/// Work kept back for the corner box once uploading.
pub const UPLOAD_RESERVE: u64 = 60_000;
/// Chunk batches read in a frame before painting may take a turn.
pub const READS_FIRST: u32 = 2;

/// What the bake may do next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Rebuild a step of the block looks (after a resource reload).
    Rebuild,
    /// Read a pending chunk.
    Read,
    /// Put a chunk read into the mosaic (a step of its own: on Pulley a
    /// chunk's read and its placing are each near a third of the budget).
    Set,
    /// Paint dirty rows, sized to the work left above [`RESERVE`].
    Paint,
    /// Paint the box's dirty rows, sized to the work left above
    /// [`UPLOAD_RESERVE`]: the box comes first (PLAN-v2 risk 1).
    FocusPaint,
    /// Upload the box's texels, likewise.
    FocusUpload,
    /// Nothing left that fits: go on to the upload.
    Stop,
}

/// What is waiting, as the frame loop sees it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Waiting {
    pub rebuild: bool,
    /// A chunk is read and waits to be put in.
    pub ready: bool,
    pub pending: bool,
    pub painting: bool,
    /// A chunk under the box is pending; rows in the box wait to be
    /// painted; texels in the box wait to be uploaded.
    pub focus_pending: bool,
    pub focus_painting: bool,
    pub focus_uploading: bool,
}

/// What the steps cost, in work, as the [`Rate`]s have learnt it: a read,
/// a set, a rebuild step, the least painting (a row) and the least upload
/// (a strip's row of texels).
#[derive(Clone, Copy, Debug, Default)]
pub struct Costs {
    pub read: u64,
    pub set: u64,
    pub rebuild: u64,
    pub paint_row: u64,
    pub upload_row: u64,
}

/// The next step with `left` work left after `reads` reads this frame;
/// `first` when nothing ran yet this frame.
///
/// A step starts only if its cost fits above the reserve it keeps: the box's
/// work keeps [`UPLOAD_RESERVE`], the rest of the mosaic [`RESERVE`]. The
/// frame's first step always starts, so a step dearer than the whole budget
/// (a chunk read on Pulley) still happens, once a frame. The box's work
/// comes first: its chunks, then its rows, then its texels. Then the rest:
/// reading nearest first, but after [`READS_FIRST`] reads painting takes
/// over while rows are dirty, so a long read queue never starves the map.
pub fn next(left: u64, reads: u32, first: bool, w: Waiting, c: &Costs) -> Step {
    let fits = |cost: u64, keep: u64| first || left > keep.saturating_add(cost);
    if left <= UPLOAD_RESERVE && !first {
        Step::Stop
    } else if w.ready && fits(c.set, UPLOAD_RESERVE) {
        Step::Set
    } else if w.rebuild && fits(c.rebuild, RESERVE) {
        Step::Rebuild
    } else if w.focus_pending && !w.ready && fits(c.read, UPLOAD_RESERVE) {
        Step::Read
    } else if w.focus_painting && fits(c.paint_row, UPLOAD_RESERVE) {
        Step::FocusPaint
    } else if w.focus_uploading && fits(c.upload_row, UPLOAD_RESERVE) {
        Step::FocusUpload
    } else if w.pending && !w.ready && (reads < READS_FIRST || !w.painting) && fits(c.read, RESERVE)
    {
        Step::Read
    } else if w.painting && fits(c.paint_row, RESERVE) {
        Step::Paint
    } else {
        Step::Stop
    }
}

/// Whether another upload piece may start with `left` work left.
pub fn may_upload(left: u64) -> bool {
    left > UPLOAD_RESERVE
}

/// How much work one unit of a step (a row painted, a byte uploaded) costs,
/// learnt from the steps taken, so a step is sized to the work left: on
/// Pulley (iOS) a unit costs some 30 times what it does compiled, and a
/// fixed size that suits one overruns or crawls on the other.
#[derive(Clone, Copy, Debug)]
pub struct Rate {
    /// Work per unit, a running average.
    per_unit: f64,
    /// Steps seen.
    seen: u32,
}
impl Rate {
    /// Starts from a guess of `per_unit`.
    pub const fn new(per_unit: f64) -> Rate {
        Rate { per_unit, seen: 0 }
    }
    /// Steps it has learnt from.
    pub fn seen(&self) -> u32 {
        self.seen
    }
    /// Units that fit in `left` work, keeping `reserve`, from 1 to `most`.
    /// One at least: a frame always moves on.
    pub fn units(&self, left: u64, reserve: u64, most: u32) -> u32 {
        let room = left.saturating_sub(reserve) as f64;
        ((room / self.per_unit.max(1e-3)) as u64).clamp(1, u64::from(most.max(1))) as u32
    }
    /// A step of `units` units cost `work`.
    pub fn observe(&mut self, work: u64, units: u32) {
        if units == 0 || work == 0 {
            return;
        }
        self.seen += 1;
        let seen = work as f64 / f64::from(units);
        // Up at once (an overrun is the worse error), down slowly.
        self.per_unit = if seen > self.per_unit {
            seen
        } else {
            self.per_unit + (seen - self.per_unit) * 0.25
        };
    }
    pub fn per_unit(&self) -> f64 {
        self.per_unit
    }
    /// A step of `units` units ran the meter to 0: it cost `work` or more,
    /// by how much the meter cannot say. The guess doubles (at least to
    /// what it saw), to at most four budgets, so such a step only starts
    /// again where it fits, or first in a frame, and cheaper steps bring it
    /// down.
    pub fn overran(&mut self, work: u64, units: u32) {
        self.seen += 1;
        let seen = work as f64 / f64::from(units.max(1));
        self.per_unit = (2.0 * self.per_unit).max(seen).min(4.0 * WORK_SCALE as f64);
    }
    /// [`Rate::observe`], or [`Rate::overran`] when the meter is at 0 after.
    pub fn after(&mut self, work: u64, units: u32, left_now: u64) {
        if left_now == 0 {
            self.overran(work, units);
        } else {
            self.observe(work, units);
        }
    }
    /// The work of `n` units.
    pub fn of(&self, n: u32) -> u64 {
        (self.per_unit * f64::from(n)) as u64
    }
}

/// Putting a chunk in the mosaic costs more than this much work (80 µs of
/// the 300 µs budget) where the mod runs on an interpreter (Pulley, iOS):
/// M7 measured 3 µs (bench) to 25 µs (the game, busy streaming) compiled,
/// 150-190 µs on Pulley.
pub const SLOW_SET: f64 = 270_000.0;
/// Sets learnt from before deciding.
pub const SETS_TO_DECIDE: u32 = 16;

/// Whether the engine is slow enough that the mosaic should drop to one
/// texel a block (PLAN-v2 risk 1, the first mitigation), from what a chunk
/// set costs; none until enough sets are seen.
pub fn slow_engine(set: &Rate) -> Option<bool> {
    (set.seen() >= SETS_TO_DECIDE).then(|| set.per_unit() > SLOW_SET)
}

/// Work units to microseconds of a `granted` µs budget.
pub fn micros(work: u64, granted: u32) -> u64 {
    work * u64::from(granted) / WORK_SCALE
}

/// The bake's cost over a window of frames, in work units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cost {
    pub frames: u32,
    pub busy: u32,
    pub sum: u64,
    pub busy_sum: u64,
    pub max: u64,
    /// The dearest single step (a read batch or a paint step).
    pub max_step: u64,
    /// The dearest read batch, paint step and recentre.
    pub max_read: u64,
    pub max_paint: u64,
    pub max_recentre: u64,
    /// Frames that used all their work: they ran past the soft budget by
    /// an amount the meter cannot show (`work-left` stops at 0).
    pub spent: u32,
    pub chunks: u64,
    pub uploaded: u64,
    pub read: u64,
    pub paint: u64,
    pub upload: u64,
    /// Drawing the box (its shapes, sent when the view moved), and the
    /// dearest draw.
    pub draw: u64,
    pub max_draw: u64,
}
impl Cost {
    /// Adds a frame that used `used` work.
    pub fn frame(&mut self, used: u64, busy: bool, chunks: usize, uploaded: usize) {
        self.frames += 1;
        self.spent += u32::from(used >= WORK_SCALE);
        self.sum += used;
        self.max = self.max.max(used);
        self.chunks += chunks as u64;
        self.uploaded += uploaded as u64;
        if busy {
            self.busy += 1;
            self.busy_sum += used;
        }
    }
    /// Adds a step's cost to its part.
    pub fn step(&mut self, step: Step, used: u64) {
        self.max_step = self.max_step.max(used);
        match step {
            Step::Read | Step::Set => {
                self.read += used;
                self.max_read = self.max_read.max(used);
            }
            Step::Paint | Step::FocusPaint => {
                self.paint += used;
                self.max_paint = self.max_paint.max(used);
            }
            Step::FocusUpload => self.upload += used,
            Step::Rebuild | Step::Stop => {}
        }
    }
    /// A draw of the box's shapes.
    pub fn draw(&mut self, used: u64) {
        self.draw += used;
        self.max_draw = self.max_draw.max(used);
    }
    /// A recentre's cost (it runs before the steps, unbudgeted).
    pub fn recentre(&mut self, used: u64) {
        self.max_recentre = self.max_recentre.max(used);
    }
    /// The window's line, in µs of a `granted` budget.
    pub fn line(&self, granted: u32) -> String {
        let us = |w: u64| micros(w, granted);
        format!(
            "{} frames, avg {} us/frame, busy {} frames avg {} us, max {} us, {} spent, \
             dearest step {} us (read {} us, paint {} us, recentre {} us) \
             (budget {granted} us); split read {} us, paint {} us, upload {} us, \
             draw {} us (dearest {} us); {} chunks read, {} KiB uploaded",
            self.frames,
            us(self.sum.checked_div(u64::from(self.frames)).unwrap_or(0)),
            self.busy,
            us(self.busy_sum.checked_div(u64::from(self.busy)).unwrap_or(0)),
            us(self.max),
            self.spent,
            us(self.max_step),
            us(self.max_read),
            us(self.max_paint),
            us(self.max_recentre),
            us(self.read),
            us(self.paint),
            us(self.upload),
            us(self.draw),
            us(self.max_draw),
            self.chunks,
            self.uploaded / 1024,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Costs = Costs {
        read: 0,
        set: 0,
        rebuild: 0,
        paint_row: 0,
        upload_row: 0,
    };

    /// A frame against a fake meter: `reads` chunk batches and `paints` paint
    /// steps waiting, each costing what the closures say, then uploads of
    /// `upload` work a piece while `pieces` remain. Returns the work used and
    /// the steps taken.
    fn run_frame(
        mut reads: u32,
        mut paints: u32,
        read_cost: impl Fn(u32) -> u64,
        paint_cost: impl Fn(u32) -> u64,
        mut pieces: u32,
        upload: u64,
    ) -> (u64, Vec<Step>) {
        let mut used = 0u64;
        let left = |used: u64| WORK_SCALE.saturating_sub(used);
        let (mut done_reads, mut steps) = (0, Vec::new());
        loop {
            let w = Waiting {
                pending: reads > 0,
                painting: paints > 0,
                ..Waiting::default()
            };
            match next(
                left(used),
                done_reads,
                steps.is_empty(),
                w,
                &Costs::default(),
            ) {
                Step::Read => {
                    used += read_cost(done_reads);
                    done_reads += 1;
                    reads -= 1;
                    // A read batch marks rows to paint.
                    paints += 1;
                    steps.push(Step::Read);
                }
                Step::Paint => {
                    used += paint_cost(paints);
                    paints -= 1;
                    steps.push(Step::Paint);
                }
                Step::Rebuild | Step::Set | Step::FocusPaint | Step::FocusUpload => unreachable!(),
                Step::Stop => break,
            }
        }
        while pieces > 0 && may_upload(left(used)) {
            used += upload;
            pieces -= 1;
        }
        (used, steps)
    }

    #[test]
    fn nothing_starts_once_the_reserve_is_reached() {
        let all = Waiting {
            rebuild: true,
            ready: true,
            pending: true,
            painting: true,
            focus_pending: true,
            focus_painting: true,
            focus_uploading: true,
        };
        assert_eq!(next(UPLOAD_RESERVE, 0, false, all, &NONE), Step::Stop);
        assert_eq!(next(RESERVE + 1, 0, false, all, &NONE), Step::Set);
        // The rest's steps stop at the reserve, the box's go on.
        let unset = Waiting {
            ready: false,
            rebuild: false,
            focus_pending: false,
            ..all
        };
        assert_eq!(next(RESERVE, 0, false, unset, &NONE), Step::FocusPaint);
        let rest = Waiting {
            focus_painting: false,
            focus_uploading: false,
            ..unset
        };
        assert_eq!(next(RESERVE, 0, false, rest, &NONE), Step::Stop);
        assert_eq!(next(0, 0, false, Waiting::default(), &NONE), Step::Stop);
        assert!(!may_upload(UPLOAD_RESERVE));
        assert!(may_upload(UPLOAD_RESERVE + 1));
    }

    #[test]
    fn the_box_comes_first() {
        let mut w = Waiting {
            rebuild: true,
            pending: true,
            painting: true,
            focus_pending: true,
            focus_painting: true,
            focus_uploading: true,
            ..Waiting::default()
        };
        assert_eq!(next(WORK_SCALE, 0, false, w, &NONE), Step::Rebuild);
        w.rebuild = false;
        assert_eq!(
            next(WORK_SCALE, 5, false, w, &NONE),
            Step::Read,
            "the box's chunks, past READS_FIRST"
        );
        w.focus_pending = false;
        assert_eq!(next(WORK_SCALE, 0, false, w, &NONE), Step::FocusPaint);
        w.focus_painting = false;
        assert_eq!(next(WORK_SCALE, 0, false, w, &NONE), Step::FocusUpload);
        w.focus_uploading = false;
        assert_eq!(next(WORK_SCALE, 0, false, w, &NONE), Step::Read);
    }

    #[test]
    fn a_step_starts_only_if_it_fits_or_comes_first() {
        let w = Waiting {
            ready: true,
            pending: true,
            painting: true,
            focus_pending: true,
            ..Waiting::default()
        };
        let dear = Costs {
            read: 2 * WORK_SCALE,
            set: 500_000,
            paint_row: 100_000,
            ..NONE
        };
        // The first step goes whatever it costs.
        assert_eq!(next(WORK_SCALE, 0, true, w, &dear), Step::Set);
        // Then only what fits above its reserve.
        assert_eq!(next(500_000, 0, false, w, &dear), Step::Paint);
        assert_eq!(next(RESERVE + 100_000, 0, false, w, &dear), Step::Stop);
        // No read while a chunk read waits to be put in.
        let waiting = Waiting {
            painting: false,
            ..w
        };
        assert_eq!(next(WORK_SCALE, 0, false, waiting, &NONE), Step::Set);
        assert_eq!(next(400_000, 0, false, waiting, &dear), Step::Stop);
        let free = Waiting {
            ready: false,
            ..waiting
        };
        assert_eq!(next(WORK_SCALE, 0, true, free, &dear), Step::Read);
        assert_eq!(next(WORK_SCALE, 0, false, free, &dear), Step::Stop);
    }

    #[test]
    fn reads_go_first_then_painting_takes_turns() {
        let w = Waiting {
            pending: true,
            painting: true,
            ..Waiting::default()
        };
        assert_eq!(next(WORK_SCALE, 0, false, w, &NONE), Step::Read);
        assert_eq!(
            next(WORK_SCALE, READS_FIRST - 1, false, w, &NONE),
            Step::Read
        );
        assert_eq!(next(WORK_SCALE, READS_FIRST, false, w, &NONE), Step::Paint);
        // With nothing dirty, reading goes on past the first batches.
        let idle = Waiting {
            painting: false,
            ..w
        };
        assert_eq!(next(WORK_SCALE, 10, false, idle, &NONE), Step::Read);
        let (_, steps) = run_frame(50, 0, |_| 60_000, |_| 60_000, 0, 0);
        assert_eq!(&steps[..3], [Step::Read, Step::Read, Step::Paint]);
    }

    /// The budget's promise: steps that each cost no more than the work
    /// between the two reserves keep the frame inside the granted budget
    /// (300 µs), and one dear step can only push it past by its own cost,
    /// which at the reserves' sizes stays under 0.5 ms of a 300 µs budget.
    #[test]
    fn a_frame_stays_inside_its_budget() {
        let fits = RESERVE - UPLOAD_RESERVE;
        for &(read, paint, upload) in &[
            (fits, fits, UPLOAD_RESERVE),
            (40_000, 25_000, 8_000),
            (120_000, 90_000, 30_000),
            (1, 1, 1),
        ] {
            let (used, _) = run_frame(1000, 0, |_| read, |_| paint, 1000, upload);
            assert!(used <= WORK_SCALE, "{read} {paint} {upload}: {used}");
        }
        // One step dearer than the whole reserve: over by at most what it
        // costs past the reserve it started above, and no upload after it.
        let dear = 900_000;
        let read = |i| if i == 3 { dear } else { 50_000 };
        let (used, _) = run_frame(1000, 0, read, |_| 50_000, 1000, 8_000);
        assert!(
            used > WORK_SCALE && used <= WORK_SCALE + dear - RESERVE,
            "{used}"
        );
        // So any single step cheaper than the whole budget ends a 300 µs
        // frame under 0.5 ms.
        assert!(micros(2 * WORK_SCALE - RESERVE, 300) < 500);
    }

    #[test]
    fn a_rate_sizes_steps_to_the_work_left() {
        let mut r = Rate::new(10_000.0);
        assert_eq!(r.units(RESERVE + 100_000, RESERVE, 64), 10);
        assert_eq!(r.units(RESERVE, RESERVE, 64), 1, "one at least");
        assert_eq!(r.units(WORK_SCALE, 0, 64), 64, "at most");
        // Dearer than thought: up at once.
        r.observe(400_000, 10);
        assert_eq!(r.per_unit(), 40_000.0);
        assert_eq!(r.units(RESERVE + 100_000, RESERVE, 64), 2);
        // Cheaper: down a quarter of the way a step.
        r.observe(100_000, 10);
        assert_eq!(r.per_unit(), 32_500.0);
        r.observe(0, 0);
        assert_eq!(r.per_unit(), 32_500.0);
        // Ran the meter out: twice the guess, or what it saw if more.
        r.after(100_000, 10, 0);
        assert_eq!(r.per_unit(), 65_000.0);
        r.after(1_000_000, 1, 0);
        assert_eq!(r.per_unit(), 1_000_000.0);
        // Never past four budgets, however often it overruns.
        for _ in 0..100 {
            r.after(3_000_000, 1, 0);
        }
        assert_eq!(r.per_unit(), 4.0 * WORK_SCALE as f64);
        r.after(500_000, 1, 10);
        assert_eq!(r.per_unit(), 3_125_000.0);
        assert_eq!(r.seen(), 105);
        // Sized steps keep a frame of them inside the budget whatever a
        // unit costs, as long as one unit fits in the reserve's gap.
        for cost in [500.0, 5_000.0, 50_000.0, 250_000.0] {
            let mut r = Rate::new(cost);
            let mut left = WORK_SCALE;
            while left > RESERVE {
                let n = r.units(left, RESERVE, 64);
                let used = (f64::from(n) * cost) as u64;
                r.observe(used, n);
                left = left.saturating_sub(used);
            }
            assert!(left > 0 || cost > (RESERVE as f64), "{cost}: {left}");
        }
    }

    #[test]
    fn a_slow_engine_is_told_by_what_a_set_costs() {
        let mut compiled = Rate::new(700_000.0);
        let mut pulley = Rate::new(700_000.0);
        assert_eq!(slow_engine(&compiled), None);
        for _ in 0..SETS_TO_DECIDE {
            compiled.observe(80_000, 1);
            pulley.observe(550_000, 1);
        }
        assert_eq!(slow_engine(&compiled), Some(false));
        assert_eq!(slow_engine(&pulley), Some(true));
    }

    #[test]
    fn the_cost_window_averages_and_keeps_the_worst() {
        let mut c = Cost::default();
        c.frame(1_000_000, true, 8, 4096);
        c.recentre(30_000);
        c.frame(0, false, 0, 0);
        c.frame(500_000, true, 4, 1024);
        c.step(Step::Read, 300_000);
        c.step(Step::Paint, 200_000);
        c.step(Step::Read, 100_000);
        assert_eq!(
            (c.frames, c.busy, c.max, c.max_step),
            (3, 2, 1_000_000, 300_000)
        );
        assert_eq!(
            (c.read, c.paint, c.chunks, c.uploaded),
            (400_000, 200_000, 12, 5120)
        );
        assert_eq!(
            c.line(300),
            "3 frames, avg 150 us/frame, busy 2 frames avg 225 us, max 300 us, 1 spent, \
             dearest step 90 us (read 90 us, paint 60 us, recentre 9 us) \
             (budget 300 us); split read 120 us, paint 60 us, upload 0 us, draw 0 us (dearest 0 us); \
             12 chunks read, 5 KiB uploaded"
        );
        assert_eq!(
            Cost::default().line(300).split(',').nth(1),
            Some(" avg 0 us/frame")
        );
    }
}
