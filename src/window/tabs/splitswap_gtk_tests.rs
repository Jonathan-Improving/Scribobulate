//! GTK integration tests for `win.split-swap`'s WINDOW scope: a
//! new tab must show whichever pane arrangement the window already has, and a
//! tab moved into a different window must adopt THAT window's arrangement —
//! the same shape zoom already has (`window::inherit_from` / `wire_tab_arrival`
//! / `resync_tab_action_state`).

use super::super::*;
use super::*;
use crate::window::testkit::test_app;

/// THE reproduction: flip `win.split-swap` in a window, then add a
/// tab through the real File ▸ New Document path (`create_tab_in_window`, exactly
/// what `add_new_document_tab` calls). The new tab must start swapped too.
///
/// RED on the unfixed tree: a fresh tab starts from its own construction default
/// (`false`) regardless of the window it is born into — it inherits nothing.
#[gtktest::test]
fn a_new_tab_inherits_the_windows_split_swap_arrangement() {
    let app = test_app("com.extollit.scribobulate.integrationtest.splitswapinherit");
    let window = new_window(&app, "IT", "# A\n\nbody", None);

    change_action_state(&window, "split-swap", &true.to_variant());

    let tab_id = create_tab_in_window(&window, "# B\n\nbody", None, false, false)
        .expect("create_tab_in_window returns the new tab's id");
    let tab = winstate::tab_by_id(tab_id).expect("new tab registered");

    assert!(
        tab.split.is_swapped(),
        "a tab created in a window whose panes are swapped must start swapped too — \
         today it starts false, the type's construction default, regardless of \
         the window it is born into"
    );

    window.destroy();
}

/// A tab moved into a DIFFERENT, PRE-EXISTING window adopts that window's own
/// arrangement — the destination's, not the one it carried in its origin window
/// — mirroring zoom's "adopted from the destination on tab move" rule
/// (`winstate` module doc state-scope table).
///
/// Drives the REAL production path: `dnd::move_tab_into_window`, the function a tab
/// dropped onto another window's tab bar runs. `append_page` inside it fires
/// `wire_tab_arrival`'s `connect_page_added` synchronously, so this exercises the
/// guard added there. It must not be re-implemented as a bare `detach_tab` +
/// `append_page`: that skips detaching the origin's format overlay, and on GTK 4.22
/// disposing the moved editor then floods warnings until the case times out. The
/// overlay assertion below is what catches that on GTK 4.6, which tolerates it
/// silently. A BRAND-NEW destination (`move_tab_to_new_window`'s own path)
/// would not discriminate: `new_window_from_source` INHERITS the source's
/// `split_swap`, so a freshly spawned destination starts already agreeing with
/// the origin — the adopt-vs-inherit distinction only shows up moving into a
/// window that already existed with its OWN, different value.
#[gtktest::test]
fn a_tab_moved_into_an_existing_window_adopts_its_split_swap_arrangement() {
    let app = test_app("com.extollit.scribobulate.integrationtest.splitswapadopt");
    let origin = new_window(&app, "origin", "# Origin\n", None);
    let destination = new_window(&app, "destination", "# Destination\n", None);

    // Origin is swapped; the pre-existing destination is not — the two must be
    // free to disagree (the state-scope table's "two windows stay free to
    // differ"), so the destination's own value cannot be an inherited copy.
    change_action_state(&origin, "split-swap", &true.to_variant());

    let tab = state(&origin).expect("origin window has a tab");
    assert!(
        tab.split.is_swapped(),
        "sanity: the tab starts swapped, matching its origin window"
    );

    let origin_chrome = winstate::chrome(&origin).expect("origin chrome");
    let dest_chrome = winstate::chrome(&destination).expect("destination chrome");
    let editor: gtk::Widget = tab.editor.clone().upcast();
    // Control: the origin's format overlay really is parented to the tab being moved,
    // so the post-move assertion below cannot pass vacuously.
    assert_eq!(
        origin_chrome.format_overlay.parent().as_ref(),
        Some(&editor),
        "precondition: the origin window's format overlay sits on its only tab's editor"
    );
    super::dnd::move_tab_into_window(&tab, &dest_chrome);
    assert_ne!(
        origin_chrome.format_overlay.parent().as_ref(),
        Some(&editor),
        "moving a window's only tab into another window must detach the origin's format \
         overlay from the moved editor; left parented, disposing that editor floods \
         `GtkPopover is not a child of GtkSourceView` on GTK 4.22"
    );

    // `wire_tab_arrival`'s split-swap sync runs inside the deferred idle
    // (alongside the zoom re-render it mirrors) — pump it.
    let ctx = glib::MainContext::default();
    for _ in 0..200 {
        if !ctx.iteration(false) {
            break;
        }
    }

    assert!(
        !tab.split.is_swapped(),
        "a tab moved into an existing, un-swapped window must adopt ITS \
         arrangement, not keep the swapped order it carried from its origin"
    );

    origin.destroy();
    destination.destroy();
}
