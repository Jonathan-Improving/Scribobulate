//! The sprite driver's own GTK-object tests — a [`SpriteTable`] driven directly on a
//! plain host widget, rather than through a themed paint.
//!
//! A plain widget is the right fixture here, where it was not for `CodePreviewView`:
//! nothing but these calls brackets this table's passes, so a repaint the driver asks
//! for (a new frame, a policy change) cannot prune an entry out from under the test.
//! `codeview::animsprite_tests` proves the wiring through real themed paints.

use super::*;
use crate::sprite::SpriteRef;
use gtk::prelude::TextureExt;

fn test_app(suffix: &str) -> gtk::Application {
    crate::window::testkit::test_app_suffixed(&format!("spriteanim.{suffix}"))
}

/// The downloaded pixel bytes of `tex` — compared by CONTENT, since `GdkTexture` has
/// no public identity accessor.
fn bytes_of(tex: &gtk::gdk::Texture) -> Vec<u8> {
    let stride = tex.width() as usize * 4;
    let mut buf = vec![0u8; stride * tex.height() as usize];
    tex.download(&mut buf, stride);
    buf
}

/// A mapped host widget in its own application window — real root, real application,
/// real frame clock, which is what the driver needs to resolve `policy::current` and
/// tick.
fn mapped_host(app: &gtk::Application) -> (gtk::Label, gtk::ApplicationWindow) {
    let host = gtk::Label::new(Some("host"));
    let window = gtk::ApplicationWindow::new(app);
    window.set_child(Some(&host));
    window.present();
    crate::testpump::until(crate::testpump::Clock::Idle, "the host to map", || {
        host.is_mapped()
    });
    (host, window)
}

/// A file-backed sprite reference over `bytes`. The directory is leaked for the test's
/// lifetime, since the reference is by path.
fn sprite_file(name: &str, bytes: &[u8]) -> SpriteRef {
    let dir = Box::leak(Box::new(tempfile::tempdir().expect("tempdir")));
    std::fs::write(dir.path().join(name), bytes).expect("write fixture");
    SpriteRef::File(crate::sprite::resolve(dir.path(), name).expect("resolves"))
}

fn still_sprite() -> SpriteRef {
    // The smallest fixture that actually decodes — see `sprite::tests::write_test_png`.
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8,
        0xFF, 0xFF, 0x3F, 0x00, 0x05, 0xFE, 0x02, 0xFE, 0xDC, 0xCC, 0x59, 0xE7, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];
    sprite_file("chip.png", PNG)
}

fn animated_sprite() -> SpriteRef {
    sprite_file(
        "anim.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/anim.webp"
        )),
    )
}

/// A still sprite costs NOTHING new: both `Frames` routes hand back exactly what
/// `sprite::texture`/`sprite::scaled` produce, and no driver entry is ever created.
#[gtktest::test]
fn a_still_sprite_is_returned_unchanged_and_creates_no_driver_state() {
    crate::sprite::clear_cache();
    let app = test_app("still");
    let (host, _window) = mapped_host(&app);
    let r = still_sprite();
    let table = SpriteTable::default();

    table.begin_pass();
    let frames = table.frames(host.upcast_ref());
    let natural = frames.natural(&r).expect("a still sprite has a texture");
    let scaled = frames.scaled(&r, 3, 2).expect("a still sprite resamples");
    table.end_pass();

    let still = crate::sprite::texture(&r).expect("decodes");
    assert_eq!(
        bytes_of(&natural),
        bytes_of(&still),
        "natural size is the still texture"
    );
    let still_scaled = crate::sprite::scaled(&r, 3, 2).expect("resamples");
    assert_eq!(
        bytes_of(&scaled),
        bytes_of(&still_scaled),
        "resample is the still resample"
    );
    assert!(
        table.with_anim(&r, |_| ()).is_none(),
        "a still sprite must never gain an entry in the animation driver's table"
    );
    crate::sprite::clear_cache();
}

/// TDD 27.9: a RESAMPLED animated sprite plays — the slot shape of a chip, a list
/// marker, a scene or a disclosure indicator, which draw a sprite into a box the layout
/// chose rather than tiling it. The resample is at the requested size, and its pixels
/// change under the real frame clock.
#[gtktest::test]
fn a_resampled_animated_sprite_plays_at_the_requested_size() {
    crate::sprite::clear_cache();
    let app = test_app("scaled");
    let (host, window) = mapped_host(&app);
    let r = animated_sprite();
    let table = SpriteTable::default();
    let (w, h) = (96, 54);

    let paint = || {
        table.begin_pass();
        let tex = table
            .frames(host.upcast_ref())
            .scaled(&r, w, h)
            .expect("the sprite resamples");
        table.end_pass();
        tex
    };
    let first = paint();
    assert_eq!((first.width(), first.height()), (w, h));
    assert!(
        table
            .with_anim(&r, |anim| anim.is_ticking())
            .unwrap_or(false),
        "a visible, policy-enabled animated sprite must be ticking"
    );
    let first = bytes_of(&first);
    let changed = crate::testpump::until_or_for(
        crate::testpump::Clock::Frame,
        std::time::Duration::from_secs(10),
        || bytes_of(&paint()) != first,
    );
    window.destroy();
    assert!(
        changed,
        "the resampled sprite never changed across 10s of wall clock"
    );
    crate::sprite::clear_cache();
}

/// TDD 27.3 / 27.9: a pass that no longer draws the sprite releases it — no decoder,
/// no tick — which is what a decoration scrolled out of its paint's viewport gate is.
#[gtktest::test]
fn a_pass_that_does_not_draw_the_sprite_drops_its_driver() {
    crate::sprite::clear_cache();
    let app = test_app("pass");
    let (host, window) = mapped_host(&app);
    let r = animated_sprite();
    let table = SpriteTable::default();

    table.begin_pass();
    let _ = table.frames(host.upcast_ref()).natural(&r);
    table.end_pass();
    assert!(
        table.with_anim(&r, |_| ()).is_some(),
        "precondition: drawn, so tracked"
    );

    table.begin_pass();
    table.end_pass();
    window.destroy();
    assert!(
        table.with_anim(&r, |_| ()).is_none(),
        "a pass that did not draw the sprite must drop its driver"
    );
    crate::sprite::clear_cache();
}

/// TDD 27.3: a host that is not on screen plays nothing, even when its paint asks —
/// the case of an anchored child that GTK still snapshots while scrolled away. It gets
/// the still frame, and no driver.
#[gtktest::test]
fn an_unmapped_host_plays_nothing_and_paints_the_still_frame() {
    crate::sprite::clear_cache();
    let r = animated_sprite();
    let host = gtk::Label::new(Some("never shown"));
    let table = SpriteTable::default();

    table.begin_pass();
    let got = table
        .frames(host.upcast_ref())
        .natural(&r)
        .expect("the still frame is still there");
    table.end_pass();

    let still = crate::sprite::texture(&r).expect("decodes");
    assert_eq!(bytes_of(&got), bytes_of(&still));
    assert!(
        table.with_anim(&r, |_| ()).is_none(),
        "an invisible host must not create a driver"
    );
    crate::sprite::clear_cache();
}
