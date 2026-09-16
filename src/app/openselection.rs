//! What a file chooser's selection means to the application: the `GListModel`
//! GTK hands back, turned into the files `GApplication::open` is given.
//!
//! Extracted from `app/appactions.rs` (GTK-wired, and outside the coverage
//! ratchet) because it is a decision rather than wiring, and because the decision
//! it makes is the one nobody would think to test at the call site: **an empty
//! selection is not an open**. `GtkFileChooser::get_files` is documented to return
//! a model, never to guarantee it holds anything, and the response that carries it
//! is `Accept` either way — so a backend that answers with an empty model on a
//! race (the portal chooser is a separate process, and its reply outlives the
//! dialog that asked) would otherwise reach `g_application_open` with zero files.
//! That is not merely a no-op: the handler's "which window does this batch target"
//! pass would run for a batch that has no files in it.
//!
//! Hence the `Option`, rather than a bare `Vec` the caller may forget to check —
//! there is no meaningful fallback for "the user chose nothing", so the absence is
//! made unrepresentable at the call site instead of being left to a convention
//! (POLICY § Typed GTK seams: `Option` with forced handling, where a wrong answer
//! is worse than no answer).

use gtk::prelude::*;

/// The files a chooser's `selection` names, in the order the chooser reports them,
/// or `None` when it names none.
///
/// Total over the model's contents: an item that is not a `GFile` cannot occur
/// through `GtkFileChooser` (whose model is documented as holding them) and is
/// skipped rather than trusted or panicked on, so a future backend returning
/// something else costs a missing file rather than a crash.
pub(crate) fn chosen_files(selection: &gtk::gio::ListModel) -> Option<Vec<gtk::gio::File>> {
    let files: Vec<gtk::gio::File> = selection
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|obj| obj.downcast::<gtk::gio::File>().ok())
        .collect();
    (!files.is_empty()).then_some(files)
}

#[cfg(test)]
mod tests {
    use super::chosen_files;
    use gtk::gio;
    use gtk::prelude::*;

    /// A model of files comes back in the chooser's own order, unchanged.
    ///
    /// Order is load-bearing rather than cosmetic: `app/openbatch.rs` gives the
    /// FIRST file of a batch the visible, eagerly-rendered tab and defers the rest,
    /// so a reordering here would change which document the user is looking at
    /// after a multi-selection.
    #[test]
    fn a_multi_selection_keeps_the_chooser_s_order() {
        let store = gio::ListStore::new::<gio::File>();
        for name in ["a.md", "b.md", "c.md"] {
            store.append(&gio::File::for_path(name));
        }
        let files = chosen_files(store.upcast_ref()).expect("three files is a selection");
        let names: Vec<String> = files
            .iter()
            .map(|f| f.basename().unwrap_or_default().display().to_string())
            .collect();
        assert_eq!(names, ["a.md", "b.md", "c.md"]);
    }

    /// One file is an ordinary selection, not a special case — the single-selection
    /// path and the multi-selection path are the same path.
    #[test]
    fn one_file_is_a_selection() {
        let store = gio::ListStore::new::<gio::File>();
        store.append(&gio::File::for_path("only.md"));
        assert_eq!(chosen_files(store.upcast_ref()).map(|f| f.len()), Some(1));
    }

    /// The whole reason this is not a bare `Vec`: an empty model must not read as
    /// "open nothing", because the caller cannot forget to handle a `None`.
    #[test]
    fn an_empty_selection_is_not_an_open() {
        let store = gio::ListStore::new::<gio::File>();
        assert!(chosen_files(store.upcast_ref()).is_none());
    }

    /// A non-`GFile` item is skipped rather than trusted — the totality claim in the
    /// doc comment, asserted rather than assumed. Anti-vacuity: the real file beside
    /// it still comes through, so this cannot pass by returning nothing at all.
    #[test]
    fn a_foreign_item_is_skipped_and_the_rest_survive() {
        let store = gio::ListStore::new::<glib::Object>();
        store.append(&gio::File::for_path("real.md"));
        store.append(&gio::MenuItem::new(Some("not a file"), None));
        let files = chosen_files(store.upcast_ref()).expect("the real file survives");
        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0]
                .basename()
                .unwrap_or_default()
                .display()
                .to_string(),
            "real.md"
        );
    }
}
