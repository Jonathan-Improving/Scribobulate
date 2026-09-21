//! The off-main-thread decode bridge (TDD 27.2, 27.4).
//!
//! Moves one [`richimg::Animation`] onto GLib's existing I/O thread pool, decodes
//! exactly one frame, and hands the animation back — along with the decoded
//! [`richimg::Frame`] — on the main context. No GTK object ever crosses a thread:
//! only the plain owned `Animation`/`Frame` data does (POLICY § Architecture rules,
//! "All GTK access on the main thread"). No widget, no paintable and no tick
//! callback live here — those are the paintable's.
//!
//! # Following `docio`'s shape, not inventing a second one
//!
//! `src/docio/pool.rs` already does exactly this for document I/O: acquire an
//! in-process admission slot, THEN dispatch to `gio::spawn_blocking`, so the slot
//! (not the pool) is what queues an over-the-cap caller. This module is the same
//! shape, with its own cap and its own gate, because the two bounds protect
//! different things — see [`MAX_CONCURRENT_DECODES`] — and `docio`'s gate is
//! private to that module by design (POLICY § Typed GTK seams: the enforcement is
//! encapsulation, and a second caller reaching into it would defeat that).
//!
//! # At most one decode in flight per animation — enforced by the type
//!
//! [`decode_next_frame`] takes `Animation` **by value** and only gives it back once
//! the decode completes. `richimg::Animation` is not `Clone` (it owns a boxed
//! codec), so a caller holding the animation cannot start a second decode of it —
//! there is nothing left to pass. This is a compile-time property, not a flag a
//! caller has to remember to check; [`tests::a_second_decode_needs_the_first_animation_back`]
//! exercises the runtime half of it (the round trip really does return the same
//! animation, decoding in order, once the first future resolves).
//!
//! # Cancellation
//!
//! `gio::spawn_blocking` dispatches the moment it is called (`gio::task::JoinHandle`'s
//! own doc comment: "Dropping the handle 'detaches' the task, allowing it to
//! complete but discarding the return value"). So dropping a decode in flight —
//! anywhere in the future chain this module returns — never aborts the pool
//! thread mid-frame and never panics: the decode runs to completion on its own
//! thread and its result is simply discarded, because nothing is left to deliver it
//! to. See [`tests::dropping_a_pending_decode_neither_panics_nor_calls_back`].

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

/// How many animation decodes may occupy GLib's shared I/O thread pool at once,
/// process-wide.
///
/// The pool is shared with the crash-recovery snapshot writer (ScrAP-243) —
/// occupying too much of it does not fail a snapshot, it makes it *late*, which for a
/// mechanism protecting unsaved work is the same problem. Requests over the cap wait
/// **here**, in-process, where waiting costs nothing; they never reach the pool.
///
/// **The number is `docio::budget`'s, not this file's**, and that is the correction
/// rather than a tidy-up. This comment used to do the arithmetic itself — *"two more
/// brings the combined worst case to six, four short of the cliff"* — which was a
/// claim about every gate in the application, written in the one file that cannot
/// see them. It was seven and three by the time anybody re-read it: a third consumer
/// had appeared and nothing computed the sum, so there was nowhere for the
/// discrepancy to surface. The budget module asserts it at compile time instead.
const MAX_CONCURRENT_DECODES: usize =
    crate::docio::budget::cap(crate::docio::budget::Consumer::AnimationDecode);

thread_local! {
    /// Main-thread-only (every future that touches this is driven on the GTK main
    /// thread), so plain interior mutability with no locking — same reasoning as
    /// `docio::pool::GATE`.
    static GATE: RefCell<Gate> = const {
        RefCell::new(Gate { running: 0, waiting: VecDeque::new() })
    };
}

struct Gate {
    running: usize,
    waiting: VecDeque<Waiter>,
}

struct Waiter {
    granted: Rc<Cell<bool>>,
    waker: Waker,
}

/// One admitted decode slot. Releasing it — on drop, so an early return, a panic,
/// or the caller simply dropping the whole future cannot leak it — hands the slot
/// to the longest-waiting caller, or gives it back to the pool if nobody is
/// waiting. Structurally identical to `docio::pool::Slot`; see that module for the
/// reasoning behind each step.
struct Slot;

impl Drop for Slot {
    fn drop(&mut self) {
        let next = GATE.with(|gate| {
            let mut gate = gate.borrow_mut();
            match gate.waiting.pop_front() {
                Some(waiter) => {
                    waiter.granted.set(true);
                    Some(waiter.waker)
                }
                None => {
                    gate.running -= 1;
                    None
                }
            }
        });
        if let Some(waker) = next {
            waker.wake();
        }
    }
}

/// Wait for a free decode slot. See [`Slot`].
struct Acquire {
    granted: Rc<Cell<bool>>,
    queued: bool,
}

impl Acquire {
    fn new() -> Self {
        Acquire {
            granted: Rc::new(Cell::new(false)),
            queued: false,
        }
    }
}

impl Future for Acquire {
    type Output = Slot;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Slot> {
        let this = self.get_mut();
        if this.granted.get() {
            this.granted.set(false);
            this.queued = false;
            return Poll::Ready(Slot);
        }
        GATE.with(|gate| {
            let mut gate = gate.borrow_mut();
            if !this.queued && gate.running < MAX_CONCURRENT_DECODES {
                gate.running += 1;
                return Poll::Ready(Slot);
            }
            gate.waiting
                .retain(|w| !Rc::ptr_eq(&w.granted, &this.granted));
            gate.waiting.push_back(Waiter {
                granted: Rc::clone(&this.granted),
                waker: cx.waker().clone(),
            });
            this.queued = true;
            Poll::Pending
        })
    }
}

impl Drop for Acquire {
    fn drop(&mut self) {
        if !(self.queued || self.granted.get()) {
            return;
        }
        // Dropped while waiting (or just after being handed a slot but before the
        // next poll noticed) — deregister, and if a slot had already been handed
        // over, release it rather than losing it forever. This is exactly the
        // "dropping the handle cancels cleanly" contract one layer down: nothing
        // here ever panics, and a slot never leaks.
        let granted = self.granted.get();
        GATE.with(|gate| {
            let mut gate = gate.borrow_mut();
            gate.waiting
                .retain(|w| !Rc::ptr_eq(&w.granted, &self.granted));
            if granted {
                gate.running -= 1;
            }
        });
    }
}

/// Run `f` on GLib's I/O thread pool, waiting first for a free slot if this
/// application already has [`MAX_CONCURRENT_DECODES`] decodes out. Resumes on the
/// main context with `f`'s result.
///
/// `f` never runs on the calling thread — same guarantee as `docio::pool::off_main`,
/// for the same reason (`g_task_start_task_thread` always pushes to the pool).
/// A panic inside `f` is caught by the pool and re-raised **here**, on the main
/// thread, so the crash-report panic hook still runs (POLICY § Logging); this
/// should not normally fire since `richimg::Animation::next_frame` already catches
/// its own panics and reports [`richimg::Error::DecoderPanicked`] instead, but a
/// bug elsewhere in `f` must not be swallowed into a silently wrong result.
async fn on_pool<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let _slot = Acquire::new().await;
    #[cfg(test)]
    let f = {
        let delay = injected_delay();
        move || {
            // On the POOL thread, exactly where a slow decode's latency lands, so the
            // main loop keeps running throughout — a `sleep` here reproduces the
            // condition rather than merely postponing the test.
            std::thread::sleep(delay);
            f()
        }
    };
    #[allow(
        clippy::disallowed_methods,
        reason = "sanctioned dispatcher for docio::budget::Consumer::AnimationDecode; the admission gate is this module's own"
    )]
    match gtk::gio::spawn_blocking(f).await {
        Ok(value) => value,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

// Test-only: how long each dispatched decode should pretend to take.
//
// Ported from `docio::pool`, where the same seam already exists, because the
// behaviours that only exist while a decode is IN FLIGHT are otherwise unreachable:
// a local decode completes before anything can be observed, so the window in which a
// sprite can be hidden, re-shown, paused or re-themed does not open.
//
// The concrete gap this closes: the `decoder_generation` staleness guards in
// `AnimatedPaintable::on_decoded` and `SpriteAnim::on_decoded` — which were themselves
// the fix for an earlier defect — had NO test anywhere. Either could be deleted and the
// suite stayed green. Their own comments say they exist for the lose-then-REGAIN case,
// and the only late-decode coverage was lose-and-stay-lost, which the sibling
// `play_wanted` guard handles by itself.
//
// `#[cfg(test)]` throughout, deliberately: an env-var-gated version would put a
// fault-injection switch in the shipped binary. Here the delay does not exist in a
// release build at all.
#[cfg(test)]
thread_local! {
    static INJECTED_DELAY: Cell<std::time::Duration> = const {
        Cell::new(std::time::Duration::ZERO)
    };
}

#[cfg(test)]
fn injected_delay() -> std::time::Duration {
    INJECTED_DELAY.with(|d| d.get())
}

/// Make every decode dispatched here take at least `delay`, until the returned guard
/// is dropped. Restores the previous value rather than zeroing, so nesting is safe and
/// a panicking test cannot leave the delay set for whatever runs next on this thread.
// Gated to its callers' cfg (the gtk-integration-tests modules), not the broader
// `cfg(test)` — otherwise a bare `cargo test` compiles the injector with nothing to
// inject into and reports it, its guard and the re-export as dead.
#[cfg(all(test, feature = "gtk-integration-tests"))]
#[must_use = "the delay is only in force while the guard is alive"]
pub(crate) fn slow_decode(delay: std::time::Duration) -> SlowDecodeGuard {
    let previous = INJECTED_DELAY.with(|d| d.replace(delay));
    SlowDecodeGuard { previous }
}

#[cfg(all(test, feature = "gtk-integration-tests"))]
pub(crate) struct SlowDecodeGuard {
    previous: std::time::Duration,
}

#[cfg(all(test, feature = "gtk-integration-tests"))]
impl Drop for SlowDecodeGuard {
    fn drop(&mut self) {
        INJECTED_DELAY.with(|d| d.set(self.previous));
    }
}

/// Decode the next frame of `animation` off the main thread, subject to
/// [`MAX_CONCURRENT_DECODES`], returning the animation and the decode's result
/// together once it completes on the main context.
///
/// The animation is always handed back, decode error included: a poisoned or
/// exhausted `Animation` is still the caller's to hold onto (or drop) — this
/// function only ever runs one `next_frame()` call and reports what happened.
pub(crate) async fn decode_next_frame(
    mut animation: richimg::Animation,
) -> (richimg::Animation, Result<richimg::Frame, richimg::Error>) {
    on_pool(move || {
        let frame = animation.next_frame();
        (animation, frame)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How many slots are currently held — test-only view of the gate, mirroring
    /// `docio::pool::tests::running`.
    fn running() -> usize {
        GATE.with(|g| g.borrow().running)
    }

    fn waiting() -> usize {
        GATE.with(|g| g.borrow().waiting.len())
    }

    fn claim_now() -> Slot {
        let mut fut = Box::pin(Acquire::new());
        match poll_once(&mut fut) {
            Poll::Ready(slot) => slot,
            Poll::Pending => panic!("a claim under the cap must be admitted at once"),
        }
    }

    fn poll_once<F: Future>(fut: &mut Pin<Box<F>>) -> Poll<F::Output> {
        fut.as_mut().poll(&mut Context::from_waker(Waker::noop()))
    }

    /// The cap is the whole point of this module, asserted directly rather than
    /// inferred from timing: with [`MAX_CONCURRENT_DECODES`] slots out, one more
    /// does not reach the pool — it queues in-process instead.
    ///
    /// Mutation: raising `MAX_CONCURRENT_DECODES` past the pool's base size, or
    /// removing the `Acquire::new().await` from `on_pool`, fails this.
    #[test]
    fn no_more_than_the_cap_reach_the_pool_at_once() {
        let held: Vec<Slot> = (0..MAX_CONCURRENT_DECODES).map(|_| claim_now()).collect();
        assert_eq!(running(), MAX_CONCURRENT_DECODES);

        let mut extra = Box::pin(Acquire::new());
        assert!(poll_once(&mut extra).is_pending());
        assert_eq!(waiting(), 1, "the over-cap claim is queued, not admitted");
        assert_eq!(
            running(),
            MAX_CONCURRENT_DECODES,
            "and did not raise the count"
        );

        drop(extra);
        drop(held);
        assert_eq!(running(), 0);
        assert_eq!(waiting(), 0, "no waiter is left behind");
    }

    /// Releasing a slot hands it to the longest-waiting claim, with the count
    /// never dipping in between (the slot moves rather than being returned and
    /// re-taken) — same property `docio::pool` pins for its own gate.
    #[test]
    fn a_released_slot_goes_to_the_longest_waiting_claim() {
        let mut held: Vec<Slot> = (0..MAX_CONCURRENT_DECODES).map(|_| claim_now()).collect();
        let mut first = Box::pin(Acquire::new());
        let mut second = Box::pin(Acquire::new());
        assert!(poll_once(&mut first).is_pending());
        assert!(poll_once(&mut second).is_pending());

        drop(held.pop());
        assert_eq!(
            running(),
            MAX_CONCURRENT_DECODES,
            "the slot moved, not vanished"
        );
        // Bound rather than discarded: dropping the returned `Slot` inline would
        // immediately hand it on to `second`, and the next assertion would then be
        // testing the opposite of what it says.
        let taken = match poll_once(&mut first) {
            Poll::Ready(slot) => slot,
            Poll::Pending => panic!("the longest-waiting claim must take the freed slot"),
        };
        assert!(
            poll_once(&mut second).is_pending(),
            "the newer one still waits"
        );

        drop(taken);
        drop(second);
        drop(held);
        assert_eq!(running(), 0);
        assert_eq!(waiting(), 0);
    }
}

/// The GTK-dependent half: these need a live main context to drive `spawn_blocking`'s
/// completion back, so they are gated and run under `#[gtktest::test]` rather than as
/// plain `#[test]`s, exactly like `policy`'s `gtk_tests` module. Nothing here touches
/// a widget, a display, or any GTK object at all — only the main *context* is needed,
/// which `#[gtktest::test]` guarantees is live under both harnesses.
#[cfg(all(test, feature = "gtk-integration-tests"))]
mod gtk_tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn poll_once<F: Future>(fut: &mut Pin<Box<F>>) -> Poll<F::Output> {
        fut.as_mut().poll(&mut Context::from_waker(Waker::noop()))
    }

    /// A fixture large enough to have more than one frame, decoded by `richimg`
    /// directly (a pure-Rust decoder — no system codec dependency, unlike the
    /// GdkPixbuf-based path `memgate` exercises), so this needs no display and no
    /// host WebP support.
    fn test_animation() -> richimg::Animation {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/anim.webp");
        let bytes: Arc<[u8]> = Arc::from(std::fs::read(&path).expect("fixture reads"));
        richimg::Animation::new(bytes, &richimg::Limits::default())
            .expect("the fixture is a well-formed animated WebP")
    }

    fn drive<F: Future>(fut: F) -> F::Output {
        gtk::glib::MainContext::default().block_on(fut)
    }

    /// The decode body genuinely runs on a thread other than the one driving the
    /// main context, and the result is delivered back on that same main-context
    /// thread — the two halves of "off-main-thread decode, main-context return"
    /// (POLICY § Architecture rules).
    #[gtktest::test]
    fn decode_runs_off_the_main_thread_and_returns_on_it() {
        let main_thread = std::thread::current().id();
        let worker_thread = drive(on_pool(|| std::thread::current().id()));
        assert_ne!(
            worker_thread, main_thread,
            "the closure must be dispatched to GLib's pool, not run inline"
        );
        assert_eq!(
            std::thread::current().id(),
            main_thread,
            "control must resume on the main thread once the future is awaited"
        );
    }

    /// `decode_next_frame` hands the SAME animation back, and a second decode
    /// picks up exactly where the first left off — proving both the round trip
    /// and that nothing about the animation's own position was lost by crossing
    /// threads twice.
    ///
    /// This is the runtime half of "at most one decode in flight, enforced by the
    /// type": there is only one `Animation` value, `decode_next_frame` consumes it,
    /// and the test cannot call it again for the same logical animation until this
    /// call's future has resolved and handed it back — the exact shape a caller
    /// holding `Option<Animation>` would be in.
    #[gtktest::test]
    fn a_second_decode_needs_the_first_animation_back() {
        let animation = test_animation();
        let (animation, first) = drive(decode_next_frame(animation));
        let first = first.expect("first frame decodes");
        assert_eq!(first.index, 0);

        let (_animation, second) = drive(decode_next_frame(animation));
        let second = second.expect("second frame decodes");
        assert_eq!(
            second.index, 1,
            "the SAME animation resumed from where the first decode left it"
        );
    }

    /// Dropping a decode while it is still running on the pool thread neither
    /// panics nor calls back into anything: the task is detached (GLib's own
    /// `spawn_blocking` contract) and simply finishes with its result unread.
    ///
    /// "Neither panics" is proven by this test simply completing: a panic
    /// anywhere in the sequence below — in the pool thread's own send, in
    /// `Slot`'s or `Acquire`'s `Drop`, or anywhere else — would fail this test
    /// body itself, since `#[gtktest::test]` runs it like any other test.
    #[gtktest::test]
    fn dropping_a_pending_decode_neither_panics_nor_calls_back() {
        let finished = Arc::new(AtomicBool::new(false));

        let finished_writer = Arc::clone(&finished);
        let mut fut = Box::pin(on_pool(move || {
            std::thread::sleep(Duration::from_millis(150));
            finished_writer.store(true, Ordering::SeqCst);
        }));

        // One poll is enough: `on_pool`'s `Acquire` resolves synchronously when a
        // slot is free, so the same call reaches `gio::spawn_blocking`, which
        // dispatches the moment it is called — the task is genuinely running on
        // the pool thread by the time this returns `Pending`.
        assert!(
            poll_once(&mut fut).is_pending(),
            "the decode is now in flight"
        );

        drop(fut);

        // Wait for the detached pool thread to finish and attempt (and fail
        // silently, since nothing is left to receive it) to deliver its result.
        //
        // A generous FAILURE bound polled to convergence, never a fixed sleep
        // (GTK4Rs/AP-122): the claim is that the task runs to COMPLETION, not
        // that it completes within any particular span, and how long a loaded
        // host takes to give a pool thread its turn is not this test's subject.
        // A fixed 400 ms wait against a task that sleeps 150 ms failed on the
        // GitHub Linux runner while passing on every development host.
        let deadline = Instant::now() + Duration::from_secs(20);
        while !finished.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }

        assert!(
            finished.load(Ordering::SeqCst),
            "the already-running decode must run to completion, just with its \
             result discarded"
        );
    }
}
