//! WP2 / TDD 2.23a-b, 6.9: truncated input and the image-webp#182 crafted
//! panic — both must degrade to an `Error`, never bring the process down,
//! and a panic must poison the `Animation` it came from.
#[path = "support/mod.rs"]
mod support;

use std::panic;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use richimg::{contained_panic_in_progress, first_frame, probe, Animation, Error, Limits};
use support::read_fixture;

#[test]
fn truncated_file_is_malformed_from_every_entry_point() {
    let limits = Limits::default();
    let bytes = read_fixture("truncated.webp");

    // Still recognisable as WebP by content — only the frame data is gone.
    assert_eq!(richimg::sniff(&bytes), Some(richimg::Format::WebP));

    assert_eq!(probe(&bytes, &limits), Err(Error::Malformed));
    assert_eq!(
        Animation::new(Arc::clone(&bytes), &limits).err(),
        Some(Error::Malformed)
    );
    assert_eq!(first_frame(&bytes, &limits).err(), Some(Error::Malformed));
}

/// The exact reproducer from image-rs/image-webp#182 ("Panic with zero
/// sized vp8 data inside ANMF following ALPH"): an ANMF frame whose VP8
/// bitstream decodes to a degenerate 0x0 image while the ANMF header
/// declares a real canvas region, which the ALPH-present branch of
/// `WebPDecoder::read_frame` (0.2.4) does not size-check before calling
/// `Frame::fill_rgba` — an out-of-bounds slice index inside `image-webp`'s
/// own YUV upsampling. `probe`/`Animation::new` succeed (the header itself
/// is well-formed); it is `next_frame` that reaches the panicking code.
#[test]
fn crafted_182_panics_are_caught_and_poison_the_animation() {
    let limits = Limits::default();
    let bytes = read_fixture("crafted_182.webp");

    // The header is well-formed enough that probe/open succeed...
    probe(&bytes, &limits).expect("probe the #182 reproducer");
    let mut anim = Animation::new(Arc::clone(&bytes), &limits).expect("open the #182 reproducer");

    // ...and decoding the frame is where image-webp 0.2.4 panics. richimg's
    // central catch_unwind must turn that into an Err, not crash this test
    // process (the mere fact that the assertions below run at all is part
    // of the proof: a process-ending panic would never reach them).
    let first_attempt = anim.next_frame();
    assert_eq!(
        first_attempt,
        Err(Error::DecoderPanicked),
        "a caught image-webp panic must surface as DecoderPanicked"
    );

    // Poisoned: every later call keeps returning the same error without
    // touching the (possibly now-inconsistent) decoder again.
    for attempt in 0..3 {
        assert_eq!(
            anim.next_frame(),
            Err(Error::DecoderPanicked),
            "poisoned Animation, call {attempt}"
        );
    }

    // rewind() on a poisoned Animation must not panic either (it takes the
    // early-return path in Animation::rewind).
    anim.rewind();
    assert_eq!(anim.next_frame(), Err(Error::DecoderPanicked));
}

// `set_hook`/`take_hook` are process-global. Only one test in this binary
// touches them, but libtest still runs test functions concurrently on
// separate threads by default, so this mutex serialises against any other
// test in this SAME binary that might ever install a hook (none currently
// does; the guard is here so that stays true rather than becoming a source
// of flakiness later) and the hook is always restored before returning,
// panic or not is not a concern here since no assertion above the restore
// can panic.
static HOOK_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

fn hook_mutex() -> &'static Mutex<()> {
    HOOK_MUTEX.get_or_init(|| Mutex::new(()))
}

/// `contained_panic_in_progress()` exists so the application's crash-report
/// panic hook (which fires on every panic, caught or not) can tell a
/// contained decoder panic apart from a real crash. Verified here the way
/// the application actually uses it: install a panic hook, trigger the real
/// #182 panic through the public API, and have the HOOK observe the flag —
/// not by reaching into richimg's internals.
#[test]
fn contained_panic_in_progress_is_observed_by_a_panic_hook_during_the_182_panic() {
    let _serialize = hook_mutex()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());

    assert!(
        !contained_panic_in_progress(),
        "must be false before any guarded call"
    );

    let observed_during_panic = Arc::new(AtomicBool::new(false));
    let hook_ran_at_all = Arc::new(AtomicBool::new(false));
    let observed_clone = Arc::clone(&observed_during_panic);
    let ran_clone = Arc::clone(&hook_ran_at_all);

    let previous_hook = panic::take_hook();
    panic::set_hook(Box::new(move |_info| {
        ran_clone.store(true, Ordering::SeqCst);
        observed_clone.store(contained_panic_in_progress(), Ordering::SeqCst);
    }));

    let limits = Limits::default();
    let bytes = read_fixture("crafted_182.webp");
    let mut anim = Animation::new(bytes, &limits).expect("open the #182 reproducer");
    let result = anim.next_frame();

    panic::set_hook(previous_hook);

    assert_eq!(result, Err(Error::DecoderPanicked));
    assert!(
        hook_ran_at_all.load(Ordering::SeqCst),
        "the panic hook never ran — the fixture no longer panics under this image-webp version"
    );
    assert!(
        observed_during_panic.load(Ordering::SeqCst),
        "contained_panic_in_progress() was false inside the panic hook"
    );
    assert!(
        !contained_panic_in_progress(),
        "must be false again once the guarded call has returned"
    );
}
