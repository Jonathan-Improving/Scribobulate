//! WP10 (TDD 27.9, `sdd/PLAN.memory-gates.md` "Theme sprites animate too") — does an
//! animated heading-band sprite actually play through the real paint plan, and does
//! `decorplan`'s own viewport gate (`bandpaint::paint_band`'s `span.is_outside` check)
//! genuinely stand in for this sprite's visibility, exactly as the plan requires
//! rather than a second geometry check invented for the purpose?
//!
//! `crate::animation::sprites::gtk_tests` proves the DRIVER (ticking, policy, frame
//! advance) in isolation, calling `frame_for` directly; this module proves it is wired
//! up correctly from a real themed document, through `bandpaint::paint_band`, and that
//! scrolling the decoration off screen — the one thing only a real paint pass can show
//! — actually releases it.

use super::painttest::{framebuffer_of, present_for_paint_sized};
use super::CodePreviewView;
use crate::sprite::SpriteRef;
use gtk::prelude::*;

fn animated_sprite_file() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("band.webp");
    std::fs::write(
        &path,
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/anim.webp"
        )),
    )
    .expect("write fixture");
    (dir, path)
}

/// A theme naming `path` as `heading_band_sprite_h1`, activated for the test.
fn activate_banded_theme(path: &std::path::Path) -> crate::theme::ActiveThemeGuard {
    let mut themes = crate::theme::themes();
    themes.merge_over_for_test(
        "[themes.animbanded]\nbackground = \"#ffffff\"\nforeground = \"#000000\"\n",
    );
    let mut theme = themes.resolve("animbanded");
    theme.sprites.heading_band[0] = Some(SpriteRef::File(path.to_path_buf()));
    let guard = crate::theme::activate_for_test(theme);
    crate::sprite::clear_cache();
    guard
}

/// The `SpriteRef` this theme's `heading_band[0]` resolves to, for asking the driver
/// directly about it.
fn sprite_ref(path: &std::path::Path) -> SpriteRef {
    SpriteRef::File(path.to_path_buf())
}

/// TDD 27.9 / 27.1: a heading band whose sprite is an animated WebP PLAYS — its
/// painted pixels change under the real frame clock, asserted the same way
/// `animation::paintable::gtk_tests` and `animation::sprites::gtk_tests` do (never a
/// sleep), applied here to the FULL painted framebuffer of a real themed document.
#[gtktest::test]
fn an_animated_heading_band_sprite_plays_while_the_heading_is_in_view() {
    let (_dir, path) = animated_sprite_file();
    let _theme = activate_banded_theme(&path);

    let view = CodePreviewView::new();
    view.buffer().set_text("A banded heading\n");
    view.set_heading_spans(vec![crate::renderer::HeadingSpan {
        span: crate::span::BufferSpan::new(0, 16),
        level_index: 0,
    }]);
    let app = crate::window::testkit::test_app_suffixed("animsprite.plays");
    let window = present_for_paint_sized(&view, 400, 200);
    window.set_application(Some(&app));

    let first = framebuffer_of(&view, 400.0, 200.0);
    let changed = crate::testpump::until_or_for(
        crate::testpump::Clock::Frame,
        std::time::Duration::from_secs(10),
        || framebuffer_of(&view, 400.0, 200.0) != first,
    );
    window.destroy();
    assert!(
        changed,
        "the banded heading's painted pixels never changed across 10s of wall clock"
    );
    crate::sprite::clear_cache();
}

/// TDD 27.3 / 27.9: scrolling the banded heading OUT of `decorplan`'s own viewport
/// gate drops the animation driver's whole entry — no tick, no decoder — and scrolling
/// it back plays it again from frame 0. This is the ONE test that can prove the
/// viewport gate really is this sprite's visibility signal, because only a real
/// `snapshot_layer` pass (not `frame_for` called directly) exercises
/// `bandpaint::paint_band`'s `span.is_outside` early return at all.
#[gtktest::test]
fn scrolling_the_banded_heading_off_screen_drops_the_sprite_driver_and_stops_ticking() {
    let (_dir, path) = animated_sprite_file();
    let _theme = activate_banded_theme(&path);
    let r = sprite_ref(&path);

    // Enough blank lines that, at scroll position 0, the banded heading — placed
    // AFTER them — is well past the 200px-tall viewport. Placed after rather than
    // before so scrolling to the adjustment's UPPER bound reveals it without having
    // to compute an exact target pixel.
    let mut text = String::new();
    for _ in 0..80 {
        text.push_str("blank line of ordinary text\n");
    }
    let heading_start = text.chars().count() as i32;
    text.push_str("A banded heading\n");
    let heading_end = heading_start + "A banded heading".chars().count() as i32;

    let view = CodePreviewView::new();
    view.buffer().set_text(&text);
    view.set_heading_spans(vec![crate::renderer::HeadingSpan {
        span: crate::span::BufferSpan::new(heading_start, heading_end),
        level_index: 0,
    }]);
    let app = crate::window::testkit::test_app_suffixed("animsprite.scroll");
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_child(Some(&view));
    let window = gtk::Window::new();
    window.set_default_size(400, 200);
    window.set_child(Some(&scroller));
    window.set_application(Some(&app));
    window.present();
    crate::testpump::until(crate::testpump::Clock::Frame, "the preview maps", || {
        view.width() > 0
    });

    // At the top, the heading is below the viewport — decorplan's own visibility gate
    // must refuse it every time, so nothing about it is ever scheduled.
    let _ = framebuffer_of(&view, 400.0, 200.0);
    assert!(
        view.with_sprite_anim(&r, |_| ()).is_none(),
        "the banded heading starts below the viewport — no driver entry must exist yet"
    );

    // Scroll the heading's own line to just inside the top of the viewport — the same
    // technique `a_bar_sprite_still_tiles_far_down_a_long_document` uses (a plain
    // `vadj.upper() - page_size()` is not enough on its own: GTK validates line
    // heights incrementally, so `upper` can under-report until the LAST line's own
    // `line_yrange` has actually settled).
    let vadj = view
        .vadjustment()
        .expect("a scrolled view carries a vadjustment");
    let heading_iter = view.buffer().iter_at_offset(heading_start);
    // The document is short (81 lines) — a couple of frames is ample for GTK's
    // incremental line-height validation to reach the last one; a fixed short drain
    // is simpler than a threshold check here and this document has no 32768px-class
    // depth to guard against (unlike the blockquote-bar regression test above).
    crate::testpump::drain_for(
        crate::testpump::Clock::Frame,
        std::time::Duration::from_millis(200),
    );
    let heading_y = f64::from(view.line_yrange(&heading_iter).0);
    crate::saferizer::scrollpos::jump(&vadj, heading_y - 20.0);
    crate::testpump::drain_for(
        crate::testpump::Clock::Frame,
        std::time::Duration::from_millis(50),
    );
    let _ = framebuffer_of(&view, 400.0, 200.0);

    assert!(
        view.with_sprite_anim(&r, |_| ()).is_some(),
        "the banded heading is now on screen — the driver must have been created"
    );
    assert!(
        view.with_sprite_anim(&r, |anim| anim.is_ticking())
            .unwrap_or(false),
        "a visible, policy-enabled animation must be ticking"
    );

    // Scroll back to the top and force another fresh paint — the heading is off
    // screen again, and NOTHING calls `frame_for` for it on this pass.
    crate::saferizer::scrollpos::jump(&vadj, 0.0);
    crate::testpump::drain_for(
        crate::testpump::Clock::Frame,
        std::time::Duration::from_millis(50),
    );
    let _ = framebuffer_of(&view, 400.0, 200.0);

    window.destroy();
    assert!(
        view.with_sprite_anim(&r, |_| ()).is_none(),
        "scrolling the decoration off screen must drop its animation driver entirely — \
         no tick, no decoder held for pixels nobody can see"
    );
    crate::sprite::clear_cache();
}

/// Finding 2 (QA, 2026-09-12): a background tab retains every sprite decoder
/// INDEFINITELY under the paint-driven pruning alone — `snapshot_layer` never runs
/// for an unmapped widget, so `reset_sprite_anim_seen`/`drop_unseen_sprite_anims`
/// never run either, and an entry that is never pruned is never released.
/// `CodePreviewView::ensure_sprite_visibility_watch`'s `animation::visibility`
/// subscription is what closes that gap: switching this view's own tab away must
/// release the driver's decoder and tick, and switching back must rebuild it from
/// frame 0. The scrolled-out case (a real paint pass, decorplan's own gate) is the
/// OTHER test above, re-verified unaffected by this fix rather than duplicated here.
#[gtktest::test]
fn a_background_tab_releases_the_sprite_driver_and_returning_restores_it() {
    let (_dir, path) = animated_sprite_file();
    let _theme = activate_banded_theme(&path);
    let r = sprite_ref(&path);

    let view = CodePreviewView::new();
    view.buffer().set_text("A banded heading\n");
    view.set_heading_spans(vec![crate::renderer::HeadingSpan {
        span: crate::span::BufferSpan::new(0, 16),
        level_index: 0,
    }]);

    let app = crate::window::testkit::test_app_suffixed("animsprite.tab");
    let other_page = gtk::Label::new(Some("other tab"));
    let stack = gtk::Stack::new();
    stack.add_titled(&view, Some("preview"), "Preview");
    stack.add_titled(&other_page, Some("other"), "Other");
    stack.set_visible_child_name("preview");

    let window = gtk::ApplicationWindow::new(&app);
    window.set_default_size(400, 200);
    window.set_child(Some(&stack));
    window.present();
    crate::testpump::until(crate::testpump::Clock::Frame, "the preview maps", || {
        view.is_mapped()
    });

    let _ = framebuffer_of(&view, 400.0, 200.0);
    assert!(
        view.with_sprite_anim(&r, |_| ()).is_some(),
        "precondition: the banded heading is on screen, so the driver must hold an \
         entry"
    );
    assert!(
        view.with_sprite_anim(&r, |anim| anim.is_ticking())
            .unwrap_or(false),
        "precondition: a visible, policy-enabled animation must be ticking"
    );

    // Switch to another tab: the view unmaps, and no paint runs against it ever
    // again while it stays hidden — the paint-driven pruning above cannot see this
    // case at all.
    stack.set_visible_child_name("other");
    crate::testpump::until(crate::testpump::Clock::Idle, "the view to unmap", || {
        !view.is_mapped()
    });

    assert!(
        view.with_sprite_anim(&r, |_| ()).is_none(),
        "a background tab must release the sprite driver's entry — no decoder, no \
         tick callback held for pixels nobody can see"
    );

    // The CPU half of the finding, not just the memory half: nothing re-creates an
    // entry with no paint running, so it must stay dropped across real wall-clock
    // time rather than being recreated by some other path.
    crate::testpump::drain_for(
        crate::testpump::Clock::Frame,
        std::time::Duration::from_millis(200),
    );
    assert!(
        view.with_sprite_anim(&r, |_| ()).is_none(),
        "must stay dropped while backgrounded"
    );

    // Switch back: the next real paint rebuilds the driver from frame 0.
    stack.set_visible_child_name("preview");
    crate::testpump::until(crate::testpump::Clock::Idle, "the view to remap", || {
        view.is_mapped()
    });
    let _ = framebuffer_of(&view, 400.0, 200.0);

    assert!(
        view.with_sprite_anim(&r, |_| ()).is_some(),
        "returning to the foreground tab must rebuild the sprite driver's entry"
    );
    assert!(
        view.with_sprite_anim(&r, |anim| anim.is_ticking())
            .unwrap_or(false),
        "the rebuilt driver must be ticking again, exactly as before backgrounding"
    );

    window.destroy();
    crate::sprite::clear_cache();
}

/// Finding 2's hidden-pane variant: `set_visible(false)` unmaps content exactly as a
/// background tab does (`animation::visibility::gtk_tests`'s own `claim_3` proves
/// both shapes with the same predicate), so the same release must happen through the
/// same mechanism.
#[gtktest::test]
fn a_hidden_pane_releases_the_sprite_driver_and_returning_restores_it() {
    let (_dir, path) = animated_sprite_file();
    let _theme = activate_banded_theme(&path);
    let r = sprite_ref(&path);

    let view = CodePreviewView::new();
    view.buffer().set_text("A banded heading\n");
    view.set_heading_spans(vec![crate::renderer::HeadingSpan {
        span: crate::span::BufferSpan::new(0, 16),
        level_index: 0,
    }]);

    let app = crate::window::testkit::test_app_suffixed("animsprite.hide");
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_child(Some(&view));
    let window = gtk::ApplicationWindow::new(&app);
    window.set_default_size(400, 200);
    window.set_child(Some(&scroller));
    window.present();
    crate::testpump::until(crate::testpump::Clock::Frame, "the preview maps", || {
        view.is_mapped()
    });

    let _ = framebuffer_of(&view, 400.0, 200.0);
    assert!(
        view.with_sprite_anim(&r, |_| ()).is_some(),
        "precondition: the driver must hold an entry while visible"
    );

    scroller.set_visible(false);
    crate::testpump::until(
        crate::testpump::Clock::Idle,
        "the pane to unmap its content",
        || !view.is_mapped(),
    );
    assert!(
        view.with_sprite_anim(&r, |_| ()).is_none(),
        "a hidden pane must release the sprite driver's entry"
    );

    scroller.set_visible(true);
    crate::testpump::until(crate::testpump::Clock::Idle, "the pane to remap", || {
        view.is_mapped()
    });
    let _ = framebuffer_of(&view, 400.0, 200.0);
    assert!(
        view.with_sprite_anim(&r, |_| ()).is_some(),
        "returning to a visible pane must rebuild the sprite driver's entry"
    );

    window.destroy();
    crate::sprite::clear_cache();
}

fn add_bare_play_animations_action(app: &gtk::Application, initial: bool) {
    let action = gtk::gio::SimpleAction::new_stateful(
        crate::animation::policy::ACTION_NAME,
        None,
        &initial.to_variant(),
    );
    action.connect_change_state(|act, value| {
        let Some(value) = value else { return };
        act.set_state(value);
    });
    app.add_action(&action);
}

/// TDD 27.5: switching View ▸ Play Animations off freezes the banded heading's sprite
/// on its current frame, with no tick callback left running; nothing else about the
/// document changes (the heading stays exactly where it is).
///
/// Built on a REAL themed heading, like the "plays" test above, rather than calling
/// `frame_for` directly — `animation::sprites::gtk_tests`'s own module doc comment
/// explains why a bare, undecorated view is the wrong fixture for this: the
/// policy-watch subscription's `queue_draw()` forces a real repaint on toggle, and only
/// a view whose `has_anything_to_draw()` is genuinely `true` keeps that repaint from
/// wiping the driver state out from under the very call meant to observe it.
#[gtktest::test]
fn play_animations_off_freezes_the_banded_heading_sprite() {
    let (_dir, path) = animated_sprite_file();
    let _theme = activate_banded_theme(&path);

    let view = CodePreviewView::new();
    view.buffer().set_text("A banded heading\n");
    view.set_heading_spans(vec![crate::renderer::HeadingSpan {
        span: crate::span::BufferSpan::new(0, 16),
        level_index: 0,
    }]);
    let app = crate::window::testkit::test_app_suffixed("animsprite.toggle");
    add_bare_play_animations_action(&app, true);
    let window = present_for_paint_sized(&view, 400, 200);
    window.set_application(Some(&app));
    let r = sprite_ref(&path);

    // The sprite's OWN current-frame bytes, read through the driver directly —
    // sidesteps the whole framebuffer (and, with it, the real preview's blinking
    // insertion caret, which this bare fixture leaves at the `GtkTextView` default and
    // which would otherwise change the framebuffer on its own timer for a reason that
    // has nothing to do with the sprite).
    let sprite_bytes = || {
        let _ = framebuffer_of(&view, 400.0, 200.0);
        view.with_sprite_anim(&r, |anim| {
            let tex = anim
                .current_texture()
                .expect("a playing sprite has a frame");
            let stride = tex.width() as usize * 4;
            let mut buf = vec![0u8; stride * tex.height() as usize];
            tex.download(&mut buf, stride);
            buf
        })
        .expect("the banded heading is on screen, so the driver must hold an entry")
    };

    let frame0 = sprite_bytes();
    assert!(
        crate::testpump::until_or_for(
            crate::testpump::Clock::Frame,
            std::time::Duration::from_secs(10),
            || sprite_bytes() != frame0
        ),
        "precondition: it must actually advance before it can be shown to freeze"
    );

    app.change_action_state(crate::animation::policy::ACTION_NAME, &false.to_variant());
    // The toggle's own `policy::watch` callback forces one repaint
    // (`SpriteAnim::ensure_bootstrapped`'s subscription calls `view.queue_draw()`),
    // which is what lets a single subsequent read observe the freeze taking effect
    // rather than racing it.
    crate::testpump::until(
        crate::testpump::Clock::Frame,
        "the pause to stop the tick",
        || {
            !view
                .with_sprite_anim(&r, |anim| anim.is_ticking())
                .unwrap_or(true)
        },
    );
    let frozen = sprite_bytes();

    // Pump real wall-clock time with the tick removed: the frame must not change on
    // its own.
    std::thread::sleep(std::time::Duration::from_millis(300));
    gtk::glib::MainContext::default().iteration(false);
    let still_frozen = sprite_bytes();
    window.destroy();
    assert_eq!(
        still_frozen, frozen,
        "paused must stay on the current frame, not keep advancing"
    );
    crate::sprite::clear_cache();
}
