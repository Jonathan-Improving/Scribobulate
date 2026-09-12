//! WP4 / TDD 2.23a-b (GIF): truncated input must degrade to
//! `Error::Malformed` from every entry point, never panic.
#[path = "support/mod.rs"]
mod support;

use std::sync::Arc;

use richimg::{first_frame, probe, Animation, Error};
use support::read_gif_fixture;

#[test]
fn truncated_file_is_malformed_from_every_entry_point() {
    let limits = richimg::Limits::default();
    let bytes = read_gif_fixture("truncated.gif");

    // Still recognisable as GIF by content — only the frame data is gone.
    assert_eq!(richimg::sniff(&bytes), Some(richimg::Format::Gif));

    assert_eq!(probe(&bytes, &limits), Err(Error::Malformed));
    assert_eq!(
        Animation::new(Arc::clone(&bytes), &limits).err(),
        Some(Error::Malformed)
    );
    assert_eq!(first_frame(&bytes, &limits).err(), Some(Error::Malformed));
}
