//! Process-wide cache for decoded image textures.
//!
//! ## Why this exists
//!
//! A render is no longer a rare event — a disclosure fold-toggle, a theme
//! switch, or a live reload re-walks every image tag. Remote images would be
//! re-fetched synchronously on the main thread (GTK4Rs/AP-44). Local images would
//! be re-decoded, and on an animated WebP that decode leaks (~12 MB per call
//! through the gdk-pixbuf incremental path). This module is the fix for both:
//! a hit returns the already-decoded texture with no network call and no
//! second decode. Keys are URLs for remote images and `local:{path}:{mtime}:{size}`
//! for local ones, sharing one LRU byte budget. The eviction/TTL policy itself
//! is pure and lives in [`policy`]; this module is the thin GTK wiring — the
//! process-wide singleton, decoded-byte accounting, and the entry point
//! [`get_or_fetch`] that [`loader`] calls.
//!
//! ## Shape
//!
//! One [`policy::Cache`] instance, keyed on the image URL, holds either a
//! [`CachedTexture`] (a hit — returned with no network access) or a
//! short-lived negative marker (a cached failure — see [`NEGATIVE_CACHE_TTL`]).
//! It is **process-wide**, not per-tab or per-window: two tabs, or two windows,
//! showing the same remote image fetch it once between them. GTK is
//! single-threaded (POLICY § Architecture rules "All GTK access on the main
//! thread"), so a `thread_local!` holds it without a lock, the same pattern
//! `theme.rs`'s active-theme cell uses for the same reason.
//!
//! Successful entries live for the session — there is no positive TTL and
//! nothing ever proactively expires one; the only way a decoded texture leaves
//! the cache is LRU eviction under [`IMAGE_CACHE_BUDGET_BYTES`].
//!
//! ## Animation (TDD 27.1)
//!
//! Each entry carries an [`AnimationHint`] alongside its texture — "a playing
//! animation is not a cache entry": the encoded bytes an
//! `AnimatedPaintable` needs live elsewhere, EXCEPT for a remote entry, where
//! retaining them is cheaper than the only alternative a hit has (a second network
//! fetch on the opt-in "Show Unsafe Images" path). [`AnimationHint::Still`] and
//! [`AnimationHint::Local`] carry no payload — one enum discriminant plus the size
//! of an unused `Arc<[u8]>` slot (two `usize`s), a small fixed cost paid by every
//! entry, animated or not. [`AnimationHint::Remote`]'s bytes are real and are
//! folded into [`cached_bytes`]'s accounting so the LRU prunes against the cache's
//! true footprint, bounded per entry by `limits::MAX_REMOTE_IMAGE_BYTES` (16
//! MiB) — up to half of [`IMAGE_CACHE_BUDGET_BYTES`] for one image, on the same
//! "still cached even past budget" terms [`policy::Cache::record_success`] already
//! applies to any oversized entry.
//!
//! `renderer::start`'s image loader (`imagecache::loader`) recovers a LOCAL
//! animated image's bytes on a hit itself (sharing with a live picture, or a
//! bounded re-read) — this module only carries the hint that tells it whether to
//! bother.

mod keys;
mod loader;
mod policy;

pub(crate) use loader::{load_texture, LoadedImage};

use gtk::prelude::TextureExt;
use policy::Cache;
use std::cell::RefCell;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Total decoded-pixel bytes the cache may hold before evicting the
/// least-recently-used entry. This bounds **system RAM under the Cairo software
/// renderer this project forces** (POLICY § Architecture rules) — a decoded
/// `GdkTexture` never touches the GPU, so it does not count against the VRAM
/// ceiling (TDD §6), but it is exactly the same "measure it, don't guess
/// it" discipline that ceiling is held to, so the number is chosen the same way:
/// `limits::MAX_REMOTE_IMAGE_BYTES`'s own doc comment measured a real 1280px
/// photographic JPEG at 583 KiB compressed; decoded to ARGB32 pixels that is
/// roughly 1280 × 853 × 4 ≈ 4.4 MiB (see [`decoded_byte_size`]). This budget
/// (32 MiB) therefore holds on the order of seven such images at once —
/// comfortably a whole document's worth of remote images — while staying well
/// under half of the ~80 MiB RAM TECH.md measures for a single document with
/// full-fidelity rendering, so a document with unusually many or large remote
/// images cannot double the application's own baseline footprint. Re-measure
/// rather than re-reason if this ever needs raising, per the same rule
/// `limits.rs` states for its own constant.
const IMAGE_CACHE_BUDGET_BYTES: usize = 32 * 1024 * 1024;

/// How long a failed fetch is remembered before the next disclosure toggle (or
/// any other re-render) is allowed to retry it. This is the number that stands
/// between the two failure modes negative caching exists to balance: with **no**
/// negative TTL, a dead URL is re-fetched synchronously on every re-render —
/// the exact freeze this cache exists to remove, since a fold toggle is exactly
/// such a re-render. With a **long** (or session-lasting) TTL, a transient
/// failure — a flaky host, a momentary DNS blip — reads as permanently broken
/// for the rest of the session, with no way for the reader to make it retry
/// short of restarting the app. ~60 seconds comfortably absorbs a burst of fold
/// toggles against the same document (each one re-renders synchronously, so a
/// human operating the UI cannot toggle faster than roughly once a second in
/// practice) while remaining short enough that a reader who waits a minute and
/// tries again sees the image reappear on its own. Operator-ratified value;
/// re-measure against real toggle cadence rather than re-guess if this ever
/// needs changing.
const NEGATIVE_CACHE_TTL: Duration = Duration::from_secs(60);

thread_local! {
    static CACHE: RefCell<Cache<CachedTexture>> =
        RefCell::new(Cache::new(IMAGE_CACHE_BUDGET_BYTES, NEGATIVE_CACHE_TTL));
}

/// What a cache entry says about whether its file animates. See the module doc's
/// "Animation" section for why [`Self::Remote`] is the only variant with a payload.
#[derive(Clone)]
pub(crate) enum AnimationHint {
    Still,
    Local,
    Remote(Arc<[u8]>),
}

/// A cached texture plus its [`AnimationHint`] — the value type this module's
/// [`policy::Cache`] instance holds.
#[derive(Clone)]
pub(crate) struct CachedTexture {
    pub(crate) texture: gtk::gdk::Texture,
    pub(crate) animation: AnimationHint,
}

/// The decoded byte cost of a texture of `width` × `height` pixels: one ARGB32
/// word (4 bytes) per pixel, the format a `GdkTexture` decodes into under the
/// Cairo software renderer this project forces. Not exact for every source
/// pixel format `GdkTexture::from_bytes` might choose internally, but it is the
/// same order of magnitude for all of them, and it is the decoded size the byte
/// budget prunes against — never the compressed transfer size `imagefetch`'s own
/// cap already bounds separately. A pure `i32 -> usize` function (rather than
/// taking `&gtk::gdk::Texture` directly) so the arithmetic is unit-tested with
/// no GTK type at all.
fn decoded_byte_size(width: i32, height: i32) -> usize {
    (width.max(0) as u64 * height.max(0) as u64 * 4) as usize
}

/// [`decoded_byte_size`] plus, for an [`AnimationHint::Remote`] entry, the
/// retained encoded bytes — the accounting the byte budget must include so the
/// LRU prunes against the cache's real footprint. Before this, a retained
/// remote animation's bytes counted against nothing at all.
fn cached_bytes(texture: &gtk::gdk::Texture, animation: &AnimationHint) -> usize {
    let retained = match animation {
        AnimationHint::Remote(bytes) => bytes.len(),
        AnimationHint::Still | AnimationHint::Local => 0,
    };
    decoded_byte_size(texture.width(), texture.height()) + retained
}

/// Look up `key` in the process-wide cache; call `fetch` only on an outright
/// miss (never on a hit, never during a live negative-cache window), and record
/// its outcome. `fetch` should perform the actual network GET/local read +
/// decode (`imagecache::loader`'s `load_remote_texture`/`load_local` supply it),
/// returning the decoded texture and its [`AnimationHint`] — this function owns
/// only whether that ever needs to happen.
pub(crate) fn get_or_fetch(
    key: &str,
    fetch: impl FnOnce() -> Option<(gtk::gdk::Texture, AnimationHint)>,
) -> Option<CachedTexture> {
    get_or_fetch_at(key, Instant::now(), fetch)
}

/// [`get_or_fetch`] with the clock supplied rather than read.
///
/// The production wrapper above reads `Instant::now()` itself, which makes every
/// time-dependent behaviour of the real cache — the negative TTL expiring, a re-attempt
/// after it, the sweep — unreachable from a test without sleeping through a real minute.
/// `policy::Cache` has taken `now` as a parameter from the start for exactly this reason;
/// this seam extends the same discipline to the thread-local the application actually
/// uses, so a test exercises the SHIPPED path rather than a second cache it built itself.
pub(crate) fn get_or_fetch_at(
    key: &str,
    now: Instant,
    fetch: impl FnOnce() -> Option<(gtk::gdk::Texture, AnimationHint)>,
) -> Option<CachedTexture> {
    // The borrow discipline lives in `policy::get_or_fetch` — it takes the `RefCell`
    // precisely so no borrow is held across `fetch`. See its doc comment.
    CACHE.with(|cell| {
        policy::get_or_fetch(cell, key, now, || {
            fetch().map(|(texture, animation)| {
                let bytes = cached_bytes(&texture, &animation);
                (CachedTexture { texture, animation }, bytes)
            })
        })
    })
}

/// Empty the thread-local cache.
///
/// Test-only, and required rather than convenient: libtest runs the whole suite in one
/// process and this cache is a `thread_local!`, so an entry one test leaves behind
/// silently answers another test's lookup — the process-global-state hazard POLICY's
/// unit-testing section names, arriving here through a cache rather than a signal
/// handler. A test that touches the shipped cache resets it first.
#[cfg(test)]
pub(crate) fn reset_for_test() {
    CACHE.with(|cell| {
        *cell.borrow_mut() = Cache::new(IMAGE_CACHE_BUDGET_BYTES, NEGATIVE_CACHE_TTL);
    });
}

#[cfg(test)]
mod tests {
    use super::{decoded_byte_size, get_or_fetch_at, reset_for_test, NEGATIVE_CACHE_TTL};
    use std::cell::Cell;
    use std::time::{Duration, Instant};

    #[test]
    fn decoded_byte_size_is_four_bytes_per_pixel() {
        assert_eq!(decoded_byte_size(1280, 853), 1280 * 853 * 4);
    }

    #[test]
    fn decoded_byte_size_never_underflows_on_a_degenerate_dimension() {
        assert_eq!(decoded_byte_size(0, 100), 0);
        assert_eq!(decoded_byte_size(-1, 100), 0);
    }

    /// TDD 2.26k's decidable core, exercised against the SHIPPED thread-local rather than
    /// a cache the test constructed: a failed URL is not re-fetched inside its TTL, and
    /// IS re-attempted once past it.
    ///
    /// This is what the clock seam buys. Before it, the only way to reach this behaviour
    /// was to sleep for the real TTL — so nothing tested it, and the rubric's coverage
    /// line could only point at `policy`'s own tests over a private cache.
    #[test]
    fn the_shipped_cache_re_attempts_a_dead_url_only_after_its_ttl() {
        reset_for_test();
        let attempts = Cell::new(0usize);
        let failing = || {
            attempts.set(attempts.get() + 1);
            None
        };
        let t0 = Instant::now();

        assert!(get_or_fetch_at("https://dead.invalid/a.png", t0, failing).is_none());
        assert_eq!(attempts.get(), 1, "the first toggle attempts the fetch");

        // Several more toggles inside the TTL.
        for tick in 1..=5 {
            let at = t0 + Duration::from_secs(tick);
            assert!(get_or_fetch_at("https://dead.invalid/a.png", at, failing).is_none());
        }
        assert_eq!(
            attempts.get(),
            1,
            "no toggle inside the TTL re-enters the fetch — the frequency contract"
        );

        // ...and past it, exactly one more attempt, which is the design and not a defect:
        // a longer TTL would make a transient outage read as permanent for the session.
        let past = t0 + NEGATIVE_CACHE_TTL + Duration::from_secs(1);
        assert!(get_or_fetch_at("https://dead.invalid/a.png", past, failing).is_none());
        assert_eq!(attempts.get(), 2, "one re-attempt once the TTL has lapsed");
        reset_for_test();
    }
}

/// **The pair is split by whether a body needs a real `GdkTexture`, not by subject.**
///
/// Both tests below drive the shipped thread-local cache. The one that survives here
/// stores a texture, which needs GTK initialised; the TTL guard next door stores
/// nothing at all — its fetch closure returns `None` — so it needs no display, and
/// gating it here kept it out of `cargo test` and out of coverage leg A entirely
/// (F-TEST-B-004). Stated because the next test in this area will otherwise be filed on
/// whichever side it is written next to.
#[cfg(all(test, feature = "gtk-integration-tests"))]
mod gtk_integration_tests {
    use super::*;
    use std::cell::Cell;

    /// A 1×1 texture, the cheapest real `GdkTexture` — the cache stores textures, and
    /// substituting anything else would test a different `Cache<V>`.
    fn pixel() -> gtk::gdk::Texture {
        use gtk::prelude::Cast;
        let bytes = gtk::glib::Bytes::from_owned(vec![0u8, 0, 0, 255]);
        gtk::gdk::MemoryTexture::new(1, 1, gtk::gdk::MemoryFormat::R8g8b8a8, &bytes, 4)
            .upcast::<gtk::gdk::Texture>()
    }

    /// A cached SUCCESS never re-enters the fetch either — the other half of the choke
    /// point, over the real cache.
    #[gtktest::test]
    fn the_shipped_cache_never_re_fetches_a_hit() {
        reset_for_test();
        let attempts = Cell::new(0usize);
        let ok = || {
            attempts.set(attempts.get() + 1);
            Some((pixel(), AnimationHint::Still))
        };
        let t0 = Instant::now();

        assert!(get_or_fetch_at("https://live.invalid/b.png", t0, ok).is_some());
        for tick in 1..=4 {
            let at = t0 + Duration::from_secs(tick * 30);
            assert!(get_or_fetch_at("https://live.invalid/b.png", at, ok).is_some());
        }
        assert_eq!(
            attempts.get(),
            1,
            "a positive entry answers every later toggle, and never expires by time"
        );
        reset_for_test();
    }

    /// A still image on a cache hit still carries no animation bytes — the counterpart
    /// to the animated hit tests in `imagecache::loader` and below (TDD 27.1).
    #[gtktest::test]
    fn a_still_image_carries_no_animation_hint_on_a_hit() {
        reset_for_test();
        let key = "https://live.invalid/still.png";

        let miss = get_or_fetch(key, || Some((pixel(), AnimationHint::Still))).expect("miss");
        assert!(matches!(miss.animation, AnimationHint::Still));

        let hit = get_or_fetch(key, || panic!("a hit must not re-fetch")).expect("hit");
        assert!(matches!(hit.animation, AnimationHint::Still));
    }

    /// **TDD 27.1.** A remote animated image's cache entry retains its encoded
    /// bytes, so a HIT answers with them directly — its only alternative is a second
    /// network fetch on the opt-in "Show Unsafe Images" path, which is worse (see the
    /// module doc's "Animation" section).
    ///
    /// Mutation: leaving the retained bytes out of the STORED hint (storing
    /// `AnimationHint::Still` instead of `AnimationHint::Remote(bytes)` on a miss)
    /// reddens the second assertion — the mutation this test exists to catch.
    #[gtktest::test]
    fn a_remote_animation_hit_never_re_fetches_and_shares_the_bytes() {
        reset_for_test();
        let key = "https://live.invalid/anim.webp";
        let bytes: Arc<[u8]> = Arc::from(&b"encoded animated bytes"[..]);
        let fetch_calls = Cell::new(0usize);

        let first = get_or_fetch(key, || {
            fetch_calls.set(fetch_calls.get() + 1);
            Some((pixel(), AnimationHint::Remote(bytes.clone())))
        })
        .expect("miss decodes");
        match first.animation {
            AnimationHint::Remote(got) => assert!(Arc::ptr_eq(&got, &bytes)),
            _ => panic!("expected AnimationHint::Remote on the miss"),
        }

        let second = get_or_fetch(key, || {
            fetch_calls.set(fetch_calls.get() + 1);
            Some((pixel(), AnimationHint::Remote(bytes.clone())))
        })
        .expect("hit answers without calling this closure");
        match second.animation {
            AnimationHint::Remote(got) => {
                assert!(
                    Arc::ptr_eq(&got, &bytes),
                    "the SAME retained bytes on a hit"
                )
            }
            _ => panic!("expected AnimationHint::Remote on the hit"),
        }
        assert_eq!(fetch_calls.get(), 1, "a hit must not re-fetch");
    }

    /// **TDD 6.6-family.** Retained remote animation bytes must be charged
    /// against the cache's own byte budget, or an oversized entry never gets evicted.
    ///
    /// Mutation: computing this entry's cached size from [`decoded_byte_size`] alone
    /// (never adding the retained bytes) reddens this — the second entry then fits
    /// alongside the first under the (wrong) accounting, the first is never evicted,
    /// and the final re-request answers from the stale cache with `refetched` staying
    /// `false` — the mutation this test exists to catch.
    #[gtktest::test]
    fn retained_remote_bytes_are_charged_against_the_budget_and_can_evict() {
        reset_for_test();
        // Bigger than half of IMAGE_CACHE_BUDGET_BYTES (32 MiB), so two of these
        // cannot coexist unless the retained bytes are (wrongly) left uncounted.
        const RETAINED: usize = 20 * 1024 * 1024;
        let heavy_bytes: Arc<[u8]> = vec![0u8; RETAINED].into();

        let _ = get_or_fetch("https://live.invalid/big-a.webp", || {
            Some((pixel(), AnimationHint::Remote(heavy_bytes.clone())))
        });
        let _ = get_or_fetch("https://live.invalid/big-b.webp", || {
            Some((pixel(), AnimationHint::Remote(heavy_bytes.clone())))
        });

        let refetched = Cell::new(false);
        let _ = get_or_fetch("https://live.invalid/big-a.webp", || {
            refetched.set(true);
            Some((pixel(), AnimationHint::Remote(heavy_bytes.clone())))
        });
        assert!(
            refetched.get(),
            "retained remote animation bytes must be charged against the cache's byte \
             budget, or an oversized entry never gets evicted"
        );
        reset_for_test();
    }
}
