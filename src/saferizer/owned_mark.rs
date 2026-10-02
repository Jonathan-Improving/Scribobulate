//! A `GtkTextMark` that deletes itself from its buffer when its owner goes away.

/// A `GtkTextMark` this program created, which deletes itself from its buffer when the
/// owner goes away. A region render's write mark and a find bar's captured passage both
/// hold one.
///
/// **Why the delete must be owned rather than called.** An anonymous mark belongs to the
/// BUFFER until `delete_mark` runs; dropping the Rust handle only unrefs the wrapper. The
/// create and the last use are in different functions, so a delete at each exit is the
/// rule the next exit forgets. The marks live in a LIVE buffer, which a tab keeps for its
/// whole life, so one left behind is never collected, and a `GtkTextMark` has no visible
/// effect until something enumerates them, which is why both accumulated silently, one
/// per splice and two per "Search in selection" capture.
///
/// **A wrapper rather than a `Drop` on the owner**, because an owner such as
/// [`crate::renderer::Renderer`] has fields moved out of it when a render finishes (`preview::build`),
/// and a type that implements `Drop` cannot be partially moved from.
#[derive(Debug)]
pub(crate) struct OwnedMark(gtk::TextMark);

impl OwnedMark {
    /// Take ownership of `mark`, which the caller has just created.
    pub(crate) fn new(mark: gtk::TextMark) -> Self {
        Self(mark)
    }

    /// The mark, for resolving against its buffer.
    pub(crate) fn mark(&self) -> &gtk::TextMark {
        &self.0
    }
}

impl Drop for OwnedMark {
    fn drop(&mut self) {
        use gtk::prelude::{TextBufferExt, TextMarkExt};
        if self.0.is_deleted() {
            return;
        }
        if let Some(buffer) = self.0.buffer() {
            buffer.delete_mark(&self.0);
        }
    }
}
