//! GTK driver for the per-render growth and finalization halves.
//!
//! Compiled only under `--features memory-gates`, so the integration step never
//! runs it. Bodies go through `#[gtktest::test]` so they register with both
//! harnesses; the pipeline step invokes `--test gtk_suite memgate`.

use crate::links::ImageResolution;
use crate::memgate::footprint::{assert_bounded, current, SAMPLE_COUNT, WARMUP};
use crate::renderer::start::{load_texture, LoadedImage};
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

/// Which decode path a sampling run exercises.
#[derive(Clone, Copy)]
enum CachePath {
    /// Re-renders of an open document: every load after the first is a cache hit
    /// (6.6, 6.8).
    Warm,
    /// The cache emptied before every load, as eviction or a changed file does — so
    /// every load is a fresh decode (6.9).
    Cold,
}

fn sample_loads(path: &Path, n: usize, cache: CachePath) -> Option<Vec<u64>> {
    // The footprint instrument is process-wide; hold it for the whole series, baseline
    // included. See `footprint::measuring`.
    let _measuring = crate::memgate::footprint::measuring();
    crate::imagecache::reset_for_test();
    let mut samples = Vec::with_capacity(n);
    for _ in 0..n {
        // `Warm` is deliberately the cached path: 6.6 is about re-renders of an open
        // document, which must reuse the decode (6.8). Mutation-tested: inserting
        // `imagecache::reset_for_test()` here reddens 6.6 on the growth assertion
        // (a step on nearly every sample), not on an earlier precondition. `Cold` is
        // that mutation made the subject: 6.9.
        if let CachePath::Cold = cache {
            crate::imagecache::reset_for_test();
        }
        let loaded = load_local_cached(path)?;
        let fp = current()?;
        samples.push(fp);
        drop(loaded);
    }
    Some(samples)
}

#[gtktest::test]
fn growth_animated_webp_ttd_6_6() {
    // Every host decodes this now — `richimg` is pure Rust, not a host gdk-pixbuf
    // loader, so there is no longer a decoder-absent skip arm here; a skip
    // would now be dead code hiding a failure.
    let path = fixture("anim.webp");
    let samples = sample_loads(&path, SAMPLE_COUNT, CachePath::Warm)
        .expect("richimg decodes anim.webp on every host; a None here is a broken fixture");
    assert_bounded("6.6 animated WebP", WARMUP, &samples);
}

#[gtktest::test]
fn uncached_decode_animated_webp_ttd_6_9() {
    // Every load is a fresh decode — the path an evicted or changed file takes,
    // which 6.6 cannot see because it measures cache hits. Every host decodes this
    // now (see 6.6's comment above) — no decoder-absent skip arm.
    let path = fixture("anim.webp");
    let samples = sample_loads(&path, SAMPLE_COUNT, CachePath::Cold)
        .expect("richimg decodes anim.webp on every host; a None here is a broken fixture");
    assert_bounded("6.9 uncached animated WebP", WARMUP, &samples);
}

#[gtktest::test]
fn uncached_decode_png_is_flat_ttd_6_9() {
    // Negative control for 6.9: a fresh PNG decode every iteration must not climb.
    // Without it, a red 6.9 could be the cache reset's own churn rather than the
    // WebP decode.
    let path = fixture("wide.png");
    let samples = sample_loads(&path, SAMPLE_COUNT, CachePath::Cold)
        .expect("PNG decode is native; a None here is a broken fixture, not a skip");
    assert_bounded("6.9 PNG control", WARMUP, &samples);
}

#[gtktest::test]
fn growth_png_is_flat_ttd_6_6() {
    // Negative control: a static PNG must not climb. If this fails, the
    // instrument is measuring warm-up or some other render-path leak, not the
    // animated-WebP loader branch.
    let path = fixture("wide.png");
    let samples = sample_loads(&path, SAMPLE_COUNT, CachePath::Warm)
        .expect("PNG decode is native; a None here is a broken fixture, not a skip");
    assert_bounded("6.6 PNG control", WARMUP, &samples);
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
    // Every host decodes this now (see 6.6's comment) — no decoder-absent skip arm.
    //
    // **Finding 2: asserts the cache HIT directly, by counting decodes, not by
    // measuring footprint growth.** `anim.webp` decodes to ~0.49 MiB — under any
    // growth bound this gate could carry — so the old byte-growth assertion passed
    // identically whether the second load was a real cache hit or a fresh decode:
    // deleting the cache outright still "grew" the footprint by too little to
    // register. `imagedecode::decode_probe` counts real
    // calls into the crate's one decode choke point, so "no new decode happened" is
    // now the literal claim, not an inference from bytes.
    let path = fixture("anim.webp");
    crate::imagecache::reset_for_test();
    crate::imagedecode::decode_probe::reset_for_test();
    let first = load_local_cached(&path).expect("richimg decodes anim.webp on every host");
    let w = first.texture.width();
    let h = first.texture.height();
    drop(first);
    let after_first = crate::imagedecode::decode_probe::count();
    assert_eq!(
        after_first, 1,
        "sanity: the first load must be exactly one genuine decode"
    );
    let second = load_local_cached(&path).expect("cached decode");
    assert_eq!(second.texture.width(), w);
    assert_eq!(second.texture.height(), h);
    let after_second = crate::imagedecode::decode_probe::count();
    assert_eq!(
        after_second, after_first,
        "TDD 6.8: second load of an unchanged local file must be a cache HIT (zero \
         new decodes) — a decode count that moved here means the cache was bypassed, \
         however small the resulting footprint growth was"
    );
}

#[gtktest::test]
fn local_cache_misses_when_the_file_is_replaced_ttd_6_8() {
    // Two PNGs of different widths so the overwrite is visible as a dimension
    // change. A `.webp` temp overwritten with PNG bytes would pick the WebP
    // loader from the extension and fail to decode.
    //
    // **Written with `std::fs::write`, not `std::fs::copy`, and that is the whole
    // point of this test's shape.** `copy` is `CopyFileExW` on Windows and carries
    // the SOURCE file's mtime onto the destination, so the replacement left the
    // stamp unmoved and this test reported a cache defect that did not exist
    // (MEASURED by the Windows seat; both fixtures happened to share an mtime to
    // the nanosecond, so even distinct sources would not have saved it). The
    // PRODUCT half of that finding is why the cache key now carries the file's
    // LENGTH as well — see `imagecache::loader::FileStamp`.
    let dir = std::env::temp_dir();
    let tmp = dir.join(format!(
        "scribobulate-memgate-6_8-{}.png",
        std::process::id()
    ));
    let wide = std::fs::read(fixture("wide.png")).expect("read wide png");
    let logo = std::fs::read(fixture("logo.png")).expect("read logo png");
    assert_ne!(
        wide.len(),
        logo.len(),
        "precondition: the two fixtures must differ in LENGTH, which is half of what \
         the cache keys on"
    );
    std::fs::write(&tmp, &wide).expect("write wide png");
    crate::imagecache::reset_for_test();
    crate::imagedecode::decode_probe::reset_for_test();
    let first = load_local_cached(&tmp).expect("first decode");
    let first_w = first.texture.width();
    drop(first);
    let after_first = crate::imagedecode::decode_probe::count();
    std::fs::write(&tmp, &logo).expect("replace with the smaller png");
    let second = load_local_cached(&tmp).expect("decode after the file was replaced");
    let _ = std::fs::remove_file(&tmp);
    let after_second = crate::imagedecode::decode_probe::count();
    assert_ne!(
        second.texture.width(),
        first_w,
        "TDD 6.8: replacing the file on disk must produce a new decode, not the cached one"
    );
    // Finding 2's instrument, applied here too (not just where it was broken): the
    // dimension check above already proves a fresh decode happened, but stating the
    // COUNT makes the claim exact — exactly one new decode, not merely "a different
    // one from before".
    assert_eq!(
        after_second,
        after_first + 1,
        "TDD 6.8: replacing the file must trigger exactly one new decode"
    );
}

#[gtktest::test]
fn local_cache_misses_when_only_the_length_changes_ttd_6_8() {
    // The Windows condition, reproduced on any platform: a file replaced with
    // DIFFERENT CONTENT whose mtime is then restored to what it was. That is what
    // `CopyFileExW` does by itself (it carries the source's mtime onto the
    // destination), and what `cp -p`, `rsync --times`, `unzip` and a git checkout
    // do everywhere. Before the cache key carried the file's LENGTH, this served
    // the stale decode and the reader never saw their new image.
    let dir = std::env::temp_dir();
    let tmp = dir.join(format!(
        "scribobulate-memgate-6_8-len-{}.png",
        std::process::id()
    ));
    let wide = std::fs::read(fixture("wide.png")).expect("read wide png");
    let logo = std::fs::read(fixture("logo.png")).expect("read logo png");
    std::fs::write(&tmp, &wide).expect("write wide png");
    let stamp = std::fs::metadata(&tmp)
        .and_then(|m| m.modified())
        .expect("read the mtime to restore");
    crate::imagecache::reset_for_test();
    crate::imagedecode::decode_probe::reset_for_test();
    let first = load_local_cached(&tmp).expect("first decode");
    let first_w = first.texture.width();
    drop(first);
    let after_first = crate::imagedecode::decode_probe::count();

    std::fs::write(&tmp, &logo).expect("replace with the smaller png");
    // Put the clock back, so mtime says nothing changed and only the length does.
    let times = std::fs::FileTimes::new().set_modified(stamp);
    std::fs::File::options()
        .write(true)
        .open(&tmp)
        .and_then(|f| f.set_times(times))
        .expect("restore the mtime");
    let restored = std::fs::metadata(&tmp)
        .and_then(|m| m.modified())
        .expect("re-read the mtime");
    assert_eq!(
        restored, stamp,
        "precondition: the mtime really was put back, so this test is about LENGTH"
    );

    let second = load_local_cached(&tmp).expect("decode after the replacement");
    let _ = std::fs::remove_file(&tmp);
    let after_second = crate::imagedecode::decode_probe::count();
    assert_ne!(
        second.texture.width(),
        first_w,
        "TDD 6.8: a replacement the filesystem clock cannot see must still produce a \
         new decode — the cache keys on the file's length as well as its mtime"
    );
    // Finding 2's instrument: exactly one new decode, not merely a different result.
    assert_eq!(
        after_second,
        after_first + 1,
        "TDD 6.8: a length-only replacement must trigger exactly one new decode"
    );
}

#[gtktest::test]
fn local_cache_makes_svg_rerender_free_ttd_6_8() {
    // A large SVG used to re-render on every paint (~239 ms on the reference host)
    // because local images had no cache.
    //
    // **Driven at a NON-IDENTITY zoom, and counted rather than weighed.** At zoom 1.0
    // this never reaches the vector path at all — it is an ordinary raster decode — so
    // the test asserted the wrong thing twice over. And the footprint assertion it used
    // had the same shape the raster gate was just rescued from: a re-render too small
    // to register as growth satisfies it whether or not the cache exists. The
    // re-rasterisation counter answers the question the rubric actually asks.
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("sdd/system-overview.svg");
    crate::imagecache::reset_for_test();
    let zoom = 2.0;

    let before_first = crate::imagedecode::decode_probe::vector_rasterize_count();
    let first = load_texture(&ImageResolution::Local(path.clone()), zoom).expect("SVG re-render");
    let w = first.texture.width();
    drop(first);
    let after_first = crate::imagedecode::decode_probe::vector_rasterize_count();
    assert_eq!(
        after_first - before_first,
        1,
        "precondition: a zoomed SVG must actually re-rasterise once, or this test is \
         measuring the raster path by mistake — which is exactly what it did before"
    );

    let second = load_texture(&ImageResolution::Local(path), zoom).expect("cached SVG");
    assert_eq!(second.texture.width(), w);
    assert_eq!(
        crate::imagedecode::decode_probe::vector_rasterize_count(),
        after_first,
        "TDD 6.8: a second render of an unchanged SVG at the same zoom must be a cache \
         HIT — it must not re-rasterise the document again"
    );
}
