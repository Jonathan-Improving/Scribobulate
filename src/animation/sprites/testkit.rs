//! Test fixtures shared by every animated-sprite host's tests (TDD 27.9) — the driver's
//! own, the preview's paint plan, and each widget that hosts a sprite table.

use crate::sprite::SpriteRef;
use gtk::prelude::*;

/// `tests/fixtures/anim.webp` (480×270, animated; its frames differ only in rows
/// 96–236) written to a temporary directory, and a file-backed reference to it. Keep
/// the directory alive for as long as the reference is used.
pub(crate) fn animated_fixture() -> (tempfile::TempDir, SpriteRef) {
    fixture(
        "anim.webp",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/anim.webp"
        )),
    )
}

/// A 1×1 white PNG — the smallest still sprite that actually decodes.
pub(crate) fn still_fixture() -> (tempfile::TempDir, SpriteRef) {
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8,
        0xFF, 0xFF, 0x3F, 0x00, 0x05, 0xFE, 0x02, 0xFE, 0xDC, 0xCC, 0x59, 0xE7, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];
    fixture("still.png", PNG)
}

fn fixture(name: &str, bytes: &[u8]) -> (tempfile::TempDir, SpriteRef) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join(name), bytes).expect("write fixture");
    let r = SpriteRef::File(crate::sprite::resolve(dir.path(), name).expect("resolves"));
    (dir, r)
}

/// The pixels `paint` draws, rendered through the Cairo renderer. Handed a widget's
/// OWN `snapshot` vfunc, this reads that widget's node rather than a parent's composite.
pub(crate) fn rendered(paint: impl FnOnce(&gtk::Snapshot)) -> Vec<u8> {
    let snapshot = gtk::Snapshot::new();
    paint(&snapshot);
    let Some(node) = snapshot.to_node() else {
        return Vec::new();
    };
    let renderer = gtk::gsk::CairoRenderer::new();
    renderer
        .realize(None::<&gtk::gdk::Surface>)
        .expect("realize");
    let texture = renderer.render_texture(&node, None);
    renderer.unrealize();
    let (w, h) = (texture.width() as usize, texture.height() as usize);
    let mut bytes = vec![0u8; w * h * 4];
    texture.download(&mut bytes, w * 4);
    bytes
}

/// `tex`'s downloaded pixels — compared by CONTENT, since `GdkTexture` has no public
/// identity accessor.
pub(crate) fn texture_bytes(tex: &gtk::gdk::Texture) -> Vec<u8> {
    let stride = tex.width() as usize * 4;
    let mut buf = vec![0u8; stride * tex.height() as usize];
    tex.download(&mut buf, stride);
    buf
}

/// The frame `table` is currently playing for `r`, as bytes — `None` when it plays
/// nothing for `r`. For a slot that shows only part of the sprite (a tile clipped to a
/// short band), where the painted pixels may not include the rows the frames change in.
pub(crate) fn current_frame(table: &super::SpriteTable, r: &SpriteRef) -> Option<Vec<u8>> {
    table.with_anim(r, |anim| texture_bytes(&anim.current_texture()))
}

/// Whether what `paint` draws changes within ten seconds of the real frame clock —
/// never a sleep. A paint that draws nothing never counts as playing.
pub(crate) fn plays(mut paint: impl FnMut(&gtk::Snapshot)) -> bool {
    let first = rendered(&mut paint);
    assert!(!first.is_empty(), "the paint drew nothing at all");
    crate::testpump::until_or_for(
        crate::testpump::Clock::Frame,
        std::time::Duration::from_secs(10),
        || rendered(&mut paint) != first,
    )
}
