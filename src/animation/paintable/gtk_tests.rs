//! `AnimatedPaintable`'s GTK-object tests, split out at POLICY's file-size soft
//! limit exactly as `palette` splits its own (`mod`/`tests`).

use super::*;
use crate::animation::policy::EnableAnimationsGuard;
use std::path::PathBuf;

/// `pub(super)`: shared with `badge_tests.rs`, the sibling test file for the
/// pause badge — both need the same fixture/harness helpers, and this stays
/// their one definition rather than a second copy (POLICY DRY expectations,
/// same reasoning `gtk_suite.rs`'s module list gate exists for elsewhere).
pub(super) fn anim_bytes() -> Arc<[u8]> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/anim.webp");
    Arc::from(std::fs::read(&path).expect("fixture reads"))
}

pub(super) fn test_app(suffix: &str) -> gtk::Application {
    crate::window::testkit::test_app_suffixed(&format!("animpaintable.{suffix}"))
}

/// A realized, mapped window hosting `pic` — real root, real application,
/// real frame clock. `pic` is left un-parented by the caller; this is what
/// parents it.
pub(super) fn realize(app: &gtk::Application, pic: &gtk::Picture) -> gtk::ApplicationWindow {
    let window = gtk::ApplicationWindow::new(app);
    window.set_child(Some(pic));
    window.present();
    crate::testpump::until(crate::testpump::Clock::Idle, "the window to map", || {
        pic.is_mapped()
    });
    window
}

/// The painted content right now, via `GdkTexture::download` — `GdkTexture`
/// has no public identity accessor, but its downloaded bytes change exactly
/// when the frame does, which is what every assertion below actually needs:
/// "did the painted content change", not "is this the same GObject". The
/// WHOLE texture, not just a corner pixel: `anim.webp`'s frames may differ
/// only in a sub-region, so comparing one fixed pixel could miss a real
/// frame change and read as "stuck" when it is not.
fn painted_bytes(paintable: &gtk::gdk::Paintable) -> Vec<u8> {
    use gtk::prelude::Cast;
    let texture = paintable
        .current_image()
        .downcast::<gtk::gdk::Texture>()
        .unwrap_or_else(|p| {
            // `AnimatedPaintable::current_image` always returns a Texture
            // (see its own doc comment) — this arm is unreachable for it in
            // practice, but a texture is what every path here actually
            // needs, so fail loudly rather than silently reading a stale
            // image on some other paintable shape.
            panic!("expected a Texture-shaped current image, got {p:?}")
        });
    // `download` requires `stride >= 4*width` and a buffer sized for the
    // WHOLE image — a too-small one panics rather than reading a corner.
    let stride = texture.width() as usize * 4;
    let mut buf = vec![0u8; stride * texture.height() as usize];
    texture.download(&mut buf, stride);
    buf
}

/// TDD 27.1/27.4: an animated WebP in a realized window advances its
/// frames under the REAL frame clock — asserted on the painted pixels
/// changing, never a sleep.
#[gtktest::test]
fn an_animated_webp_advances_its_frames_under_the_real_frame_clock() {
    let _enable = EnableAnimationsGuard::set(true);
    let app = test_app("advances");
    let pic = gtk::Picture::new();
    let animated = AnimatedPaintable::new(pic.upcast_ref(), anim_bytes()).expect("fixture decodes");
    pic.set_paintable(Some(&animated));
    let _window = realize(&app, &pic);

    assert!(
        animated.tick_installed(),
        "autoplay must install the tick callback once policy and root resolve"
    );
    let first = painted_bytes(pic.paintable().as_ref().unwrap());
    let changed = crate::testpump::until_or_for(
        crate::testpump::Clock::Frame,
        Duration::from_secs(10),
        || painted_bytes(pic.paintable().as_ref().unwrap()) != first,
    );
    assert!(
        changed,
        "the painted pixels never changed across 10s of wall clock"
    );
}

/// TDD 27.4: play/pause via `app.play-animations` — pausing removes the
/// tick callback and leaves the CURRENT frame (never resets to frame 0);
/// resuming reinstalls it and the sequence continues.
#[gtktest::test]
fn play_pause_via_the_action_freezes_and_resumes() {
    let _enable = EnableAnimationsGuard::set(true);
    let app = test_app("playpause");
    add_play_animations_action(&app, true);
    let pic = gtk::Picture::new();
    let animated = AnimatedPaintable::new(pic.upcast_ref(), anim_bytes()).expect("fixture decodes");
    pic.set_paintable(Some(&animated));
    let _window = realize(&app, &pic);
    assert!(animated.tick_installed());

    // Let it advance past frame 0 so "never resets to 0" is a real
    // assertion rather than a coincidence.
    let frame0 = painted_bytes(pic.paintable().as_ref().unwrap());
    assert!(
        crate::testpump::until_or_for(
            crate::testpump::Clock::Frame,
            Duration::from_secs(10),
            || { painted_bytes(pic.paintable().as_ref().unwrap()) != frame0 }
        ),
        "precondition: playback must advance at least once before pausing"
    );

    app.change_action_state(policy::ACTION_NAME, &false.to_variant());
    crate::testpump::until(
        crate::testpump::Clock::Idle,
        "the pause to take effect",
        || !animated.tick_installed(),
    );
    let frozen = painted_bytes(pic.paintable().as_ref().unwrap());
    // Pump real wall-clock time with the callback removed: the frame must
    // not change on its own.
    std::thread::sleep(Duration::from_millis(300));
    gtk::glib::MainContext::default().iteration(false);
    assert_eq!(
        painted_bytes(pic.paintable().as_ref().unwrap()),
        frozen,
        "paused must freeze the CURRENT frame, not just stop advancing eventually"
    );
    assert_ne!(
        frozen, frame0,
        "must not have reset to frame 0 while pausing"
    );

    app.change_action_state(policy::ACTION_NAME, &true.to_variant());
    crate::testpump::until(
        crate::testpump::Clock::Idle,
        "resuming to reinstall the tick callback",
        || animated.tick_installed(),
    );
    assert!(
        crate::testpump::until_or_for(
            crate::testpump::Clock::Frame,
            Duration::from_secs(10),
            || { painted_bytes(pic.paintable().as_ref().unwrap()) != frozen }
        ),
        "resuming must continue the sequence, not sit frozen forever"
    );
}

/// TDD 27.6/27.7: `gtk-enable-animations = false` holds FRAME 0 (not
/// merely whatever frame was current), with no tick callback.
#[gtktest::test]
fn reduce_animations_holds_frame_zero_with_no_tick_callback() {
    let enable = EnableAnimationsGuard::set(true);
    let app = test_app("reduce");
    let pic = gtk::Picture::new();
    let animated = AnimatedPaintable::new(pic.upcast_ref(), anim_bytes()).expect("fixture decodes");
    pic.set_paintable(Some(&animated));
    let _window = realize(&app, &pic);

    let frame0 = painted_bytes(pic.paintable().as_ref().unwrap());
    assert!(
        crate::testpump::until_or_for(
            crate::testpump::Clock::Frame,
            Duration::from_secs(10),
            || { painted_bytes(pic.paintable().as_ref().unwrap()) != frame0 }
        ),
        "precondition: playback must advance before reduce-animations engages"
    );

    enable.set_live(false);
    crate::testpump::until(
        crate::testpump::Clock::Idle,
        "reduce-animations to take effect",
        || !animated.tick_installed(),
    );
    assert_eq!(
        painted_bytes(pic.paintable().as_ref().unwrap()),
        frame0,
        "reduce-animations must show FRAME 0, not whatever frame was current"
    );
}

/// TDD §27: dropping the picture drops the paintable, removes the
/// callback, and releases the decoder — a weak-ref assertion, the same
/// shape `memgate`'s 6.7 uses for a still texture.
#[gtktest::test]
fn dropping_the_picture_releases_the_paintable_and_its_decoder() {
    let _enable = EnableAnimationsGuard::set(true);
    let app = test_app("drop");
    let pic = gtk::Picture::new();
    let animated = AnimatedPaintable::new(pic.upcast_ref(), anim_bytes()).expect("fixture decodes");
    pic.set_paintable(Some(&animated));
    let window = realize(&app, &pic);
    assert!(
        animated.tick_installed(),
        "sanity: really playing before drop"
    );

    let weak = animated.downgrade();
    drop(animated);
    assert!(
        weak.upgrade().is_some(),
        "the PICTURE still holds a strong ref — dropping our own local must not free it yet"
    );

    window.set_child(gtk::Widget::NONE);
    drop(window);
    drop(pic);

    assert!(
        weak.upgrade().is_none(),
        "with the picture and window gone, the paintable (and its decoder) must be released"
    );
}

/// Finding 1 (QA, 2026-09-12): a decode that lands AFTER visibility is lost must be
/// DISCARDED — not stored and "dropped again the very next `recompute`", the premise
/// `on_decoded`'s old doc comment made. That premise was false: while a picture stays
/// invisible, nothing fires `recompute` again at all — `visibility::watch` calls
/// nothing further once it has reported `false`, and the tick that would otherwise
/// poll the schedule is already removed. Proven on a background-tab shape (unmap, not
/// scroll): `visibility::current`'s `is_mapped()` leg alone decides it here (this
/// fixture has no `GtkScrolledWindow` ancestor at all), which is the same predicate
/// `animation::visibility::gtk_tests`'s own `claim_3` proves genuinely unmaps.
#[gtktest::test]
fn a_decode_landing_after_visibility_is_lost_is_discarded_and_restarts_at_frame_zero() {
    let _enable = EnableAnimationsGuard::set(true);
    let app = test_app("late-decode");
    let pic = gtk::Picture::new();
    let animated = AnimatedPaintable::new(pic.upcast_ref(), anim_bytes()).expect("fixture decodes");
    pic.set_paintable(Some(&animated));
    let frame0_bytes = painted_bytes(pic.paintable().as_ref().unwrap());

    let other_page = gtk::Label::new(Some("other tab"));
    let stack = gtk::Stack::new();
    stack.add_titled(&pic, Some("pic"), "Picture");
    stack.add_titled(&other_page, Some("other"), "Other");
    stack.set_visible_child_name("pic");

    let window = gtk::ApplicationWindow::new(&app);
    window.set_default_size(200, 200);
    window.set_child(Some(&stack));
    window.present();
    crate::testpump::until(crate::testpump::Clock::Idle, "the picture to map", || {
        pic.is_mapped()
    });
    crate::testpump::until(crate::testpump::Clock::Idle, "playback to start", || {
        animated.tick_installed()
    });

    // Catch the exact window a decode is genuinely in flight: `start_decode` moves
    // `self.animation` out for the duration of the pool-thread decode, so
    // `decoder_active()` reads `false` while the tick callback is still installed
    // and no decode has yet landed to hand it back.
    let caught_in_flight = crate::testpump::until_or_for(
        crate::testpump::Clock::Worker,
        Duration::from_secs(10),
        || animated.tick_installed() && !animated.decoder_active(),
    );
    assert!(
        caught_in_flight,
        "precondition: never observed a decode in flight — cannot exercise the \
         race this fix defends"
    );

    // Lose visibility WHILE that decode is still running on the pool thread.
    stack.set_visible_child_name("other");
    crate::testpump::until(crate::testpump::Clock::Idle, "the picture to unmap", || {
        !pic.is_mapped()
    });
    assert!(
        !animated.tick_installed(),
        "an invisible picture must stop ticking immediately, decode-in-flight or not"
    );

    // Let the in-flight decode actually land on the main context.
    crate::testpump::drain_for(crate::testpump::Clock::Worker, Duration::from_millis(500));

    assert!(
        !animated.decoder_active(),
        "the late decode landing while invisible must be DISCARDED, not stored — \
         storing it would resurrect the decoder for a picture nobody can see"
    );
    assert!(
        !animated.tick_installed(),
        "the late decode must not install a tick callback either"
    );

    // Return to view: playback must restart at frame 0, not resume mid-sequence —
    // `ensure_decoded`'s `animation.is_some()` early return would have skipped
    // rebuilding it entirely had the late decode been stored instead of discarded.
    stack.set_visible_child_name("pic");
    crate::testpump::until(crate::testpump::Clock::Idle, "the picture to remap", || {
        pic.is_mapped()
    });
    crate::testpump::until(crate::testpump::Clock::Idle, "playback to resume", || {
        animated.decoder_active()
    });
    assert!(
        animated.tick_installed(),
        "returning to view must reinstall the tick callback"
    );
    assert_eq!(
        painted_bytes(pic.paintable().as_ref().unwrap()),
        frame0_bytes,
        "returning to view must restart playback from frame 0, not resume \
         mid-sequence from wherever the discarded decode had left it"
    );
    window.destroy();
}

/// **The `decoder_generation` guard's first test.** Lose visibility while a decode is
/// in flight, REGAIN it before that decode lands, and the frame that arrives belongs to
/// a decoder incarnation that has already been torn down.
///
/// Until now this guard had no coverage at all — it could be deleted and the suite
/// stayed green. The only late-decode test beside it loses visibility and stays lost,
/// which the sibling `play_wanted` guard decides by itself; the generation check never
/// got to answer. Its own doc comment says it exists for lose-then-REGAIN, and that is
/// exactly the case nothing exercised.
///
/// It is reachable only while a decode is slow enough to outlive a visibility round
/// trip, which a local decode never is — hence `worker::slow_decode`, the seam ported
/// from `docio::pool` for precisely this class of window.
#[gtktest::test]
fn a_decode_outlived_by_its_own_decoder_is_dropped_even_though_the_picture_is_visible_again() {
    let _enable = EnableAnimationsGuard::set(true);
    // Long enough that a decode dispatched before the unmap is still on the pool
    // thread after the remap has built a fresh decoder.
    let _slow = crate::animation::worker::slow_decode(Duration::from_millis(600));

    let app = test_app("stale-generation");
    let pic = gtk::Picture::new();
    let animated = AnimatedPaintable::new(pic.upcast_ref(), anim_bytes()).expect("fixture decodes");
    pic.set_paintable(Some(&animated));

    let other_page = gtk::Label::new(Some("other tab"));
    let stack = gtk::Stack::new();
    stack.add_titled(&pic, Some("pic"), "Picture");
    stack.add_titled(&other_page, Some("other"), "Other");
    stack.set_visible_child_name("pic");

    let window = gtk::ApplicationWindow::new(&app);
    window.set_default_size(200, 200);
    window.set_child(Some(&stack));
    window.present();
    crate::testpump::until(crate::testpump::Clock::Idle, "the picture to map", || {
        pic.is_mapped()
    });
    crate::testpump::until(crate::testpump::Clock::Idle, "playback to start", || {
        animated.tick_installed()
    });

    // The same in-flight detector the sibling test uses: `start_decode` moves the
    // animation out for the decode's duration, so the decoder reads inactive while
    // the tick is still installed.
    let caught_in_flight = crate::testpump::until_or_for(
        crate::testpump::Clock::Worker,
        Duration::from_secs(10),
        || animated.tick_installed() && !animated.decoder_active(),
    );
    assert!(
        caught_in_flight,
        "precondition: never observed a decode in flight — cannot exercise the race \
         this guard defends"
    );
    let stale_generation = animated.decoder_generation_for_test();

    // Lose visibility, then REGAIN it, both while that decode is still on the pool.
    stack.set_visible_child_name("other");
    crate::testpump::until(crate::testpump::Clock::Idle, "the picture to unmap", || {
        !pic.is_mapped()
    });
    stack.set_visible_child_name("pic");
    crate::testpump::until(crate::testpump::Clock::Idle, "the picture to remap", || {
        pic.is_mapped()
    });

    let live_generation = animated.decoder_generation_for_test();
    assert_ne!(
        live_generation, stale_generation,
        "precondition: the round trip must have torn the decoder down and built a \
         fresh one, or the decode that lands is not stale and this proves nothing"
    );

    // Let the stale decode land on the main context.
    crate::testpump::drain_for(crate::testpump::Clock::Worker, Duration::from_millis(900));

    // The guard's whole job: the frame belongs to the previous incarnation, and the
    // current one must be untouched by it. Visibility says PLAY here, so `play_wanted`
    // cannot be what refuses it — only the generation check can.
    assert_eq!(
        animated.decoder_generation_for_test(),
        live_generation,
        "a stale frame must not disturb the live incarnation into rebuilding"
    );
    assert!(
        animated.tick_installed(),
        "the visible picture must still be playing after the stale frame was dropped"
    );
    // The assertion with teeth, and the reason it counts rather than inspects: a
    // correct refusal leaves NO trace. The schedule's awaiting latch, the decoder's
    // presence and the pixels are all re-established by the live incarnation's own
    // decode a moment later, so anything read after the fact cannot distinguish
    // "refused" from "accepted and then overwritten". A first version of this test
    // asserted the awaiting latch and passed with the guard deleted.
    assert_eq!(
        animated.stale_decodes_dropped_for_test(),
        1,
        "the stale frame was not refused by the generation guard — it belongs to an \
         incarnation that was torn down, and the live schedule was restarted when \
         this decoder was built, so it holds no request that decode could satisfy"
    );
    window.destroy();
}

/// Mirrors `policy.rs`'s own `add_bare_action` — this module's tests need
/// the SAME action shape `app::appactions::add_play_animations_action`
/// registers, without depending on that function (which reads the real
/// session) or on `policy`'s private test helper. `pub(super)`: also used
/// by `badge_tests.rs` — see `anim_bytes`'s doc comment above.
pub(super) fn add_play_animations_action(app: &gtk::Application, initial: bool) {
    let action =
        gtk::gio::SimpleAction::new_stateful(policy::ACTION_NAME, None, &initial.to_variant());
    action.connect_change_state(|act, value| {
        let Some(value) = value else { return };
        act.set_state(value);
    });
    app.add_action(&action);
}
