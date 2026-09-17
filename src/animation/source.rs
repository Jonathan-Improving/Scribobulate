//! Shared encoded bytes for animated pictures (TDD §27, "Animation state: per
//! picture, bounded by what is on screen").
//!
//! Two pictures showing the same animated file share ONE `Arc<[u8]>` of its
//! encoded bytes — nothing else. Decoder state (`richimg::Animation`) is never
//! shared: each picture plays at its own position, so [`super::paintable`]
//! gives each one its own decoder and its own working canvas. This module is
//! only the byte-sharing half: a process-wide registry of `Weak<[u8]>`, keyed
//! the same way `imagecache` keys a local file (path+mtime+size) or a remote
//! one (its URL) — `imagecache::loader` computes that key and calls [`shared`]
//! (a fresh decode already has bytes in hand) or [`shared_or_else`] (a cache HIT
//! recovering a local file's bytes: cheap when a holder is alive, a bounded
//! re-read otherwise) with it, so this module stays pure and knows nothing about
//! paths or URLs.
//!
//! Nothing here KEEPS the bytes alive — that is what "shared by reference"
//! means. The registry holds only [`Weak`]; the first caller to register a key
//! becomes the sole strong holder (ordinarily an [`super::paintable::AnimatedPaintable`],
//! via its `richimg::Animation`), and the entry answers a second caller only
//! for as long as that holder (or another) is still alive. Once every picture
//! showing a file is gone, the next [`shared`] call for that key allocates a
//! fresh copy — there is no cache to evict and no budget to track, unlike
//! `imagecache`'s decoded-texture budget, because there is nothing left
//! resident to bound.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Weak};

thread_local! {
    /// Main-thread-only, like `imagecache`'s own thread-local — GTK is
    /// single-threaded (POLICY § Architecture rules) and every caller of
    /// [`shared`] runs on the main thread; the `Weak<[u8]>` values it holds
    /// point at bytes that DO cross to a worker thread (`animation::worker`),
    /// but the registry itself is never touched from there.
    static REGISTRY: RefCell<HashMap<String, Weak<[u8]>>> = RefCell::new(HashMap::new());
}

/// Return the bytes already shared under `key`, if some other picture is
/// still holding them; otherwise register `fresh` as the shared copy for
/// `key` and return it unchanged — the caller becomes the (first) strong
/// holder, and nothing here keeps it alive past that.
///
/// Opportunistically drops every OTHER dead entry on the way in — cheap for
/// the tiny table this registry ever holds (one entry per distinct animated
/// file ever shown, not per byte), and it is what keeps the table from
/// growing forever across a long session as pictures come and go.
pub(crate) fn shared(key: &str, fresh: Arc<[u8]>) -> Arc<[u8]> {
    REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        registry.retain(|_, weak| weak.strong_count() > 0);
        if let Some(existing) = registry.get(key).and_then(Weak::upgrade) {
            return existing;
        }
        registry.insert(key.to_string(), Arc::downgrade(&fresh));
        fresh
    })
}

/// Like [`shared`], but the fresh bytes are computed LAZILY, only when no live
/// picture is already holding this key's bytes.
///
/// `imagecache`'s local-animation cache-hit recovery (TDD 27.1) needs exactly
/// this ordering: a cache hit's cheap path — another picture is still alive — must
/// never pay for the bounded re-read `imagedecode::read_local` performs, and the
/// expensive path must run at most once even when nobody is holding the bytes.
/// Checking the registry first and only then calling `fresh` is what makes that
/// true; passing an already-read `Arc` to [`shared`] cannot express "don't bother
/// reading unless nobody has this".
///
/// `None` from `fresh` (a re-read that failed — the file vanished, changed shape,
/// grew past the cap) propagates as `None` here; nothing is registered.
pub(crate) fn shared_or_else(
    key: &str,
    fresh: impl FnOnce() -> Option<Arc<[u8]>>,
) -> Option<Arc<[u8]>> {
    let existing = REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        registry.retain(|_, weak| weak.strong_count() > 0);
        registry.get(key).and_then(Weak::upgrade)
    });
    if let Some(existing) = existing {
        return Some(existing);
    }
    Some(shared(key, fresh()?))
}

/// Empty the registry. Test-only, for the same reason `imagecache::reset_for_test`
/// exists: this is a `thread_local!` and libtest runs the whole suite in one
/// process, so a key one test leaves resident (accidentally kept alive past its
/// own scope) could answer another test's lookup.
#[cfg(test)]
pub(crate) fn reset_for_test() {
    REGISTRY.with(|registry| registry.borrow_mut().clear());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two callers for the SAME key, while the first Arc is still held, converge
    /// on ONE allocation — the sharing this module exists to provide.
    ///
    /// Mutation: making `shared` always insert-and-return `fresh` (never checking
    /// the registry for an existing live entry) reddens this — the mutation this
    /// test exists to catch.
    #[test]
    fn two_pictures_of_the_same_file_share_one_arc() {
        reset_for_test();
        let first = Arc::<[u8]>::from(&b"encoded bytes"[..]);
        let shared_first = shared("local:a.webp:100:16", first);

        let second = Arc::<[u8]>::from(&b"encoded bytes"[..]);
        let shared_second = shared("local:a.webp:100:16", second);

        assert!(
            Arc::ptr_eq(&shared_first, &shared_second),
            "the second caller must get back the FIRST caller's allocation, not its own"
        );
        reset_for_test();
    }

    /// A different key never shares with another — two distinct files never
    /// converge on one allocation just because they happened to be requested
    /// close together.
    #[test]
    fn different_keys_never_share() {
        reset_for_test();
        let a = shared("local:a.webp:100:16", Arc::from(&b"a"[..]));
        let b = shared("local:b.webp:100:16", Arc::from(&b"b"[..]));
        assert!(!Arc::ptr_eq(&a, &b));
        reset_for_test();
    }

    /// Once every strong holder of a key's bytes is dropped, the entry does not
    /// keep them alive — the next [`shared`] call for that key allocates fresh,
    /// proving the registry holds only a [`Weak`], never a budget-tracked cache.
    ///
    /// Mutation: holding the ORIGINAL `Arc` inside the registry (a `RefCell<HashMap<String,
    /// Arc<[u8]>>>`, say) instead of a `Weak` reddens this — the third call
    /// would then return the first (stale) allocation, byte-for-byte identical
    /// but the SAME pointer, which the pointer-identity assertion below catches
    /// where a value-equality one would not.
    #[test]
    fn dropping_every_holder_lets_the_entry_die() {
        reset_for_test();
        let key = "local:a.webp:100:16";
        let first = shared(key, Arc::from(&b"encoded bytes"[..]));
        let ptr_before = Arc::as_ptr(&first);
        drop(first);

        let second = shared(key, Arc::from(&b"encoded bytes"[..]));
        assert_ne!(
            Arc::as_ptr(&second),
            ptr_before,
            "with no live holder left, a fresh allocation must be registered — \
             the same address recurring would mean something besides the caller \
             kept the old bytes alive"
        );
        reset_for_test();
    }

    /// The registry cleans up dead entries as a side effect of ordinary use —
    /// it does not grow by one entry per distinct key forever even though
    /// nothing ever explicitly removes a *live* one.
    #[test]
    fn a_dead_entry_is_swept_on_the_next_unrelated_call() {
        reset_for_test();
        let first = shared("local:a.webp:100:16", Arc::from(&b"a"[..]));
        drop(first);
        assert_eq!(
            REGISTRY.with(|r| r.borrow().len()),
            1,
            "sanity: still recorded, just dead"
        );

        // An unrelated call sweeps the dead entry for "a.webp" on its way in.
        let _second = shared("local:b.webp:100:16", Arc::from(&b"b"[..]));
        assert_eq!(
            REGISTRY.with(|r| r.borrow().len()),
            1,
            "the dead \"a.webp\" entry must have been swept, leaving only \"b.webp\""
        );
        reset_for_test();
    }

    /// [`shared_or_else`]'s cheap path: a live holder answers with NO call to `fresh`
    /// at all — the whole point, since `imagecache`'s cache-hit recovery uses
    /// `fresh` for a bounded disk re-read that must never run when it isn't needed.
    #[test]
    fn shared_or_else_never_calls_fresh_while_a_holder_is_alive() {
        reset_for_test();
        let key = "local:a.webp:100:16";
        let held = shared(key, Arc::from(&b"encoded bytes"[..]));

        let mut fresh_calls = 0;
        let answer = shared_or_else(key, || {
            fresh_calls += 1;
            Some(Arc::from(&b"a different re-read"[..]))
        });

        assert!(
            Arc::ptr_eq(&answer.expect("a live holder must answer"), &held),
            "must return the SAME allocation the live holder has, not a fresh one"
        );
        assert_eq!(fresh_calls, 0, "fresh must not run while a holder is alive");
        reset_for_test();
    }

    /// The other half: with no live holder, `fresh` runs exactly once and its bytes
    /// become the new shared entry — proving the lazy path actually reaches
    /// [`shared`] rather than silently doing nothing.
    #[test]
    fn shared_or_else_calls_fresh_exactly_once_with_no_live_holder() {
        reset_for_test();
        let key = "local:a.webp:100:16";
        let mut fresh_calls = 0;

        let answer = shared_or_else(key, || {
            fresh_calls += 1;
            Some(Arc::from(&b"re-read from disk"[..]))
        })
        .expect("fresh supplied bytes");
        assert_eq!(fresh_calls, 1);

        // And it is now the registered entry: a second caller shares THIS Arc.
        let second = shared_or_else(key, || {
            fresh_calls += 1;
            Some(Arc::from(&b"should never be read"[..]))
        })
        .expect("the just-registered entry answers");
        assert!(Arc::ptr_eq(&answer, &second));
        assert_eq!(
            fresh_calls, 1,
            "the second call must reuse the just-registered entry"
        );
        reset_for_test();
    }

    /// A `fresh` that fails (the file vanished, grew past the cap, changed shape)
    /// propagates `None` — nothing is registered, so a later call tries again rather
    /// than being stuck on a permanent miss.
    #[test]
    fn shared_or_else_propagates_a_failed_reread_and_registers_nothing() {
        reset_for_test();
        let key = "local:a.webp:100:16";

        assert!(shared_or_else(key, || None).is_none());
        assert_eq!(
            REGISTRY.with(|r| r.borrow().len()),
            0,
            "a failed re-read must not leave a dangling registry entry"
        );
        reset_for_test();
    }
}
