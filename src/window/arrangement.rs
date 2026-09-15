//! The split-pane ARRANGEMENT — whether the editor and preview panes are swapped
//! (`split-swap`) and whether the split is stacked top/bottom
//! (`split-orientation`) — is ONE app-wide preference (TDD 7.3, 15.8a): every tab
//! of every window shows it, and a change made in any window applies everywhere at
//! once. It was once scoped to a window (pane order) and a tab (orientation), and
//! since both are short-lived the preference never felt saved.
//!
//! **Two layers, one source of truth.** The app-scoped `app.split-swap` /
//! `app.split-orientation` stateful actions hold the value and do the work: their
//! `change-state` handler re-applies the arrangement to every tab's `SplitView`
//! and mirrors it onto every window's own `win.` action. The `win.` actions stay
//! only as per-window FORWARDERS, because they are greyed outside split mode and
//! sensitivity is per window — an `app.` action's `enabled` flag is one bit for the
//! whole application. Menus and toolbar bind the `win.` name; its handler holds no
//! logic of its own.
//!
//! Held on the `GtkApplication` rather than in a `thread_local`, so each test
//! application starts from its own value and no test can leak an arrangement into
//! the next (POLICY: a test restores any process-global state it installs).

use super::*;
use gtk::gio::SimpleAction;

pub(crate) const SWAP: &str = "split-swap";
pub(crate) const ORIENTATION: &str = "split-orientation";

/// The two halves of the app-wide split arrangement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SplitArrangement {
    /// Editor and preview panes exchanged (editor right / bottom).
    pub(crate) swapped: bool,
    /// Stacked top/bottom rather than side by side.
    pub(crate) vertical: bool,
}

/// Register `app.split-swap` / `app.split-orientation` once per application,
/// seeded from the saved session. Idempotent; `build_window` calls it before the
/// window's forwarders read it, so whichever window is built first — restored or
/// fresh — seeds the value, and every later window reads the LIVE one. The session
/// is read only while no live value exists yet, which is what keeps this clear of
/// ScrAP-136 (seeding live UI state from the persisted snapshot).
pub(crate) fn ensure_registered(app: &Application) {
    if app.lookup_action(SWAP).is_some() {
        return;
    }
    let saved = crate::session::load();
    add_action(app, SWAP, saved.split_swap);
    add_action(app, ORIENTATION, saved.split_vertical);
}

fn add_action(app: &Application, name: &str, initial: bool) {
    let action = SimpleAction::new_stateful(name, None, &initial.to_variant());
    action.connect_change_state(glib::clone!(
        #[weak]
        app,
        move |action, value| {
            let Some(on) = value.and_then(|v| v.get::<bool>()) else {
                return;
            };
            action.set_state(&on.to_variant());
            apply_everywhere(&app);
        }
    ));
    app.add_action(&action);
}

/// The live app-wide arrangement; the default (un-swapped, side by side) for an
/// application that has not built a window yet.
pub(crate) fn current(app: &Application) -> SplitArrangement {
    let read = |name| {
        app.action_state(name)
            .and_then(|v| v.get::<bool>())
            .unwrap_or(false)
    };
    SplitArrangement {
        swapped: read(SWAP),
        vertical: read(ORIENTATION),
    }
}

/// [`current`] for the application `window` belongs to.
pub(crate) fn for_window(window: &ApplicationWindow) -> SplitArrangement {
    window
        .application()
        .map(|app| current(&app))
        .unwrap_or_default()
}

/// A `win.` forwarder's whole job: hand the request to the app action.
pub(super) fn request(window: &ApplicationWindow, name: &str, on: bool) {
    if let Some(app) = window.application() {
        app.change_action_state(name, &on.to_variant());
    }
}

/// Re-apply the arrangement to every tab of every window — background (deferred)
/// tabs included, since they are registered from creation — and mirror each
/// window's `win.` ticks.
fn apply_everywhere(app: &Application) {
    let arrangement = current(app);
    for window in app
        .windows()
        .into_iter()
        .filter_map(|w| w.downcast::<ApplicationWindow>().ok())
    {
        // `set_state`, never `change_state`: the forwarder's handler would request
        // this very change again.
        set_action_state(&window, SWAP, &arrangement.swapped.to_variant());
        set_action_state(&window, ORIENTATION, &arrangement.vertical.to_variant());
        for tab in winstate::tabs_for_window(&window) {
            tab.split.set_arrangement(arrangement);
        }
    }
}

#[cfg(all(test, feature = "gtk-integration-tests"))]
mod tests {
    use super::*;
    use crate::window::testkit::test_app_suffixed;

    const BOTH: SplitArrangement = SplitArrangement {
        swapped: true,
        vertical: true,
    };

    fn every_tab(window: &ApplicationWindow) -> Vec<SplitArrangement> {
        winstate::tabs_for_window(window)
            .iter()
            .map(|t| t.split.arrangement())
            .collect()
    }

    /// TDD 7.3 / 15.8a: toggling in ONE window rearranges every tab of EVERY window
    /// — a background (never-shown) tab of another window included — and that
    /// other window's own menu/toolbar ticks follow.
    #[gtktest::test]
    fn a_toggle_in_one_window_rearranges_every_tab_of_every_window() {
        let app = test_app_suffixed("arrangementeverywhere");
        let first = new_window(&app, "first", "# A\n", None);
        let second = new_window(&app, "second", "# B\n", None);
        create_tab_in_window(&second, "# C\n", None, false, true).expect("background tab");

        change_action_state(&first, SWAP, &true.to_variant());
        change_action_state(&first, ORIENTATION, &true.to_variant());

        assert_eq!(current(&app), BOTH);
        for window in [&first, &second] {
            assert!(every_tab(window).iter().all(|a| *a == BOTH));
            assert!(bool_action_state(window, SWAP, false));
            assert!(bool_action_state(window, ORIENTATION, false));
        }
        first.destroy();
        second.destroy();
    }

    /// TDD 15.8a: a new tab and a new window are born with the arrangement.
    #[gtktest::test]
    fn new_tabs_and_new_windows_start_with_the_arrangement() {
        let app = test_app_suffixed("arrangementnewborn");
        let first = new_window(&app, "first", "# A\n", None);
        change_action_state(&first, SWAP, &true.to_variant());
        change_action_state(&first, ORIENTATION, &true.to_variant());

        create_tab_in_window(&first, "# B\n", None, false, false).expect("new tab");
        let later = new_window(&app, "later", "# C\n", None);

        assert!(every_tab(&first).iter().all(|a| *a == BOTH));
        assert_eq!(every_tab(&later), vec![BOTH]);
        assert!(bool_action_state(&later, SWAP, false));
        assert!(bool_action_state(&later, ORIENTATION, false));
        first.destroy();
        later.destroy();
    }

    /// TDD 7.3 (survives a restart): the first window of a fresh application takes
    /// the arrangement the session saved.
    #[gtktest::test]
    fn the_first_window_takes_the_saved_arrangement() {
        let dir = tempfile::tempdir().unwrap();
        crate::session::with_state_home_for_test(dir.path(), || {
            crate::session::save(&crate::session::Session {
                split_swap: true,
                split_vertical: true,
                ..Default::default()
            });
            let app = test_app_suffixed("arrangementsaved");
            let window = new_window(&app, "IT", "# A\n", None);
            assert_eq!(current(&app), BOTH);
            assert_eq!(every_tab(&window), vec![BOTH]);
            window.destroy();
        });
    }
}
