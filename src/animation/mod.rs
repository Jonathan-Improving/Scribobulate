//! Animation playback: pausing/resuming animated images (TDD §27).
//!
//! [`policy`] is the reader's Play Animations choice, GTK's own "reduce animations"
//! system setting, and the pure function that reconciles the two into one effective
//! play/pause decision. [`schedule`] is the pure playback decision core (which frame
//! is due, skip-ahead when late, stop after the final loop); [`worker`] is the bridge
//! that decodes a frame off the main thread and hands it back on the main context.
//! [`source`] is the process-wide shared-encoded-bytes registry two pictures of the
//! same file converge on; [`paintable`] is the `GdkPaintable` that actually plays —
//! the tick callback, the schedule/worker wiring, and the picture-construction site
//! in `renderer::start`. [`visibility`] is the
//! "can anyone see this picture right now?" decision — scroll viewport,
//! disclosure nesting, mapped state and minimized-window state, each re-asked from
//! the signal that can actually change it — which [`paintable`] ANDs with
//! [`policy`]'s play/pause decision to decide whether to hold a decoder at all.
//! [`sprites`] is the same playback (`schedule`/`worker`/`policy`, reused rather than
//! reimplemented) for a THEME sprite (TDD 27.9) — a decoration the preview's
//! paint plan (`decorplan.rs`) draws directly rather than through a `GdkPaintable`, so
//! PER-SPRITE visibility comes from that plan's own viewport gates rather than a
//! second geometry check. The VIEW hosting every sprite it draws is still gated by
//! [`visibility`] itself (QA finding, 2026-09-12: the paint-driven gate above never
//! sees a view that has stopped painting altogether — a background tab, a hidden
//! pane), applied once per view rather than once per sprite.

pub(crate) mod paintable;
pub(crate) mod policy;
pub(crate) mod schedule;
pub(crate) mod source;
pub(crate) mod sprites;
pub(crate) mod tick;
pub(crate) mod visibility;
pub(crate) mod worker;
