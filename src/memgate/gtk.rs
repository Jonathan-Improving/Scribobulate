//! GTK driver for the per-render growth-slope and finalization halves.
//!
//! Compiled only under `--features memory-gates`, so the integration step never
//! runs it. Bodies go through `#[gtktest::test]` so they register with both
//! harnesses; the pipeline step invokes `--test gtk_suite memgate`.

use crate::links::ImageResolution;
use crate::memgate::footprint::{current, SAMPLE_COUNT, TOLERANCE_BYTES, WARMUP};
use crate::memgate::slope::assert_flat;
use crate::renderer::start::{load_texture, LoadedImage};
use crate::testsymlink::skipped;
use gtk::gdk::prelude::TextureExt;
use gtk::glib::object::ObjectExt;
use gtk::prelude::{GtkWindowExt, NativeExt, WidgetExt};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn load_local_cached(path: &Path) -> Option<LoadedImage> {
    load_texture(&ImageResolution::Local(path.to_path_buf()), 1.0)
}

/// The renderer GSK actually chose, from the realized native — not a
/// `CairoRenderer` we constructed, and not `$GSK_RENDERER`.
///
/// `.cargo/config.toml`'s plain `GSK_RENDERER = "cairo"` only applies when the
/// variable is unset; an ambient `GSK_RENDERER=gl` wins. The two seats then
/// fail in opposite directions, measured: at the 4.6 floor GL never releases
/// a texture (6.7 false red), and at 4.22.4 GL does finalize (6.7 false green,
/// having measured the wrong renderer). Reading the env would repeat the same
/// mistake. `None` is an unrealized native: refuse, do not skip.
fn realized_gsk_renderer() -> Option<gtk::gsk::Renderer> {
    let win = gtk::Window::new();
    win.set_default_size(8, 8);
    win.present();
    while gtk::glib::MainContext::default().iteration(false) {}
    let renderer = win.native().and_then(|n| n.renderer());
    win.close();
    while gtk::glib::MainContext::default().iteration(false) {}
    renderer
}

#[gtktest::test]
fn cairo_renderer_is_the_measured_arm() {
    match realized_gsk_renderer() {
        Some(r) if r.is::<gtk::gsk::CairoRenderer>() => {}
        other => panic!(
            "TDD 6.7 measured nothing, wrong arm: GSK renderer is {}, not \
             GskCairoRenderer. Finalization is only sound under Cairo (GL at \
             the 4.6 floor never releases the texture). Unset GSK_RENDERER — \
             `.cargo/config.toml` pins cairo only when the variable is absent \
             from the environment",
            other
                .as_ref()
                .map(|r| r.type_().name().to_string())
                .unwrap_or_else(|| "None (native unrealized)".into())
        ),
    }
}

fn sample_loads(path: &Path, n: usize) -> Option<Vec<u64>> {
    crate::imagecache::reset_for_test();
    let mut samples = Vec::with_capacity(n);
    for _ in 0..n {
        // Deliberately the cached path: 6.6 is about re-renders of an open
        // document, which must reuse the decode (6.8). Mutation-tested: inserting
        // `imagecache::reset_for_test()` here reddens 6.6 on the slope assertion
        // (~5 MB second-half delta), not on an earlier precondition.
        let loaded = load_local_cached(path)?;
        let fp = current()?;
        samples.push(fp);
        drop(loaded);
    }
    Some(samples)
}

#[gtktest::test]
fn growth_slope_animated_webp_ttd_6_6() {
    let path = fixture("anim.webp");
    match sample_loads(&path, SAMPLE_COUNT) {
        None => skipped(
            "TDD 6.6",
            "this host has no decoder for the animated WebP fixture (gvsbuild \
             ships SVG only; a missing webp-pixbuf-loader is the same skip)",
        ),
        Some(samples) => {
            if let Err(err) = assert_flat(&samples, WARMUP, TOLERANCE_BYTES) {
                panic!("TDD 6.6 animated WebP: {err}");
            }
        }
    }
}

#[gtktest::test]
fn growth_slope_png_is_flat_ttd_6_6() {
    // Negative control: a static PNG must not climb. If this fails, the
    // instrument is measuring warm-up or some other render-path leak, not the
    // animated-WebP loader branch.
    let path = fixture("wide.png");
    let samples = sample_loads(&path, SAMPLE_COUNT)
        .expect("PNG decode is native; a None here is a broken fixture, not a skip");
    assert_flat(&samples, WARMUP, TOLERANCE_BYTES)
        .unwrap_or_else(|err| panic!("TDD 6.6 PNG control: {err}"));
}

#[gtktest::test]
fn decoded_texture_finalizes_ttd_6_7() {
    let path = fixture("wide.png");
    crate::imagecache::reset_for_test();
    let loaded = load_local_cached(&path).expect("PNG decode");
    let weak = loaded.texture.downgrade();
    drop(loaded);
    // The cache is an application reference. A test that asserted finalization
    // while the cache still held the texture would fail on healthy code and
    // train us to delete 6.7.
    assert!(
        weak.upgrade().is_some(),
        "TDD 6.7 positive control: the cache must keep the texture alive"
    );
    crate::imagecache::reset_for_test();
    assert!(
        weak.upgrade().is_none(),
        "TDD 6.7: the GdkTexture must finalize once every application \
         reference is dropped, including the cache, with no main-loop pump \
         (Cairo renderer is pinned)"
    );
}

#[gtktest::test]
fn local_cache_reuses_decode_ttd_6_8() {
    let path = fixture("anim.webp");
    crate::imagecache::reset_for_test();
    let first = match load_local_cached(&path) {
        Some(img) => img,
        None => {
            skipped(
                "TDD 6.8",
                "this host has no decoder for the animated WebP fixture",
            );
            return;
        }
    };
    let w = first.texture.width();
    let h = first.texture.height();
    drop(first);
    let before = current().expect("footprint");
    let second = load_local_cached(&path).expect("cached decode");
    let after = current().expect("footprint");
    assert_eq!(second.texture.width(), w);
    assert_eq!(second.texture.height(), h);
    let grew = after.saturating_sub(before);
    assert!(
        grew <= TOLERANCE_BYTES,
        "TDD 6.8: second load of an unchanged local file grew footprint by {grew} bytes"
    );
}

#[gtktest::test]
fn local_cache_misses_when_mtime_changes_ttd_6_8() {
    // Two PNGs of different widths so the overwrite is visible as a dimension
    // change. A `.webp` temp overwritten with PNG bytes would pick the WebP
    // loader from the extension and fail to decode.
    let dir = std::env::temp_dir();
    let tmp = dir.join(format!(
        "scribobulate-memgate-6_8-{}.png",
        std::process::id()
    ));
    std::fs::copy(fixture("wide.png"), &tmp).expect("copy wide png");
    crate::imagecache::reset_for_test();
    let first = load_local_cached(&tmp).expect("first decode");
    let first_w = first.texture.width();
    drop(first);
    std::fs::copy(fixture("logo.png"), &tmp).expect("overwrite with smaller png");
    let second = load_local_cached(&tmp).expect("decode after mtime change");
    let _ = std::fs::remove_file(&tmp);
    assert_ne!(
        second.texture.width(),
        first_w,
        "TDD 6.8: replacing the file on disk must produce a new decode, not the cached one"
    );
}

#[gtktest::test]
fn local_cache_makes_svg_rerender_free_ttd_6_8() {
    // A large SVG used to re-decode on every render (~239 ms on the reference
    // host) because local images had no cache. Asserted as a cache hit
    // (footprint, not wall-clock — a timing assertion flakes on a loaded host).
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("sdd/system-overview.svg");
    crate::imagecache::reset_for_test();
    let first = load_local_cached(&path).expect("SVG decode");
    let w = first.texture.width();
    drop(first);
    let before = current().expect("footprint");
    let second = load_local_cached(&path).expect("cached SVG");
    let after = current().expect("footprint");
    assert_eq!(second.texture.width(), w);
    let grew = after.saturating_sub(before);
    assert!(
        grew <= TOLERANCE_BYTES,
        "TDD 6.8: second load of an unchanged SVG grew footprint by {grew} bytes"
    );
}
