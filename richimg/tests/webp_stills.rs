//! TDD 2.23a-b: still WebP (lossy, lossless, and with alpha) decode
//! correctly and report `Info` per the still-image contract.
#[path = "support/mod.rs"]
mod support;

use richimg::{first_frame, probe, Animation, Error, Limits, LoopCount};
use support::{assert_matches_reference, fixture_path, read_fixture, read_ref_frame};

const WIDTH: u32 = 16;
const HEIGHT: u32 = 16;

fn assert_still_info(bytes: &[u8], limits: &Limits) {
    let info = probe(bytes, limits).expect("probe a still webp");
    assert_eq!(info.width, WIDTH);
    assert_eq!(info.height, HEIGHT);
    assert!(!info.animated);
    assert_eq!(info.frame_count, Some(1));
    assert_eq!(info.loop_count, LoopCount::Finite(1));
}

fn assert_still_decodes_to(fixture: &str, reference_stem: &str) {
    let limits = Limits::default();
    let bytes = read_fixture(fixture);
    assert_still_info(&bytes, &limits);

    let reference = read_ref_frame(reference_stem, 0);

    // `first_frame` convenience.
    let frame = first_frame(&bytes, &limits).expect("first_frame on a still");
    assert_eq!(frame.width, WIDTH);
    assert_eq!(frame.height, HEIGHT);
    assert_eq!(frame.index, 0);
    assert_matches_reference(
        &frame.rgba,
        &reference,
        WIDTH,
        HEIGHT,
        &[],
        &format!("{fixture} via first_frame"),
    );

    // `Animation::next_frame` on a still always returns frame 0, every call.
    let mut anim = Animation::new(bytes, &limits).expect("open a still as an Animation");
    for call in 0..3 {
        let frame = anim.next_frame().unwrap_or_else(|err| {
            panic!("{fixture} next_frame call {call} failed: {err}");
        });
        assert_eq!(frame.index, 0, "{fixture} call {call}: index must stay 0");
        assert_matches_reference(
            &frame.rgba,
            &reference,
            WIDTH,
            HEIGHT,
            &[],
            &format!("{fixture} via Animation::next_frame call {call}"),
        );
    }
}

#[test]
fn lossy_still_decodes_correctly() {
    assert_still_decodes_to("lossy_still.webp", "lossy_still");
}

#[test]
fn lossless_still_decodes_correctly() {
    assert_still_decodes_to("lossless_still.webp", "lossless_still");
}

#[test]
fn alpha_still_decodes_with_straight_alpha() {
    assert_still_decodes_to("alpha_still.webp", "alpha_still");
}

#[test]
fn sniff_recognises_webp_fixtures_regardless_of_file_name() {
    // The fixtures are all named ".webp", so this mainly documents that
    // sniff() (unit-tested exhaustively in src/format.rs) also agrees with
    // real encoder output, not just hand-built byte strings.
    let bytes = read_fixture("lossy_still.webp");
    assert_eq!(richimg::sniff(&bytes), Some(richimg::Format::WebP));
}

#[test]
fn fixtures_directory_is_present() {
    // A cheap sanity check that fails loudly (rather than every other test
    // failing with a confusing "no such file") if make.sh was never run.
    assert!(
        fixture_path("lossy_still.webp").exists(),
        "run richimg/tests/fixtures/make.sh to (re)generate fixtures"
    );
}

#[test]
fn unrecognised_bytes_are_unsupported_via_every_entry_point() {
    let limits = Limits::default();
    let bytes: std::sync::Arc<[u8]> = std::sync::Arc::from(&b"plainly not an image"[..]);
    assert_eq!(probe(&bytes, &limits), Err(Error::Unsupported));
    assert_eq!(
        Animation::new(std::sync::Arc::clone(&bytes), &limits).err(),
        Some(Error::Unsupported)
    );
    assert_eq!(first_frame(&bytes, &limits).err(), Some(Error::Unsupported));
}
