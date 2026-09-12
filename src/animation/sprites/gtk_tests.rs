//! `SpriteAnim`'s own GTK-object test — the driver called directly through
//! [`frame_for`] rather than through a themed `bandpaint::paint_band`.
//!
//! **Only the STILL-sprite case belongs here.** An animated case needs the view to
//! ALSO carry real decoration content (`set_heading_spans` on a themed view): once a
//! tick installs, `SpriteAnim::ensure_bootstrapped`'s policy-watch subscription calls
//! `view.queue_draw()` on any Play-Animations/reduce-animations change, which forces a
//! REAL `snapshot_layer` pass on this view — and a bare view with nothing else drawn
//! answers `has_anything_to_draw() == false`, so that forced pass's own
//! `reset_sprite_anim_seen`/`drop_unseen_sprite_anims` bracketing (unconditional,
//! `codeview::mod::snapshot_layer`) drops an entry this file created only by calling
//! `frame_for` directly, outside any paint `bandpaint::paint_band` would itself have
//! driven. That is a **test-methodology artifact**, not a production bug — a real
//! document with the sprite decoration also has `has_anything_to_draw() == true` and
//! `bandpaint::paint_band` runs on every such forced repaint too, so the two calls
//! cooperate instead of racing. `codeview::animsprite_tests` is where the animated
//! cases live instead, each through a themed heading exactly as production reaches
//! this module.

use super::*;
use crate::codeview::CodePreviewView;
use crate::sprite::SpriteRef;
use gtk::prelude::TextureExt;

fn test_app(suffix: &str) -> gtk::Application {
    crate::window::testkit::test_app_suffixed(&format!("spriteanim.{suffix}"))
}

/// The downloaded pixel bytes of `tex` — `GdkTexture` has no public identity
/// accessor (`animation::paintable::gtk_tests`'s own `painted_bytes` doc comment), so
/// every comparison below is by CONTENT, not by object identity.
fn bytes_of(tex: &gtk::gdk::Texture) -> Vec<u8> {
    let stride = tex.width() as usize * 4;
    let mut buf = vec![0u8; stride * tex.height() as usize];
    tex.download(&mut buf, stride);
    buf
}

/// A realized, mapped `CodePreviewView` in its own application window — real root,
/// real application, real frame clock, exactly what `SpriteAnim::ensure_bootstrapped`
/// needs to resolve `policy::current`.
fn realize(app: &gtk::Application) -> (CodePreviewView, gtk::ApplicationWindow) {
    let view = CodePreviewView::new();
    let window = gtk::ApplicationWindow::new(app);
    window.set_child(Some(&view));
    window.present();
    crate::testpump::until(crate::testpump::Clock::Idle, "the view to map", || {
        view.is_mapped()
    });
    (view, window)
}

fn still_sprite_ref() -> (SpriteRef, gtk::gdk::Texture) {
    let dir = tempfile::tempdir().expect("tempdir");
    let dir = Box::leak(Box::new(dir));
    let path = dir.path().join("chip.png");
    // The smallest fixture that actually decodes — see `sprite::tests::write_test_png`
    // for the same bytes; duplicated here rather than exposed cross-module for a
    // one-off fixture.
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8,
        0xFF, 0xFF, 0x3F, 0x00, 0x05, 0xFE, 0x02, 0xFE, 0xDC, 0xCC, 0x59, 0xE7, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];
    std::fs::write(&path, PNG).expect("write fixture");
    let resolved =
        SpriteRef::File(crate::sprite::resolve(dir.path(), "chip.png").expect("resolves"));
    let tex = crate::sprite::texture(&resolved).expect("decodes");
    (resolved, tex)
}

/// A still sprite must cost NOTHING new: `frame_for` hands back the caller's own
/// `natural` texture unchanged, and no `SpriteAnim` is ever created for it — verified
/// both by content (byte-for-byte) and by absence from the view's own table
/// (`with_sprite_anim` finds nothing to run against). No incidental repaint can ever
/// disturb this case either way: nothing is ever inserted to begin with.
#[gtktest::test]
fn a_still_sprite_is_returned_unchanged_and_creates_no_driver_state() {
    crate::sprite::clear_cache();
    let app = test_app("still");
    let (view, _window) = realize(&app);
    let (r, natural) = still_sprite_ref();

    let got = frame_for(&view, &r, Some(natural.clone())).expect("a still sprite has a texture");
    assert_eq!(
        bytes_of(&got),
        bytes_of(&natural),
        "a still sprite's texture must come back byte-for-byte identical"
    );
    assert!(
        view.with_sprite_anim(&r, |_| ()).is_none(),
        "a still sprite must never gain an entry in the animation driver's table"
    );
    crate::sprite::clear_cache();
}
