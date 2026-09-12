//! The playback decision core (TDD 27.2, 27.4): given the clock and what has
//! already been presented, what should an animation do next?
//!
//! No GTK, no display, no clock of its own — every method takes the current time as
//! a parameter, monotonic **microseconds** as `i64` (exactly what
//! `GdkFrameClock::frame_time` returns), so the whole thing is unit-tested with no
//! main loop. WP7b drives it from the real frame clock; these tests drive it with
//! supplied times.
//!
//! # Fall behind by skipping, never by queueing
//!
//! A frame's **decode** can never be skipped: WebP, GIF and APNG frames composite
//! onto the previous canvas (`richimg::Animation::next_frame` is only ever "the very
//! next frame"), so reaching frame N always means decoding every frame before it.
//! What this schedule protects is **presentation**, not decode: when a decode takes
//! longer than the frame's own slot, the next frame is not shown late and the
//! schedule does not try to claw the lost time back by bursting several frames onto
//! the screen in a row. Instead the due-time baseline simply resets to "now", so
//! only a paint and a texture upload are ever skipped — never a decode.
//!
//! # One request outstanding at a time
//!
//! [`Schedule::poll`] only ever answers [`Action::NeedNextFrame`] once per frame —
//! it remembers that it asked, and answers [`Action::Hold`] on every following poll
//! until [`Schedule::present`] reports the answer. This is what stops a caller
//! polled every tick from asking [`worker`](super::worker) for a second decode of
//! the same animation while the first is still in flight, however many due-times
//! elapse in between.

use std::time::Duration;

/// Monotonic time as `GdkFrameClock::frame_time` reports it: microseconds, never
/// wall-clock-of-day, and never read from a real clock by this module — every
/// caller supplies it.
pub(crate) type Micros = i64;

/// What playback should do right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    /// The current frame's slot has not elapsed; nothing to do.
    Hold,
    /// The current frame's slot elapsed — decode and present the next frame. Never
    /// answered again for the same frame until [`Schedule::present`] resolves it,
    /// so a caller cannot ask for a second decode while one is already in flight.
    NeedNextFrame,
    /// Every requested play has finished; the animation stays on its last frame and
    /// nothing further is ever requested.
    Stopped,
}

/// The playback decision core for one animation.
///
/// Holds only what the decision needs: the due time of the frame currently on
/// screen, the last presented frame's index (to detect the decoder's own wrap back
/// to 0), how many loops have completed, and whether every requested play has
/// finished.
pub(crate) struct Schedule {
    loop_count: richimg::LoopCount,
    completed_loops: u32,
    last_index: u32,
    due_at: Micros,
    awaiting: bool,
    stopped: bool,
}

impl Schedule {
    /// Begin playback at `now`, with frame 0 already on screen for `first_delay`
    /// and `loop_count` total plays (or [`richimg::LoopCount::Infinite`]).
    pub(crate) fn start(
        now: Micros,
        first_delay: Duration,
        loop_count: richimg::LoopCount,
    ) -> Self {
        Schedule {
            loop_count,
            completed_loops: 0,
            last_index: 0,
            due_at: now.saturating_add(micros(first_delay)),
            awaiting: false,
            stopped: false,
        }
    }

    /// What to do at time `now`. Pure and idempotent between calls to
    /// [`Self::present`]: polling twice at the same (or a later) time before a
    /// decode resolves answers [`Action::NeedNextFrame`] exactly once.
    pub(crate) fn poll(&mut self, now: Micros) -> Action {
        if self.stopped {
            return Action::Stopped;
        }
        if self.awaiting {
            return Action::Hold;
        }
        if now >= self.due_at {
            self.awaiting = true;
            Action::NeedNextFrame
        } else {
            Action::Hold
        }
    }

    /// Record that the frame requested by the last [`Action::NeedNextFrame`]
    /// decoded and arrived at `now`, carrying `frame_index` and `delay` (a
    /// [`richimg::Frame`]'s own fields — its *effective* delay, floor already
    /// applied, per TDD 27.2).
    ///
    /// Returns `true` when the frame should actually be shown. Returns `false`
    /// only when this presentation would be the wrap that finishes the last
    /// requested play of a [`richimg::LoopCount::Finite`] animation — the decode
    /// still happened (it always does), but the wrapped frame is discarded and the
    /// **previous** frame stays on screen, with no further frame ever requested
    /// (TDD 27.1's "stays on its last frame and uses no more CPU").
    pub(crate) fn present(&mut self, now: Micros, frame_index: u32, delay: Duration) -> bool {
        self.awaiting = false;
        // `<=`, not `<`: a strict decrease misses the one-frame animation, whose
        // "next" frame is always index 0 again — the wrap and the wrapped index are
        // the same value there, so equality must count as a wrap too. A genuine
        // forward step always has `frame_index > last_index`, so this never
        // mis-detects ordinary mid-sequence progress as a loop closing.
        let wrapped = frame_index <= self.last_index;
        if wrapped {
            self.completed_loops += 1;
            if let richimg::LoopCount::Finite(total_plays) = self.loop_count {
                if self.completed_loops >= total_plays {
                    self.stopped = true;
                    return false;
                }
            }
        }
        self.last_index = frame_index;

        // The no-drift, no-burst rule: advance the baseline by exactly one delay
        // from where it already was, UNLESS that lands in the past relative to the
        // moment this frame is actually shown (the decode overran its slot) — in
        // which case the baseline resets to now, so the next frame is due one delay
        // from here rather than from a moment that has already gone by. That reset
        // is the whole of "skip, don't queue": nothing is played in a burst to make
        // up the difference.
        let naive_next_due = self.due_at.saturating_add(micros(delay));
        self.due_at = if naive_next_due < now {
            now.saturating_add(micros(delay))
        } else {
            naive_next_due
        };
        true
    }

    /// Give up on the frame the last [`Action::NeedNextFrame`] requested: it will
    /// never be [`Self::present`]ed, because the decode failed or its result was
    /// discarded on arrival.
    ///
    /// **Clears only the in-flight latch.** `last_index` and `completed_loops` describe
    /// what has actually been SHOWN, and an abandoned decode showed nothing — so
    /// resetting them here would silently rewind the animation's position and its
    /// loop accounting, turning a dropped frame into a replayed loop. That is the
    /// difference between this and [`Self::restart`], which deliberately rewinds both
    /// because it is answering "start again from the top".
    ///
    /// Without this, `awaiting` was a **one-way latch**: [`Self::poll`] sets it on the
    /// way out and only `present`/`restart` ever cleared it, so a decode that ended any
    /// other way left the schedule permanently answering [`Action::Hold`] —
    /// [`Action::NeedNextFrame`] is unreachable while `awaiting`. Removing the tick
    /// registration hides it (nothing is polling), but the moment anything re-arms the
    /// clock without going through `restart` the animation is pinned *and* dead. Two
    /// routes reach exactly that: the sprite driver, which never calls `restart` at all,
    /// and the reader toggling Play Animations off and back on (QA round 2, F-R2-1).
    pub(crate) fn abandon(&mut self) {
        self.awaiting = false;
    }

    /// Resume playback from frame 0 at `now`, as if freshly [`Self::start`]ed —
    /// for a stopped or paused animation coming back (WP3/visibility decide
    /// *when*; this only answers *what happens once it does*).
    pub(crate) fn restart(&mut self, now: Micros, first_delay: Duration) {
        self.completed_loops = 0;
        self.last_index = 0;
        self.due_at = now.saturating_add(micros(first_delay));
        self.awaiting = false;
        self.stopped = false;
    }
}

/// A [`Duration`] as whole microseconds, clamped to `i64::MAX` rather than
/// overflowing — a duration this large never occurs on a real animation, and a
/// saturating clamp keeps the arithmetic above infallible without an `unwrap`.
fn micros(d: Duration) -> Micros {
    i64::try_from(d.as_micros()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use richimg::LoopCount;

    const DELAY: Duration = Duration::from_millis(100);
    const DELAY_US: Micros = 100_000;

    /// Steady playback: each `present` lands the baseline exactly one delay past
    /// where it already was, so due-times fall on a fixed grid (0, 100_000,
    /// 200_000, …) however close the actual presentation lands to its slot —
    /// nothing creeps.
    #[test]
    fn steady_playback_advances_one_frame_per_delay_with_no_drift() {
        let mut s = Schedule::start(0, DELAY, LoopCount::Infinite);

        assert_eq!(
            s.poll(50_000),
            Action::Hold,
            "before the first slot elapses"
        );
        assert_eq!(s.poll(DELAY_US), Action::NeedNextFrame);
        // Presented a little late (decode took 5ms into the next slot) — still
        // within the slot, so the grid does not move.
        assert!(s.present(DELAY_US + 5_000, 1, DELAY));

        assert_eq!(s.poll(150_000), Action::Hold);
        assert_eq!(s.poll(200_000), Action::NeedNextFrame);
        assert!(s.present(205_000, 2, DELAY));

        // The grid landed on exactly 300_000 — 3 * DELAY_US from the origin — not
        // "205_000 + 100_000", which would have drifted forward by the 5ms slack
        // taken on every earlier frame.
        assert_eq!(s.poll(299_999), Action::Hold, "one microsecond before due");
        assert_eq!(s.poll(300_000), Action::NeedNextFrame);
    }

    /// A frame with the floored delay (TDD 27.2: under 20ms becomes 50ms) is
    /// scheduled for exactly the EFFECTIVE delay `richimg::Frame` reports — this
    /// module never re-derives or re-floors it, it only ever honours what it is
    /// given.
    #[test]
    fn honours_whatever_effective_delay_it_is_given() {
        let floored = Duration::from_millis(50);
        let mut s = Schedule::start(0, floored, LoopCount::Infinite);
        assert_eq!(s.poll(49_999), Action::Hold);
        assert_eq!(s.poll(50_000), Action::NeedNextFrame);
    }

    /// A decode that overran its slot (arrives well after the baseline's naive
    /// next due-time) resets the baseline to "now" instead of leaving it in the
    /// past — the defining behaviour of "skip, don't queue".
    ///
    /// Mutation: making `present` always use `due_at + delay` (dropping the reset
    /// branch) reddens this — see its second assertion.
    #[test]
    fn a_late_presentation_resets_the_baseline_instead_of_accumulating_lateness() {
        let mut s = Schedule::start(0, DELAY, LoopCount::Infinite);
        assert_eq!(s.poll(DELAY_US), Action::NeedNextFrame);

        // The decode took 400ms for a 100ms slot — arrives at t=500_000, long
        // after the naive next due-time (100_000 + 100_000 = 200_000).
        assert!(s.present(500_000, 1, DELAY));

        // Without the reset the due-time would still be 200_000, which is already
        // in the past at t=550_000 — so `poll` would fire immediately instead of
        // holding, and the mutation above turns this into a burst.
        assert_eq!(
            s.poll(550_000),
            Action::Hold,
            "the baseline must have reset to 500_000 + delay = 600_000, not stayed \
             at the stale 200_000"
        );
        assert_eq!(s.poll(600_000), Action::NeedNextFrame);
    }

    /// However many due-times elapse while one decode is outstanding, `poll`
    /// never asks for a second one — there is no backlog to work through once the
    /// slow frame finally arrives, only the single next frame.
    #[test]
    fn a_slow_decode_in_flight_never_produces_a_backlog_of_requests() {
        let mut s = Schedule::start(0, DELAY, LoopCount::Infinite);
        assert_eq!(s.poll(DELAY_US), Action::NeedNextFrame);

        // Many due-times pass with the decode still outstanding — every one of
        // them must hold, not re-request.
        for t in [150_000, 1_000_000, 5_000_000, 50_000_000] {
            assert_eq!(
                s.poll(t),
                Action::Hold,
                "a decode is already in flight at t={t}"
            );
        }

        assert!(s.present(50_005_000, 1, DELAY));
        // Exactly one further request, once the new baseline's slot elapses.
        assert_eq!(s.poll(50_050_000), Action::Hold);
        assert_eq!(s.poll(50_105_000), Action::NeedNextFrame);
    }

    /// `LoopCount::Finite(1)`: the animation stops on the wrap that closes its
    /// only play, discarding that wrapped frame rather than showing it, and asks
    /// for nothing more afterward.
    ///
    /// Mutation: removing the `stopped = true` branch reddens this — `poll` would
    /// keep answering `NeedNextFrame` past the last play.
    #[test]
    fn stops_on_the_wrap_that_finishes_a_single_play() {
        let mut s = Schedule::start(0, DELAY, LoopCount::Finite(1));
        assert_eq!(s.poll(DELAY_US), Action::NeedNextFrame);
        assert!(
            s.present(DELAY_US, 1, DELAY),
            "an ordinary mid-sequence frame"
        );
        assert_eq!(s.poll(200_000), Action::NeedNextFrame);
        assert!(
            s.present(200_000, 2, DELAY),
            "the last frame before the wrap"
        );

        assert_eq!(s.poll(300_000), Action::NeedNextFrame);
        assert!(
            !s.present(300_000, 0, DELAY),
            "the wrap back to frame 0 finishes the one requested play — discard it"
        );
        assert_eq!(
            s.poll(1_000_000_000),
            Action::Stopped,
            "nothing is ever requested again, at any later time"
        );
    }

    /// `LoopCount::Finite(3)`: the same shape, generalised — only the THIRD wrap
    /// stops it, not the first or second.
    #[test]
    fn stops_only_after_the_final_of_several_finite_plays() {
        let mut s = Schedule::start(0, DELAY, LoopCount::Finite(3));
        let mut t: Micros = 0;
        let mut index = 0u32;
        // Two full plays of three frames (0,1,2 then wrap to 0) that must NOT stop.
        for play in 0..2 {
            for _ in 0..3 {
                t += DELAY_US;
                assert_eq!(s.poll(t), Action::NeedNextFrame, "play {play}");
                index = (index + 1) % 3;
                assert!(
                    s.present(t, index, DELAY),
                    "play {play} must not stop early"
                );
            }
        }
        // The third and final play.
        for i in 0..2 {
            t += DELAY_US;
            assert_eq!(s.poll(t), Action::NeedNextFrame);
            index = (index + 1) % 3;
            assert!(s.present(t, index, DELAY), "final play, frame {i}");
        }
        // The wrap that closes the third play.
        t += DELAY_US;
        assert_eq!(s.poll(t), Action::NeedNextFrame);
        assert!(!s.present(t, 0, DELAY), "the third wrap must stop it");
        assert_eq!(s.poll(t + 1_000_000), Action::Stopped);
    }

    /// `LoopCount::Infinite` never stops, however many times the index wraps.
    #[test]
    fn infinite_loop_count_never_stops() {
        let mut s = Schedule::start(0, DELAY, LoopCount::Infinite);
        let mut t: Micros = 0;
        let mut index = 0u32;
        for _ in 0..50 {
            t += DELAY_US;
            assert_eq!(s.poll(t), Action::NeedNextFrame);
            index = (index + 1) % 4;
            assert!(
                s.present(t, index, DELAY),
                "infinite playback never discards a frame"
            );
        }
        assert_eq!(s.poll(t + DELAY_US), Action::NeedNextFrame);
    }

    /// F-R2-1: a frame request that is never presented must be retirable, or `awaiting`
    /// is a one-way latch and the schedule answers `Hold` forever.
    ///
    /// Mutation: make [`Schedule::abandon`] a no-op (an empty body) and the second
    /// `NeedNextFrame` below becomes `Hold`. That is precisely the shape the bug had —
    /// nothing observable until something re-arms the clock, and then an animation that
    /// is pinned *and* silent.
    #[test]
    fn an_abandoned_frame_request_lets_the_next_poll_ask_again() {
        let mut s = Schedule::start(0, DELAY, LoopCount::Infinite);
        assert_eq!(s.poll(DELAY_US), Action::NeedNextFrame);
        // The latch is real: a second poll does not re-ask while one is outstanding.
        assert_eq!(s.poll(DELAY_US), Action::Hold);

        // That decode failed, or its result was discarded on arrival.
        s.abandon();

        assert_eq!(
            s.poll(DELAY_US),
            Action::NeedNextFrame,
            "after abandoning the request the schedule must be able to ask again"
        );
    }

    /// [`Schedule::abandon`] retires the REQUEST and nothing else: an abandoned decode
    /// showed no frame, so rewinding position or loop accounting would replay a loop.
    /// This is the whole difference between `abandon` and `restart`.
    ///
    /// Mutation: give `abandon` `restart`'s body (also clearing `last_index` and
    /// `completed_loops`) and the animation never stops — the first completed loop is
    /// forgotten, so the second wrap counts as loop 1 of 2 and playback runs forever.
    ///
    /// It asserts through LOOP ACCOUNTING rather than through `last_index`, because a
    /// `last_index` assertion does not discriminate: the interesting wrap is to index 0,
    /// and `0 <= 0` reads as a wrap just as `0 <= 1` does, so both the correct and the
    /// rewinding version pass it. (Written that way first, and caught by running the
    /// mutation instead of trusting the reasoning.)
    #[test]
    fn abandoning_a_request_does_not_rewind_what_has_been_shown() {
        // Two plays of a two-frame animation: the SECOND wrap back to 0 ends it.
        let mut s = Schedule::start(0, DELAY, LoopCount::Finite(2));
        assert_eq!(s.poll(DELAY_US), Action::NeedNextFrame);
        assert!(s.present(DELAY_US, 1, DELAY));
        assert_eq!(s.poll(2 * DELAY_US), Action::NeedNextFrame);
        assert!(s.present(2 * DELAY_US, 0, DELAY), "first loop completes");

        // A request is made and then abandoned — it showed nothing, so it must not
        // disturb the fact that one full play has already happened.
        assert_eq!(s.poll(3 * DELAY_US), Action::NeedNextFrame);
        s.abandon();

        assert_eq!(s.poll(3 * DELAY_US), Action::NeedNextFrame);
        assert!(s.present(3 * DELAY_US, 1, DELAY));
        assert_eq!(s.poll(4 * DELAY_US), Action::NeedNextFrame);
        assert!(
            !s.present(4 * DELAY_US, 0, DELAY),
            "the wrap completing the SECOND play must be withheld — if `abandon` had \
             cleared `completed_loops`, this would read as only the first"
        );
        assert_eq!(s.poll(5 * DELAY_US), Action::Stopped);
    }

    /// A stopped animation resumes exactly like a fresh one once restarted —
    /// WP3/visibility decide when that happens; this is only what happens next.
    #[test]
    fn restart_after_a_stop_resumes_normal_playback() {
        let mut s = Schedule::start(0, DELAY, LoopCount::Finite(1));
        assert_eq!(s.poll(DELAY_US), Action::NeedNextFrame);
        assert!(
            !s.present(DELAY_US, 0, DELAY),
            "wraps immediately: a one-frame loop"
        );
        assert_eq!(s.poll(10_000_000), Action::Stopped);

        s.restart(10_000_000, DELAY);
        assert_eq!(s.poll(10_050_000), Action::Hold);
        assert_eq!(s.poll(10_100_000), Action::NeedNextFrame);
        assert!(
            s.present(10_100_000, 1, DELAY),
            "playback resumed, not still stopped"
        );
    }

    /// Loops are counted ONLY by the index wrapping (decreasing), never merely by
    /// `present` being called — a `Finite(1)` animation with several in-sequence
    /// frames must not stop until the wrap actually happens.
    #[test]
    fn a_loop_completes_only_on_the_index_wrap_not_on_every_present() {
        let mut s = Schedule::start(0, DELAY, LoopCount::Finite(1));
        for (t, index) in [(DELAY_US, 1), (2 * DELAY_US, 2), (3 * DELAY_US, 3)] {
            assert_eq!(s.poll(t), Action::NeedNextFrame);
            assert!(
                s.present(t, index, DELAY),
                "index {index} increases — no wrap yet, must not stop"
            );
        }
        let t = 4 * DELAY_US;
        assert_eq!(s.poll(t), Action::NeedNextFrame);
        assert!(
            !s.present(t, 0, DELAY),
            "index dropped back to 0 — the wrap"
        );
    }
}
