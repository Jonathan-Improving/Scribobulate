//! WP5 / TDD 2.23a-b, 27.1 (APNG): a truncated file must degrade to
//! `Error::Malformed` from every entry point, never panic.
#[path = "support/mod.rs"]
mod support;

use std::sync::Arc;

use richimg::{first_frame, probe, Animation, Error, Format, Limits};
use support::read_fixture;

#[test]
fn truncated_file_is_malformed_from_every_entry_point() {
    let limits = Limits::default();
    let bytes = read_fixture("apng/truncated.png");

    // Still recognisable as APNG by content (signature + IHDR + a leading
    // acTL before any IDAT) — only the pixel data is gone.
    assert_eq!(richimg::sniff(&bytes), Some(Format::Apng));

    assert_eq!(probe(&bytes, &limits), Err(Error::Malformed));
    assert_eq!(
        Animation::new(Arc::clone(&bytes), &limits).err(),
        Some(Error::Malformed)
    );
    assert_eq!(first_frame(&bytes, &limits).err(), Some(Error::Malformed));
}
