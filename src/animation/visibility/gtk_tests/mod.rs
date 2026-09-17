//! The visibility decision's live verification suite, split out at POLICY's 500-line soft limit —
//! `mod.rs` keeps the decision core and its display-free tests, exactly like
//! `paintable`'s own `mod.rs`/`gtk_tests.rs` split.
//!
//! # Layout
//!
//! - `claim_*` — the four GTK 4.6 claims TDD 27.3's plan records, each verified
//!   directly before anything below relies on it (see `super`'s module doc for
//!   what each one settled and, for claim 2, corrected).
//! - `table_*` — one test per row of the plan's "not visible because…" table
//!   that this project can actually produce (minimized is folded into
//!   `claim_4`, which is where its runtime skip already lives — a second copy
//!   would test nothing new).
//! - `return_to_view_restarts_at_frame_zero` — TDD 27.3's other half.
//!
//! Every test drives a REAL `AnimatedPaintable` on a REAL widget tree — the
//! oracles are `AnimatedPaintable::tick_installed`/`decoder_active` (both built
//! for exactly this: neither `gtk_widget_has_tick_callback` nor a public
//! decoder-liveness accessor exists) plus painted pixels, never a private field
//! read directly.

use std::path::PathBuf;
use std::sync::Arc;

use gtk::prelude::*;

use crate::animation::paintable::AnimatedPaintable;
use crate::animation::policy::EnableAnimationsGuard;
use crate::animation::visibility;
use crate::codeview::CodePreviewView;

fn anim_bytes() -> Arc<[u8]> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/anim.webp");
    Arc::from(std::fs::read(&path).expect("fixture reads"))
}

fn test_app(suffix: &str) -> gtk::Application {
    crate::window::testkit::test_app_suffixed(&format!("visibility.{suffix}"))
}

fn painted_bytes(paintable: &gtk::gdk::Paintable) -> Vec<u8> {
    use gtk::prelude::Cast;
    let texture = paintable
        .current_image()
        .downcast::<gtk::gdk::Texture>()
        .expect("AnimatedPaintable::current_image always returns a Texture while it has one");
    let stride = texture.width() as usize * 4;
    let mut buf = vec![0u8; stride * texture.height() as usize];
    texture.download(&mut buf, stride);
    buf
}

/// A `CodePreviewView` with `lines_before` filler lines, then an animated
/// picture — wrapped in a `GtkOverlay`, exactly as `renderer::start::anchor_image`
/// always wraps one before anchoring it (see `super`'s module doc on why this
/// makes the "nested inside another widget" row of the plan's table the
/// ORDINARY case, not a special one) — then `lines_after` more filler lines.
fn build_scrollable_animation(
    lines_before: usize,
    lines_after: usize,
) -> (CodePreviewView, gtk::Picture, AnimatedPaintable) {
    let view = CodePreviewView::new();
    let buf = view.buffer();
    let before: String = (0..lines_before).map(|n| format!("line {n}\n")).collect();
    buf.set_text(&before);

    let mut iter = buf.end_iter();
    let anchor = buf.create_child_anchor(&mut iter);

    let pic = gtk::Picture::new();
    let animated = AnimatedPaintable::new(pic.upcast_ref(), anim_bytes()).expect("fixture decodes");
    pic.set_paintable(Some(&animated));
    // GTK4Rs/AP-58: an anchored, can-shrink `GtkPicture` with no size request
    // measures its for-width-0 height as 0 and blanks — the same seed every
    // production anchor gets.
    pic.set_size_request(64, 64);
    pic.set_can_shrink(true);
    let overlay = gtk::Overlay::new();
    overlay.set_halign(gtk::Align::Start);
    overlay.set_child(Some(&pic));
    view.add_child_at_anchor(&overlay, &anchor);

    let mut end = buf.end_iter();
    let after: String = (0..lines_after).map(|n| format!("\nline {n}\n")).collect();
    buf.insert(&mut end, &after);

    (view, pic, animated)
}

/// Realize `view` inside a `GtkScrolledWindow` inside a window sized
/// `win_w`x`win_h`, and pump until it is mapped with real (non-draft)
/// adjustment bounds AND `animated` has actually started playing.
///
/// The second half is not redundant: `try_bootstrap`'s visibility seed can
/// run — and answer `false` — before the toplevel has ever been mapped (it
/// fires as soon as `host.root()` resolves, which needs only PARENTING, not
/// mapping or allocation), so autoplay starts from the LATER `map`/adjustment
/// signals `visibility::watch` installs, not synchronously with `present()`.
/// Asserting `tick_installed()` immediately after this function returned
/// (before this fix) was flaky in exactly the GTK4Rs/AP-13/GTK4Rs/AP-78 shape
/// this skill warns about: it happened to pass when GTK's own idle-driven
/// work settled inside the FIRST pump loop, and failed when it didn't.
fn realize_scrolled(
    app: &gtk::Application,
    view: &CodePreviewView,
    animated: &AnimatedPaintable,
    win_w: i32,
    win_h: i32,
) -> (gtk::ApplicationWindow, gtk::ScrolledWindow) {
    let sw = gtk::ScrolledWindow::new();
    sw.set_child(Some(view));
    let window = gtk::ApplicationWindow::new(app);
    window.set_default_size(win_w, win_h);
    window.set_child(Some(&sw));
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
    wait_until_playing(animated);
    (window, sw)
}

/// Pump until `animated` has a tick callback installed — see
/// [`realize_scrolled`]'s doc comment on why this is a real wait, not a
/// formality.
fn wait_until_playing(animated: &AnimatedPaintable) {
    crate::testpump::until(
        crate::testpump::Clock::Idle,
        "playback to start once the picture is mapped and allocated",
        {
            let animated = animated.clone();
            move || animated.tick_installed()
        },
    );
}

fn scroll_to(sw: &gtk::ScrolledWindow, value: f64) {
    crate::saferizer::scrollpos::jump(&sw.vadjustment(), value);
}

// ---------------------------------------------------------------------------
// Claim verification (dark pattern, POLICY: "verify by test before relying").
// ---------------------------------------------------------------------------

/// **Claim 1**: an anchored `GtkTextView` child scrolled out of the preview's
/// viewport stays MAPPED — only its allocation moves — so `is_mapped()` is the
/// WRONG predicate for "scrolled out of view". Confirmed: `geometry_visible`
/// (via `compute_bounds`, WIDGET space) is what actually changes.
#[gtktest::test]
fn claim_1_scrolled_out_stays_mapped_and_geometry_is_the_right_predicate() {
    let _enable = EnableAnimationsGuard::set(true);
    let app = test_app("claim1");
    let (view, pic, animated) = build_scrollable_animation(2, 400);
    let (window, sw) = realize_scrolled(&app, &view, &animated, 300, 150);

    assert!(
        visibility::geometry_visible(pic.upcast_ref(), sw.upcast_ref()),
        "precondition: the picture starts within the small viewport"
    );

    // `GtkTextView` validates line heights LAZILY (GTK4Rs/AP-13 family), so
    // `vadjustment().upper()` right after mapping can be a draft that grows
    // over several idle turns — a single scroll computed against today's
    // `upper()` can undershoot the true end of the document. Re-issue the
    // scroll to the CURRENT bottom on every poll so it converges once
    // validation catches up, rather than trusting one snapshot of `upper()`.
    crate::testpump::until(
        crate::testpump::Clock::Idle,
        "the scroll to move the picture out of the viewport",
        {
            let sw = sw.clone();
            let pic = pic.clone();
            move || {
                let vadj = sw.vadjustment();
                scroll_to(&sw, vadj.upper() - vadj.page_size());
                !visibility::geometry_visible(pic.upcast_ref(), sw.upcast_ref())
            }
        },
    );

    assert!(
        pic.is_mapped(),
        "claim 1: an anchored child scrolled out of the viewport STAYS MAPPED — \
         is_mapped() must not have gone false on its own"
    );
    assert!(
        pic.compute_bounds(&sw).is_some(),
        "claim 1: it still has a real allocation (\"still snapshotted\") — only \
         the allocation's POSITION moved, it was not parked at a degenerate size"
    );
    assert!(
        !visibility::geometry_visible(pic.upcast_ref(), sw.upcast_ref()),
        "claim 1: geometry_visible is the predicate that correctly says NO here, \
         precisely because is_mapped() cannot"
    );
    window.destroy();
}

/// **Claim 2, corrected**: the plan's table describes GTK's GENERIC collapse
/// mechanism (an invisible tag parks the child at `(-w, -h)` without unmapping
/// it, `GTK4Rs/AP-166`). **This project does not use that mechanism.**
/// `renderer::start::inside_collapsed_body` means a collapsed body's images are
/// never anchored, and `preview::splice`'s collapse path deletes the
/// summary+body range outright (`src/preview/splice.rs`,
/// `buf.delete(&mut del_start, &mut del_end)`) rather than tagging it
/// invisible. `GtkTextBuffer::delete` unparents every anchored child in the
/// deleted range (`GTK4Rs/AP-320`) — this test proves that GENERAL mechanism
/// directly, on the same buffer/anchor shape `preview::splice` uses, without
/// standing up the full Markdown → render → splice pipeline for one fact
/// already provable at the buffer layer. So this row of the table does not
/// describe a live collapse in THIS renderer: `AnimatedPaintable::dispose`
/// (already exercised by `paintable::gtk_tests`'s own
/// `dropping_the_picture_releases_the_paintable_and_its_decoder`) is what
/// cleans this case up, with no help from `animation::visibility` at all.
#[gtktest::test]
fn claim_2_deleting_a_details_bodys_buffer_range_drops_its_anchored_picture_rather_than_parking_it()
{
    let _enable = EnableAnimationsGuard::set(true);
    let app = test_app("claim2");
    let (view, pic, animated) = build_scrollable_animation(1, 1);
    let (window, _sw) = {
        let sw = gtk::ScrolledWindow::new();
        sw.set_child(Some(&view));
        let window = gtk::ApplicationWindow::new(&app);
        window.set_default_size(300, 300);
        window.set_child(Some(&sw));
        window.present();
        crate::testpump::until(crate::testpump::Clock::Idle, "the window to map", {
            let view = view.clone();
            move || view.is_mapped()
        });
        (window, sw)
    };
    wait_until_playing(&animated);
    assert!(
        animated.tick_installed(),
        "sanity: playing before the delete"
    );

    let weak = animated.downgrade();
    drop(animated);
    assert!(
        weak.upgrade().is_some(),
        "the picture still holds it — dropping our own local must not free it yet"
    );

    // Delete exactly the anchor's own placeholder character — the shape
    // `preview::splice`'s collapse path takes over a whole summary+body
    // region, reduced to its essential mechanism. Found the same way
    // `preview::splice::install::anchors_in` finds one: `forward_find_char`
    // ADVANCES BEFORE it tests, so the start iterator's own character is
    // checked separately first (`build_scrollable_animation`'s one filler
    // line puts the anchor a few characters in here, but the general finder
    // is what the production code actually relies on).
    let buf = view.buffer();
    let mut iter = buf.start_iter();
    if iter.child_anchor().is_none() {
        assert!(
            iter.forward_find_char(|c| c == '\u{FFFC}', None),
            "the fixture's own anchor placeholder must be findable"
        );
    }
    assert!(iter.child_anchor().is_some(), "landed on the anchor");
    let mut end = iter;
    assert!(end.forward_char(), "advance past the U+FFFC placeholder");
    buf.delete(&mut iter, &mut end);

    assert!(
        pic.parent().is_none(),
        "GTK4Rs/AP-320: delete() must unparent the anchored child, not park it"
    );

    // `pic` is still THIS TEST's own local — unparenting removed it from the
    // widget tree, not from this variable. Drop it (as
    // `paintable::gtk_tests::dropping_the_picture_releases_the_paintable_and_its_decoder`
    // does with its own local) so nothing but the weak ref remains.
    drop(pic);
    assert!(
        weak.upgrade().is_none(),
        "with the picture unparented and dropped, and no other reference, dispose() \
         must have released the paintable and its decoder — the collapse case needs \
         no help from animation::visibility"
    );
    window.destroy();
}

/// **Claim 3**: a background tab (`child-visible` false via `GtkStack`) and a
/// hidden pane (`set_visible(false)`) both genuinely UNMAP — `is_mapped()` is
/// the RIGHT predicate here, unlike claims 1/2.
#[gtktest::test]
fn claim_3_a_background_tab_and_a_hidden_pane_do_unmap() {
    let _enable = EnableAnimationsGuard::set(true);
    let app = test_app("claim3");
    let (view, pic, _animated) = build_scrollable_animation(1, 1);
    let sw = gtk::ScrolledWindow::new();
    sw.set_child(Some(&view));

    let other_page = gtk::Label::new(Some("other tab"));
    let stack = gtk::Stack::new();
    stack.add_titled(&sw, Some("anim"), "Animation");
    stack.add_titled(&other_page, Some("other"), "Other");
    stack.set_visible_child_name("anim");

    let window = gtk::ApplicationWindow::new(&app);
    window.set_default_size(300, 300);
    window.set_child(Some(&stack));
    window.present();
    crate::testpump::until(crate::testpump::Clock::Idle, "the window to map", {
        let pic = pic.clone();
        move || pic.is_mapped()
    });
    assert!(
        pic.is_mapped(),
        "precondition: mapped on the foreground tab"
    );

    stack.set_visible_child_name("other");
    crate::testpump::until(
        crate::testpump::Clock::Idle,
        "the background tab to unmap its content",
        {
            let pic = pic.clone();
            move || !pic.is_mapped()
        },
    );
    assert!(
        !pic.is_mapped(),
        "claim 3: a background tab's page must unmap its content"
    );

    stack.set_visible_child_name("anim");
    crate::testpump::until(crate::testpump::Clock::Idle, "the tab to remap", {
        let pic = pic.clone();
        move || pic.is_mapped()
    });

    sw.set_visible(false);
    crate::testpump::until(
        crate::testpump::Clock::Idle,
        "a hidden pane to unmap its content",
        {
            let pic = pic.clone();
            move || !pic.is_mapped()
        },
    );
    assert!(
        !pic.is_mapped(),
        "claim 3: set_visible(false) on the pane must unmap its content"
    );
    window.destroy();
}

/// **Claim 4**: a minimized toplevel is observable as
/// `GdkToplevelState::MINIMIZED` via `notify::state`, and the frame clock does
/// NOT necessarily freeze on X11. The `#[gtktest::test]` bodies this pipeline
/// runs under (`scripts/gtk-run.sh`) launch a bare `Xvfb` with no window
/// manager (see its own header on `xvfb-run`/`dbus-run-session` nesting) — and
/// iconifying a window is an ICCCM request a WM must honour; with none
/// running, `gtk_window_minimize()` has nothing to grant it. So the MINIMIZED
/// half of this claim is a genuine runtime skip here, reported as one rather
/// than either silently passing on a state that never arrived or hanging on a
/// pump waiting for it.
#[gtktest::test]
fn claim_4_and_table_minimized_window() {
    let _enable = EnableAnimationsGuard::set(true);
    let app = test_app("claim4");
    let (view, _pic, animated) = build_scrollable_animation(1, 1);
    let (window, _sw) = realize_scrolled(&app, &view, &animated, 300, 300);
    assert!(animated.tick_installed(), "sanity: playing before minimize");

    window.minimize();
    let became_minimized = crate::testpump::until_or_for(
        crate::testpump::Clock::Idle,
        std::time::Duration::from_secs(3),
        {
            let window = window.clone();
            move || {
                window
                    .surface()
                    .and_then(|s| s.downcast::<gtk::gdk::Toplevel>().ok())
                    .is_some_and(|t| t.state().contains(gtk::gdk::ToplevelState::MINIMIZED))
            }
        },
    );

    if !became_minimized {
        crate::testsymlink::skipped(
            "TDD 27.3 minimized window",
            "no window manager is running under this pipeline's bare Xvfb, so \
             gtk_window_minimize()'s ICCCM iconify request has nothing to grant it \
             and the toplevel never reaches GdkToplevelState::MINIMIZED — the \
             notify::state wiring itself is exercised functionally by the other \
             table_* tests below (they all fire through the SAME visibility::watch \
             machinery), but the minimize TRANSITION specifically needs a real \
             compositor",
        );
        window.destroy();
        return;
    }

    // Reached (a WM IS present in this run) — prove both halves for real:
    // the toplevel-state predicate answers correctly, AND — the second half
    // of claim 4 — the frame clock is NOT relied on to have frozen; the
    // paintable's own `notify::state` wiring is what stopped it.
    crate::testpump::until(
        crate::testpump::Clock::Idle,
        "the visibility watch to react to the minimize",
        {
            let animated = animated.clone();
            move || !animated.tick_installed()
        },
    );
    assert!(
        !animated.decoder_active(),
        "a minimized window is not visible — the decoder must be released too"
    );
    window.destroy();
}

mod table;
