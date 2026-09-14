//! The table-row tests, split from `gtk_tests/mod.rs` at POLICY's 500-line soft
//! limit — one test per row of the plan's "not visible because…" table this
//! project can produce headlessly, plus the return-to-view restart test.

use super::*;

// ---------------------------------------------------------------------------
// One test per table row this project can actually produce headlessly.
// ---------------------------------------------------------------------------

/// Scrolled out of the preview's viewport: stops playing and releases the
/// decoder; scrolling back plays again (folded into
/// `return_to_view_restarts_at_frame_zero` below, which needs the same
/// setup — this test is the "stops" half on its own, kept separate so a
/// failure names exactly which half broke).
#[gtktest::test]
fn table_scrolled_out_of_viewport_stops_playing_and_releases_the_decoder() {
    let _enable = EnableAnimationsGuard::set(true);
    let app = test_app("scrolledout");
    let (view, pic, animated) = build_scrollable_animation(2, 400);
    let (window, sw) = realize_scrolled(&app, &view, &animated, 300, 150);
    assert!(animated.tick_installed(), "sanity: playing while visible");
    assert!(animated.decoder_active(), "sanity: decoding while visible");

    // Re-issue the scroll to the CURRENT bottom on every poll — `upper()` is a
    // lazily-validated draft right after mapping (GTK4Rs/AP-13 family) and can
    // grow over several idle turns, so one scroll computed against today's
    // `upper()` can undershoot the true end of the document.
    crate::testpump::until(
        crate::testpump::Clock::Idle,
        "the visibility watch to react to the scroll",
        {
            let animated = animated.clone();
            let sw = sw.clone();
            move || {
                let vadj = sw.vadjustment();
                scroll_to(&sw, vadj.upper() - vadj.page_size());
                !animated.tick_installed()
            }
        },
    );
    assert!(
        !animated.tick_installed(),
        "no tick callback while scrolled away"
    );
    assert!(
        !animated.decoder_active(),
        "no decoder held while scrolled away — PLAN.memory-gates.md: \
         \"holds no more memory than its file\""
    );
    let _ = pic;
    window.destroy();
}

/// A `GtkScrolledWindow`/window resize — no scroll `value` change at all —
/// must ALSO be re-tested, because it changes `page_size`/`upper` (the
/// adjustment's `changed` signal), not `value`. This is the case mutation
/// test 3 (dropping the `changed` half of the wiring) is required to redden.
#[gtktest::test]
fn table_a_pure_resize_with_no_scroll_value_change_also_re_tests_visibility() {
    let _enable = EnableAnimationsGuard::set(true);
    let app = test_app("resize");
    let (view, pic, animated) = build_scrollable_animation(30, 30);
    let sw = gtk::ScrolledWindow::new();
    sw.set_child(Some(&view));

    // A `GtkWindow` gives its sole `set_child` the WHOLE content area
    // regardless of expand flags or size requests on that child — so
    // shrinking `sw`'s own size request would not shrink its ALLOCATION
    // while the window stays tall, and resizing an already-mapped toplevel
    // is not reliable under a bare Xvfb with no WM either. A `GtkPaned`
    // divider is neither: repositioning it is a normal, WM-independent
    // internal-layout operation that reallocates `sw` on the spot.
    let filler = gtk::Label::new(Some("filler"));
    let paned = gtk::Paned::new(gtk::Orientation::Vertical);
    paned.set_start_child(Some(&sw));
    paned.set_end_child(Some(&filler));
    // Tall enough that the picture — a couple dozen lines down — starts
    // inside the viewport at scroll value 0.
    paned.set_position(700);

    let window = gtk::ApplicationWindow::new(&app);
    window.set_default_size(300, 750);
    window.set_child(Some(&paned));
    window.present();
    {
        let view = view.clone();
        let sw = sw.clone();
        crate::testpump::until(
            crate::testpump::Clock::Idle,
            "the window to map",
            move || view.is_mapped() && sw.vadjustment().upper() > 0.0,
        );
    }
    wait_until_playing(&animated);

    assert!(
        visibility::geometry_visible(pic.upcast_ref(), sw.upcast_ref()),
        "precondition: visible in the tall pane, scrolled to the top"
    );
    assert_eq!(
        sw.vadjustment().value(),
        0.0,
        "precondition: value never moved"
    );
    assert!(animated.tick_installed(), "sanity: playing while visible");

    // Shrink `sw`'s ALLOCATION via the divider — `page_size` shrinks under
    // an UNCHANGED `value`.
    paned.set_position(60);
    crate::testpump::until(
        crate::testpump::Clock::Idle,
        "the shrunk viewport to re-test visibility",
        {
            let animated = animated.clone();
            move || !animated.tick_installed()
        },
    );
    assert_eq!(
        sw.vadjustment().value(),
        0.0,
        "the scroll VALUE must genuinely not have moved — this is what isolates \
         the `changed` signal from `value-changed`"
    );
    assert!(
        !animated.tick_installed(),
        "shrinking the viewport must stop playback even though `value` never changed"
    );
    assert!(!animated.decoder_active(), "and release the decoder");
    window.destroy();
}

/// Nested inside a `<details>` body: every anchored picture in this renderer
/// is already wrapped in a `GtkOverlay` before being anchored
/// (`renderer::start::anchor_image`), so `build_scrollable_animation` already
/// exercises multi-level `compute_bounds` for every OTHER test in this file.
/// This test is the explicit positive control: visible through the nesting,
/// with the SAME mechanism a `<details>` body's own extra indentation
/// wrapping (if any) would need.
#[gtktest::test]
fn table_nested_inside_another_widget_is_reached_through_the_nesting() {
    let _enable = EnableAnimationsGuard::set(true);
    let app = test_app("nested");
    let (view, pic, animated) = build_scrollable_animation(1, 1);
    let (window, sw) = realize_scrolled(&app, &view, &animated, 300, 300);

    assert!(
        pic.parent().is_some_and(|p| p.is::<gtk::Overlay>()),
        "sanity: the picture really is nested one level inside a GtkOverlay, \
         the shape every anchored image takes"
    );
    assert!(
        visibility::geometry_visible(pic.upcast_ref(), sw.upcast_ref()),
        "compute_bounds must walk the Overlay↔ScrolledWindow ancestor chain \
         correctly, not just a bare anchored child"
    );
    assert!(animated.tick_installed(), "and it actually plays");
    window.destroy();
}

/// Background tab: folds `claim_3`'s mechanism into the paintable itself.
#[gtktest::test]
fn table_background_tab_stops_playing_and_releases_the_decoder() {
    let _enable = EnableAnimationsGuard::set(true);
    let app = test_app("bgtab");
    let (view, _pic, animated) = build_scrollable_animation(1, 1);
    let sw = gtk::ScrolledWindow::new();
    sw.set_child(Some(&view));
    let other = gtk::Label::new(Some("other"));
    let stack = gtk::Stack::new();
    stack.add_named(&sw, Some("anim"));
    stack.add_named(&other, Some("other"));
    stack.set_visible_child_name("anim");

    let window = gtk::ApplicationWindow::new(&app);
    window.set_default_size(300, 300);
    window.set_child(Some(&stack));
    window.present();
    crate::testpump::until(crate::testpump::Clock::Idle, "the window to map", {
        let animated = animated.clone();
        move || animated.tick_installed()
    });

    stack.set_visible_child_name("other");
    crate::testpump::until(
        crate::testpump::Clock::Idle,
        "the background tab to stop the animation",
        {
            let animated = animated.clone();
            move || !animated.tick_installed()
        },
    );
    assert!(!animated.tick_installed());
    assert!(!animated.decoder_active());
    window.destroy();
}

/// Hidden preview pane (edit mode): `set_visible(false)` on the pane.
#[gtktest::test]
fn table_hidden_pane_stops_playing_and_releases_the_decoder() {
    let _enable = EnableAnimationsGuard::set(true);
    let app = test_app("hiddenpane");
    let (view, _pic, animated) = build_scrollable_animation(1, 1);
    let sw = gtk::ScrolledWindow::new();
    sw.set_child(Some(&view));
    let window = gtk::ApplicationWindow::new(&app);
    window.set_default_size(300, 300);
    window.set_child(Some(&sw));
    window.present();
    crate::testpump::until(crate::testpump::Clock::Idle, "the window to map", {
        let animated = animated.clone();
        move || animated.tick_installed()
    });

    sw.set_visible(false);
    crate::testpump::until(
        crate::testpump::Clock::Idle,
        "the hidden pane to stop the animation",
        {
            let animated = animated.clone();
            move || !animated.tick_installed()
        },
    );
    assert!(!animated.tick_installed());
    assert!(!animated.decoder_active());
    window.destroy();
}

/// Hidden window: `set_visible(false)` on the toplevel itself.
#[gtktest::test]
fn table_hidden_window_stops_playing_and_releases_the_decoder() {
    let _enable = EnableAnimationsGuard::set(true);
    let app = test_app("hiddenwindow");
    let (view, _pic, animated) = build_scrollable_animation(1, 1);
    let (window, _sw) = realize_scrolled(&app, &view, &animated, 300, 300);
    assert!(animated.tick_installed(), "sanity: playing before hiding");

    window.set_visible(false);
    crate::testpump::until(
        crate::testpump::Clock::Idle,
        "the hidden window to stop the animation",
        {
            let animated = animated.clone();
            move || !animated.tick_installed()
        },
    );
    assert!(!animated.tick_installed());
    assert!(!animated.decoder_active());
    window.destroy();
}

/// TDD 27.3's other half: coming back into view plays again from frame 0 —
/// never resuming mid-loop.
#[gtktest::test]
fn return_to_view_restarts_at_frame_zero() {
    let _enable = EnableAnimationsGuard::set(true);
    let app = test_app("restart");
    let (view, _pic, animated) = build_scrollable_animation(2, 400);
    let (window, sw) = realize_scrolled(&app, &view, &animated, 300, 150);
    assert!(animated.tick_installed());

    let frame0 = painted_bytes(&animated.clone().upcast());
    assert!(
        crate::testpump::until_or_for(
            crate::testpump::Clock::Frame,
            std::time::Duration::from_secs(10),
            {
                let animated = animated.clone();
                let frame0 = frame0.clone();
                move || painted_bytes(&animated.clone().upcast()) != frame0
            }
        ),
        "precondition: playback must advance past frame 0 before this test means \
         anything"
    );

    // Re-issue the scroll to the CURRENT bottom on every poll — see
    // `table_scrolled_out_of_viewport_stops_playing_and_releases_the_decoder`'s
    // comment on why one scroll against today's (lazily-validated) `upper()`
    // is not enough.
    crate::testpump::until(crate::testpump::Clock::Idle, "scrolling away to stop it", {
        let animated = animated.clone();
        let sw = sw.clone();
        move || {
            let vadj = sw.vadjustment();
            scroll_to(&sw, vadj.upper() - vadj.page_size());
            !animated.tick_installed()
        }
    });
    assert!(
        !animated.decoder_active(),
        "sanity: decoder released while away"
    );

    scroll_to(&sw, 0.0);
    crate::testpump::until(
        crate::testpump::Clock::Idle,
        "scrolling back to resume it",
        {
            let animated = animated.clone();
            move || animated.tick_installed()
        },
    );

    assert_eq!(
        painted_bytes(&animated.clone().upcast()),
        frame0,
        "coming back into view must show FRAME 0, not resume mid-loop — the \
         decoder was rebuilt from the shared bytes, not kept"
    );
    window.destroy();
}

/// TDD 27.3's other half, from the other direction: a picture BELOW the viewport at load
/// starts playing when ONE scroll brings it into view. The tests above re-issue their
/// scroll on every poll, which would also re-run the visibility check and so hide a check
/// that read the picture's position before layout had moved it — measured on a theme
/// sprite's host, where the only report after such a scroll said hidden.
#[gtktest::test]
fn a_picture_scrolled_into_view_from_below_by_one_scroll_starts_playing() {
    let _enable = EnableAnimationsGuard::set(true);
    let app = test_app("frombelow");
    let (view, _pic, animated) = build_scrollable_animation(400, 2);
    let sw = gtk::ScrolledWindow::new();
    sw.set_child(Some(&view));
    let window = gtk::ApplicationWindow::new(&app);
    window.set_default_size(300, 150);
    window.set_child(Some(&sw));
    window.present();

    // Let the lazily-validated `upper` settle before the one scroll, so the scroll
    // lands at the true end rather than a draft of it.
    let mut last_upper = -1.0;
    crate::testpump::until(
        crate::testpump::Clock::Frame,
        "the document height to settle",
        || {
            crate::testpump::drain_for(
                crate::testpump::Clock::Frame,
                std::time::Duration::from_millis(200),
            );
            let upper = sw.vadjustment().upper();
            let settled = view.is_mapped() && upper > 0.0 && upper == last_upper;
            last_upper = upper;
            settled
        },
    );
    assert!(
        !animated.tick_installed(),
        "precondition: the picture starts below the viewport, so it must not be playing"
    );

    let vadj = sw.vadjustment();
    scroll_to(&sw, vadj.upper() - vadj.page_size());
    let started = crate::testpump::until_or_for(
        crate::testpump::Clock::Frame,
        std::time::Duration::from_secs(5),
        || animated.tick_installed(),
    );
    window.destroy();
    assert!(
        started,
        "one scroll brought the picture into view and it never started playing"
    );
}
