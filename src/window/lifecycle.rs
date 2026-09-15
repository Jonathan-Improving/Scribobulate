//! Window lifecycle: the close-request handler that persists the session state and
//! guards unsaved changes with a Save / Discard / Cancel prompt.
use super::*;

/// Wire the window's close-request: persist session geometry / view state and, for a
/// dirty document, prompt before closing.
pub(super) fn wire_close_request(window: &ApplicationWindow) {
    let force_close = Rc::new(Cell::new(false));
    window.connect_close_request(move |win| {
        // A close during a PDF export cancels the export and closes once it has
        // stopped, rather than destroying the window beneath it.
        let reclose = win.downgrade();
        if crate::window::defer_until_export_stops(win, move || {
            if let Some(win) = reclose.upgrade() {
                win.close();
            }
        }) {
            return glib::Propagation::Stop;
        }
        log::info!(
            "window close-request ({} tabs, forced: {})",
            winstate::tabs_for_window(win).len(),
            force_close.get()
        );
        // Persist EVERY currently-open window's geometry/zoom/tabs (TDD 7.2),
        // not just this closing one — otherwise closing
        // one window of several would overwrite the saved session with only
        // that window's state, losing every other still-open window entirely.
        // This handles a STANDALONE window close (WM close / Ctrl+W): the set is
        // this window plus whatever else is still open. A COORDINATED quit closes
        // windows sequentially, which would make each successive close persist a
        // shrinking set (the last one persisting only itself — TDD 15.10); that
        // path is handled by `quit_all_windows`, which snapshots all windows once
        // and FREEZES `session::save` for the duration, so this call no-ops then.
        persist_all_windows_session(win);

        if force_close.get() {
            return glib::Propagation::Proceed;
        }
        // Prompt when ANY of the window's tabs needs guarding — not just the
        // active one (a background at-risk tab must not be silently discarded
        // just because the tab the user happens to be looking at is clean).
        // `needs_close_prompt` covers both unsaved edits and a document whose
        // backing file was deleted on disk (TDD 15.22): an edited untitled doc
        // now confirms (Save As / Discard / Cancel) instead of silently
        // discarding the content, and a clean doc over a deleted file confirms
        // too — closing it without a Save would lose its only copy.
        let needs_prompt = winstate::tabs_for_window(win)
            .iter()
            .any(|t| t.needs_close_prompt());
        if !needs_prompt {
            return glib::Propagation::Proceed;
        }
        confirm_close(win, &force_close);
        glib::Propagation::Stop
    });
}

/// Coordinated app quit (File ▸ Exit / Ctrl+Q — `app.quit_action`): snapshot the
/// FULL multi-window session ONCE while every window is still alive, then close
/// each window so the per-window unsaved-changes prompt still fires. Freezing
/// session writes (`session::set_frozen`) after the upfront snapshot stops the
/// sequential per-window closes from re-persisting a SHRINKING window set — the
/// last window to close would otherwise overwrite the session with only itself,
/// losing every other window on restart (TDD 15.10). A cancelled close (the user
/// aborts quit) thaws again from `confirm_close`'s Cancel arm.
///
/// NOT `app.quit()`: that destroys windows without close-request, bypassing the
/// unsaved-changes prompt (the same reason the quit action already looped `close`).
pub(crate) fn quit_all_windows(app: &gtk::Application) {
    let windows = app.windows();
    if let Some(anchor) = windows
        .iter()
        .find_map(|w| w.clone().downcast::<ApplicationWindow>().ok())
    {
        persist_all_windows_session(&anchor);
    }
    crate::session::set_frozen(true);
    for w in &windows {
        w.close();
    }
}

/// Snapshot every currently-open, registered window (`closing` included — it is
/// still fully alive at this point in `close-request`, before `destroy`) into a
/// [`crate::session::Session`] and persist it (TDD 7.2).
///
/// Every window-scoped value — geometry, zoom, and chrome visibility — is read
/// from the window it belongs to, inside the loop below. `closing` is used ONLY
/// to reach the `GtkApplication` (and hence the full window set); it has no
/// privileged say over any value. It used to supply the chrome for the whole
/// session under a "last window to touch it wins" rule, which silently discarded
/// a toggle made in any OTHER window: hide the toolbar in one window, then close
/// a different window whose toolbar was showing, and the session recorded
/// "showing" for everyone.
fn persist_all_windows_session(closing: &ApplicationWindow) {
    let Some(app) = closing.application() else {
        return;
    };
    let windows: Vec<crate::session::WindowSession> = app
        .windows()
        .into_iter()
        .filter_map(|w| w.downcast::<ApplicationWindow>().ok())
        .filter_map(|w| {
            let chrome = winstate::chrome(&w)?;
            let tabs_state = winstate::tabs_for_window(&w);
            if tabs_state.is_empty() {
                return None;
            }
            let active_id = state(&w).map(|st| st.id);
            let active_tab = tabs_state
                .iter()
                .position(|t| Some(t.id) == active_id)
                .unwrap_or(0);
            let (width, height) = (w.width(), w.height());
            Some(crate::session::WindowSession {
                width: if width > 0 {
                    width
                } else {
                    config().window.width
                },
                height: if height > 0 {
                    height
                } else {
                    config().window.height
                },
                zoom_level: chrome.zoom_level.get(),
                active_tab,
                // THIS window's own chrome, off its own `win.*` toggles — the
                // same reader `window::inherit_from` seeds a new window with, so
                // the value that gets persisted and the value that gets
                // inherited can never drift apart.
                chrome: crate::window::read_window_chrome(&w),
                tabs: tabs_state
                    .iter()
                    .map(|t| crate::session::TabSession {
                        path: t.path.borrow().clone(),
                        // Persisted so a restored tab can be matched to the swap file
                        // holding its unsaved content. Advisory only: the swap file's
                        // own header is authoritative, so a session that loses this
                        // costs a recovered tab its placement, never its content
                        // (`swapfile`'s self-sufficiency principle).
                        doc_id: Some(t.doc_id().as_str().to_string()),
                        view_mode: t.view_mode.get(),
                        show_unsafe_images: t.allow_unsafe_images.get(),
                    })
                    .collect(),
            })
        })
        .collect();

    // App-wide, off the `app.split-*` actions' own state — one value, no "which
    // window's?" question (TDD 7.3).
    let crate::window::arrangement::SplitArrangement { swapped, vertical } =
        crate::window::arrangement::current(&app);
    crate::session::save(&crate::session::Session {
        split_swap: swapped,
        split_vertical: vertical,
        // Genuinely app-wide, and read straight off the live active theme rather
        // than off any window's action state: the theme is one app-wide CSS
        // provider, so there is exactly one value and no "which window's?"
        // question to answer (TDD 18.12).
        preview_theme: crate::theme::active().id.clone(),
        // The READER'S raw choice off `app.play-animations`'s own state — never the
        // effective (system "reduce animations"-adjusted) value, which is
        // recomputed live from `gtk-enable-animations` on every launch instead of
        // being persisted (TDD 27.7). `app` here is the same `GtkApplication` every
        // window shares, so — like `preview_theme` above — there is exactly one
        // value and no "which window's?" question to answer.
        play_animations: crate::animation::policy::reader_choice(&app),
        windows,
    });
}

#[cfg(all(test, feature = "gtk-integration-tests"))]
mod tests {
    use super::*;
    use crate::animation::policy::EnableAnimationsGuard;

    /// TDD 27.7's persistence half, pinned at the actual write site: a close/quit
    /// must save the READER'S raw `app.play-animations` choice, never the effective
    /// (system "reduce animations"-adjusted) value. Forcing the two to genuinely
    /// disagree — reader ON, system setting forcing OFF — is the only condition
    /// that can catch a save site that persists the wrong one (a mutation that
    /// swaps `reader_choice` for `current` at the call site above passes every
    /// other test in this file, since they never diverge).
    #[gtktest::test]
    fn persisted_choice_is_the_readers_raw_choice_not_the_effective_state() {
        let dir = tempfile::tempdir().unwrap();
        crate::session::with_state_home_for_test(dir.path(), || {
            let _settings = EnableAnimationsGuard::set(false); // reduce-animations ON

            let app = gtk::Application::new(
                Some("com.extollit.scribobulate.integrationtest.lifecycle.persistchoice"),
                gtk::gio::ApplicationFlags::NON_UNIQUE,
            );
            app.register(gtk::gio::Cancellable::NONE)
                .expect("register before building a window");
            // The same shape `app::appactions::add_play_animations_action` registers
            // (that function is `pub(super)` to `app` and reads the real session — this
            // mirrors its shape without either dependency, matching `animation::policy`'s
            // own `add_bare_action` test helper).
            let action = gtk::gio::SimpleAction::new_stateful(
                crate::animation::policy::ACTION_NAME,
                None,
                &true.to_variant(), // the reader chose ON
            );
            action.connect_change_state(|act, value| {
                let Some(value) = value else { return };
                act.set_state(value);
            });
            app.add_action(&action);

            let window = crate::window::new_window(&app, "IT", "alpha", None);
            persist_all_windows_session(&window);

            assert!(
                crate::session::load().play_animations,
                "the saved field must be the reader's ON choice, not the effective OFF state \
                 the system setting is currently forcing"
            );

            window.destroy();
        });
    }

    /// TDD 7.3's persistence half, at the write site: a close saves the app-wide
    /// split arrangement once, at the top level, whichever window set it.
    #[gtktest::test]
    fn persisted_split_arrangement_is_the_app_wide_value() {
        let dir = tempfile::tempdir().unwrap();
        crate::session::with_state_home_for_test(dir.path(), || {
            let app = crate::window::testkit::test_app_suffixed("persistarrangement");
            let setter = crate::window::new_window(&app, "setter", "# A\n", None);
            let closing = crate::window::new_window(&app, "closing", "# B\n", None);
            change_action_state(&setter, "split-swap", &true.to_variant());
            change_action_state(&setter, "split-orientation", &true.to_variant());

            persist_all_windows_session(&closing);

            let saved = crate::session::load();
            assert!(saved.split_swap && saved.split_vertical);
            setter.destroy();
            closing.destroy();
        });
    }
}
