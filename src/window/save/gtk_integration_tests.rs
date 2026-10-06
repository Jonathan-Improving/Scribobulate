use super::*;

/// Build a registered, non-unique application for a test window to live in.
use crate::window::testkit::test_app_suffixed as test_app;

/// Run `adopt_and_save` — the whole of Save As after the chooser has answered —
/// to completion, and report whether it wrote.
fn drive_save_as(window: &ApplicationWindow, st: &Rc<TabState>, path: std::path::PathBuf) -> bool {
    let outcome: Rc<Cell<Option<bool>>> = Rc::new(Cell::new(None));
    let sink = Rc::clone(&outcome);
    let window = window.clone();
    let st = Rc::clone(st);
    gtk::glib::MainContext::default().spawn_local(async move {
        sink.set(Some(adopt_and_save(&window, &st, path).await));
    });
    assert!(
        crate::docio::settle(|| outcome.get().is_some()),
        "the Save As must complete"
    );
    outcome.get() == Some(true)
}

/// **Save As titles the window by the same formula as every other path (TDD 4.7 / 15.7).**
///
/// Save As used to derive the title itself, from a bare `file_name()`, and so
/// produced `saved-as.md` where every other path produces
/// `saved-as.md — Scribobulate`. The suffix is not decoration: it is what makes
/// the window identifiable in a taskbar or window switcher, and a derived view
/// that disagrees with itself depending on *how* the document got its name is
/// exactly the Derived-view CAM row 4 / column B failure.
///
/// The assertion is deliberately against `window_title_for_tabs`' output rather
/// than a literal: a test carrying its own copy of the formula would be a fourth
/// derivation, and would pass while the window said something else.
#[gtktest::test]
fn save_as_titles_the_window_by_the_one_shared_formula() {
    let dir = tempfile::tempdir().unwrap();
    crate::session::with_state_home_for_test(dir.path(), || {
        let app = test_app("saveastitle");
        let window = crate::window::new_window(&app, "IT", "content\n", None);
        let st = state(&window).expect("state");

        assert!(drive_save_as(&window, &st, dir.path().join("saved-as.md")));

        assert_eq!(
            window.title().as_deref(),
            Some(winstate::window_title_for_tabs(1, Some("saved-as.md")).as_str()),
            "Save As must produce the same title the open/restore/link paths do"
        );
        window.destroy();
    });
}

/// **Save As in a multi-tab window keeps the sibling count (TDD 15.7).**
///
/// The second, quieter half of the same defect, and the one no amount of staring
/// at the old call site would have surfaced: it retitled the *window* from one
/// tab's filename ALONE, dropping the count of everything else the window held —
/// a window title actively misdescribing its contents. Routing through the choke
/// point fixes both instances at once, which is the argument for deleting the
/// second derivation rather than patching it.
///
/// The title now leads with the active document's name in both cases, so what
/// distinguishes a correct Save As from the old derivation is the `(+1 document)`
/// the choke point adds and a filename-only derivation cannot: the assertion is
/// still against the shared formula, and it is still the multi-tab half of it.
#[gtktest::test]
fn save_as_in_a_multi_tab_window_keeps_the_count_title() {
    let dir = tempfile::tempdir().unwrap();
    crate::session::with_state_home_for_test(dir.path(), || {
        let app = test_app("saveasmultitab");
        let window = crate::window::new_window(&app, "IT", "content\n", None);
        let first = state(&window).expect("state");

        // A second tab, then back to the first — Save As always acts on the
        // active document, and `create_tab_in_window` switches to what it makes.
        crate::window::create_tab_in_window(&window, "elsewhere", None, false, false)
            .expect("a second tab");
        let chrome = winstate::chrome(&window).expect("chrome");
        chrome.tabs.focus_page(&first.content_box);
        assert_eq!(
            winstate::tabs_for_window(&window).len(),
            2,
            "precondition: the window holds more than one document"
        );

        assert!(drive_save_as(
            &window,
            &first,
            dir.path().join("one-of-two.md")
        ));

        assert_eq!(
            window.title().as_deref(),
            Some(winstate::window_title_for_tabs(2, Some("one-of-two.md")).as_str()),
            "a window with two documents names the active one AND counts the \
             other, however the active one acquired its name"
        );
        // …and the count is what a filename-only derivation would have dropped:
        // pin it literally too, so this cannot pass on a title that agrees with
        // the formula only because the formula itself lost the count.
        assert!(
            window.title().is_some_and(|t| t.contains("(+1 document)")),
            "the sibling count must survive a Save As, got {:?}",
            window.title()
        );
        window.destroy();
    });
}

/// **Every modal confirmation carries a window title.**
///
/// `GtkMessageDialog` leaves the title empty, and GDK-Win32 refuses an empty
/// caption — it substitutes a literal `.` (gtk-4.22.4
/// `gdk/win32/gdksurface-win32.c:1238`), which is what the app's close, overwrite
/// and save-error dialogs showed in their title bars and in the taskbar on the
/// native Win32 frame.
///
/// The check runs everywhere rather than under a Windows gate, because the
/// *property* is portable even though only one backend renders the failure: the
/// title is either set at the shared construction site or it is not, and this is
/// the assertion that keeps it set.
///
/// It diffs the window's modal transients across the call rather than scanning
/// for one, because a document window already owns another modal transient — the
/// Keyboard Shortcuts help window, built with the rest of the chrome — so
/// "the modal transient" is not a well-formed question. The diff also states the
/// stronger fact: the call produced **exactly one** new modal, and that one is
/// titled.
#[gtktest::test]
fn a_modal_confirmation_carries_a_window_title() {
    let dir = tempfile::tempdir().unwrap();
    crate::session::with_state_home_for_test(dir.path(), || {
        let app = test_app("dialogtitle");
        let window = crate::window::new_window(&app, "IT", "content\n", None);

        let before = modal_transients_of(&window);
        confirm_dialog(
            &window,
            gtk::MessageType::Question,
            "Save changes before closing?",
            "If you don't save, your changes will be lost.",
            &[("Cancel", gtk::ResponseType::Cancel)],
            gtk::ResponseType::Cancel,
            |_, _| {},
        );
        let opened: Vec<gtk::Window> = modal_transients_of(&window)
            .into_iter()
            .filter(|w| !before.contains(w))
            .collect();

        assert_eq!(
            opened.len(),
            1,
            "precondition: the call opened exactly one modal, or this test is \
             asserting about the wrong window"
        );
        assert_eq!(
            opened[0].title().as_deref(),
            Some(winstate::APP_NAME),
            "a confirmation built with no title renders as a lone '.' on the \
             native Win32 frame"
        );

        opened[0].destroy();
        window.destroy();
    });
}

/// Every modal toplevel that is transient for `window`, in `toplevels()` order.
fn modal_transients_of(window: &ApplicationWindow) -> Vec<gtk::Window> {
    let parent: &gtk::Window = window.upcast_ref();
    let toplevels = gtk::Window::toplevels();
    (0..toplevels.n_items())
        .filter_map(|i| toplevels.item(i))
        .filter_map(|o| o.downcast::<gtk::Window>().ok())
        .filter(|w| w.is_modal() && w.transient_for().as_ref() == Some(parent))
        .collect()
}

/// **A second Save while one is still being written is dropped, not raced (TDD 4.10).**
///
/// Unreachable on a local disk — the write finishes before a second request can be
/// made — so the filesystem is made slow in-process (`docio::slow_io`), which puts
/// the latency on the pool thread exactly where a slow mount's would land.
///
/// The point is not that the second request is refused; it is that **one text
/// reaches disk and the baseline records that same text**. Two writes allowed to
/// race can land in either order and report completion in either order, so the app
/// can believe it saved something it did not (C1).
#[gtktest::test]
fn a_second_save_while_one_is_in_flight_is_dropped_not_raced() {
    let dir = tempfile::tempdir().unwrap();
    crate::session::with_state_home_for_test(dir.path(), || {
        let doc = dir.path().join("busy.md");
        std::fs::write(&doc, "start\n").unwrap();
        let app = gtk::Application::new(
            Some("com.extollit.scribobulate.integrationtest.savebusy"),
            gtk::gio::ApplicationFlags::NON_UNIQUE,
        );
        app.register(gtk::gio::Cancellable::NONE).expect("register");
        let window = crate::window::new_window(&app, "IT", "start\n", Some(&doc));
        let st = state(&window).expect("state");

        let _slow = crate::docio::slow_io(std::time::Duration::from_millis(300));
        st.editor_buf.set_text("first\n");
        save_with_guard(&window);
        // Still in flight: the guard read alone has not come back yet.
        st.editor_buf.set_text("second\n");
        save_with_guard(&window);

        assert!(
            crate::docio::settle(|| !st.is_dirty()),
            "a save must eventually land"
        );
        let on_disk = std::fs::read_to_string(&doc).unwrap();
        assert_eq!(
            on_disk,
            *st.saved_baseline.borrow(),
            "the bytes on disk and the recorded clean baseline must be the SAME \
             text — two writes allowed to race can disagree, and the application \
             then believes it saved something it did not (C1)"
        );
        window.destroy();
    });
}

/// **A guard read overtaken by our OWN save re-reads instead of accusing the user
/// (TDD 5.7).**
///
/// The guard compares the bytes it read against `saved_baseline` — and reads that
/// baseline at *decision* time, which is after its read came back. A save of ours
/// completing inside that window leaves the two describing different moments:
/// pre-write bytes against a post-write baseline. They differ, so the user is told
/// another program modified their file and asked whether to overwrite it. Nothing
/// else wrote to it; they are being asked about their own save.
///
/// Deliberately **not** an interleaving drive, for the reason
/// `a_completed_save_supersedes_a_read_that_was_already_in_flight` states: the
/// real window is the gap between a read's syscall and its completion callback,
/// which no test can win on purpose. What it drives instead is the *state* that
/// race produces, installed while the guard's read is genuinely out on the pool —
/// the baseline and source a landed save leaves, with the file itself still
/// holding the bytes the read is about to return. Every input to the decision is
/// then exactly what the race delivers.
///
/// Mutation: removing `st.write_epoch.bump()` from `save_window`, or the re-read
/// loop from `save_with_guard_tab`, puts the overwrite prompt on screen and leaves
/// the document unwritten.
#[gtktest::test]
fn a_guard_read_our_own_save_overtook_is_re_read_not_challenged() {
    let dir = tempfile::tempdir().unwrap();
    crate::session::with_state_home_for_test(dir.path(), || {
        let doc = dir.path().join("overtaken.md");
        std::fs::write(&doc, "start\n").unwrap();
        let app = test_app("saveovertaken");
        let window = crate::window::new_window(&app, "IT", "start\n", Some(&doc));
        let st = state(&window).expect("state");

        let _slow = crate::docio::slow_io(std::time::Duration::from_millis(300));
        st.editor_buf.set_text("mine\n");
        save_with_guard(&window);

        // While that read is out on the pool: a save of ours lands. This is what
        // one leaves behind — baseline and source moved to the text it wrote, and
        // the write announced (`save_window`).
        gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(60), {
            let st = Rc::clone(&st);
            move || {
                st.set_source("ours\n");
                *st.saved_baseline.borrow_mut() = "ours\n".to_owned();
                st.write_epoch.bump();
            }
        });
        // …and the file catches up only AFTER the guard's read has taken its
        // answer, so that answer is the pre-write text the race hands back.
        let catch_up = doc.clone();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(360), move || {
            std::fs::write(&catch_up, "ours\n").unwrap();
        });

        let before = modal_transients_of(&window);
        // Settling on EITHER outcome so the failing arm reports in a second
        // rather than after the full deadline.
        let settled = crate::docio::settle(|| {
            !st.is_dirty() || modal_transients_of(&window).len() > before.len()
        });

        assert_eq!(
            modal_transients_of(&window).len(),
            before.len(),
            "no overwrite prompt may appear: the only thing that changed the file \
             was this application's own save, and a guard that accuses the user of \
             a conflict they did not cause teaches them to answer Overwrite without \
             reading it"
        );
        assert!(settled && !st.is_dirty(), "the save must land");
        assert_eq!(
            std::fs::read_to_string(&doc).unwrap(),
            "mine\n",
            "the guard's second read agreed with the baseline, so the text the user \
             pressed Save on is what reached disk"
        );
        window.destroy();
    });
}

/// **A save must not starve while the file watcher is churning.**
///
/// Regression guard for a defect this project's own slow-filesystem rig caught an
/// hour after it was written. The save guard briefly checked a `DocEpoch` ticket
/// and re-issued itself when the ticket was stale. On a filesystem GIO *polls*
/// rather than watches — any FUSE or network mount, i.e. precisely the case the
/// asynchronous save exists for — watcher events arrive faster than a slow read
/// completes, so the guard never observed a current ticket and re-issued forever.
/// MEASURED: 13 re-issues at 1.5 s intervals, nothing written, and the only trace
/// an `info` log line. **The user's Save silently never happened.**
///
/// The lesson is in the shape, not the mechanism: a retry whose precondition is
/// invalidated by an *independent* event source is not a retry, it is a livelock.
#[gtktest::test]
fn a_save_lands_even_while_the_watcher_keeps_claiming() {
    let dir = tempfile::tempdir().unwrap();
    crate::session::with_state_home_for_test(dir.path(), || {
        let doc = dir.path().join("churn.md");
        std::fs::write(&doc, "start\n").unwrap();
        let app = gtk::Application::new(
            Some("com.extollit.scribobulate.integrationtest.savechurn"),
            gtk::gio::ApplicationFlags::NON_UNIQUE,
        );
        app.register(gtk::gio::Cancellable::NONE).expect("register");
        let window = crate::window::new_window(&app, "IT", "start\n", Some(&doc));
        let st = state(&window).expect("state");

        let _slow = crate::docio::slow_io(std::time::Duration::from_millis(200));
        st.editor_buf.set_text("written despite the churn\n");
        save_with_guard(&window);

        // Stand in for a polling watcher firing throughout the save: every claim
        // invalidates any ticket taken before it.
        let churn = gtk::glib::timeout_add_local(std::time::Duration::from_millis(20), {
            let st = Rc::clone(&st);
            move || {
                st.doc_epoch.claim();
                gtk::glib::ControlFlow::Continue
            }
        });
        let landed = crate::docio::settle(|| !st.is_dirty());
        churn.remove();

        assert!(
            landed,
            "the save must land despite continuous watcher activity — a guard that \
             re-issues whenever an independent event source has moved never \
             observes a quiet moment, and the write never happens"
        );
        assert_eq!(
            std::fs::read_to_string(&doc).unwrap(),
            "written despite the churn\n"
        );
        window.destroy();
    });
}

/// **A completed save announces itself, so a read already in flight is discarded.**
///
/// This pins the *wiring* the `DocEpoch` unit tests cannot see: that the real save
/// path actually calls `bump()` on completion. The consequence — an older read
/// losing to a newer mutation — is proved there, deterministically, because it is
/// pure data.
///
/// It is deliberately NOT an interleaving drive. A first attempt issued a reload
/// and then a save and expected the save to win; both go to the same pool, their
/// completion order is not controllable from here, and — worse — "Reload then Save"
/// is a sequence whose *correct* outcome is the reverted content being saved. A
/// test that has to win a race to pass is a flaky test asserting the wrong thing.
///
/// Mutation: removing `st.doc_epoch.bump()` from `save_window` fails this.
#[gtktest::test]
fn a_completed_save_supersedes_a_read_that_was_already_in_flight() {
    let dir = tempfile::tempdir().unwrap();
    crate::session::with_state_home_for_test(dir.path(), || {
        let doc = dir.path().join("raced.md");
        std::fs::write(&doc, "on disk before the save\n").unwrap();

        let app = gtk::Application::new(
            Some("com.extollit.scribobulate.integrationtest.savebumps"),
            gtk::gio::ApplicationFlags::NON_UNIQUE,
        );
        app.register(gtk::gio::Cancellable::NONE).expect("register");
        let window = crate::window::new_window(&app, "IT", "on disk before the save\n", Some(&doc));
        let st = state(&window).expect("state registered after new_window");
        st.editor_buf.set_text("the user's newest work\n");

        // Stand in for a read that went out before the save — exactly what the
        // live-reload watcher holds while a save is running.
        let outstanding = st.doc_epoch.claim();
        assert!(st.doc_epoch.is_current(outstanding), "sanity");

        save_with_guard(&window);
        assert!(
            crate::docio::settle(|| !st.is_dirty()),
            "the save must land"
        );

        assert_eq!(
            std::fs::read_to_string(&doc).unwrap(),
            "the user's newest work\n"
        );
        assert!(
            !st.doc_epoch.is_current(outstanding),
            "a save that changed the baseline must supersede a read already in \
             flight: applying that read afterwards puts pre-save content in the \
             buffer AND records it as clean, so the tab reads clean while \
             differing from its own file"
        );

        window.destroy();
    });
}

/// The **close-prompt** save path: `save_and_then` must still write, and must still
/// call its callback, now that the write is asynchronous.
///
/// This path had no test at all, which mattered more than the count suggests: it is
/// the one the close confirmation runs, so its callback is what actually closes the
/// tab or window. The async conversion put that callback inside a `spawn_local`,
/// and a callback that never fires does not fail loudly — the tab simply stays
/// open, having silently swallowed the user's "Save", with nothing logged. The
/// assertion that it RAN is therefore as load-bearing as the assertion that it
/// reported success.
#[gtktest::test]
fn the_close_prompt_save_path_writes_and_reports_back() {
    let dir = tempfile::tempdir().unwrap();
    crate::session::with_state_home_for_test(dir.path(), || {
        let doc = dir.path().join("closing.md");
        std::fs::write(&doc, "before\n").unwrap();

        let app = gtk::Application::new(
            Some("com.extollit.scribobulate.integrationtest.closesave"),
            gtk::gio::ApplicationFlags::NON_UNIQUE,
        );
        app.register(gtk::gio::Cancellable::NONE).expect("register");
        let window = crate::window::new_window(&app, "IT", "before\n", Some(&doc));
        let st = state(&window).expect("state registered after new_window");
        st.editor_buf.set_text("after\n");
        assert!(st.is_dirty(), "precondition: there is something to save");

        let outcome: Rc<Cell<Option<bool>>> = Rc::new(Cell::new(None));
        let sink = Rc::clone(&outcome);
        save_and_then(&window, move |_, saved| sink.set(Some(saved)));

        assert!(
            crate::docio::settle(|| outcome.get().is_some()),
            "the callback must run — the close confirmation does nothing at all \
             until it does, so a callback lost inside the spawned future reads to \
             the user as Save having been ignored"
        );
        assert_eq!(
            outcome.get(),
            Some(true),
            "and must report success only for a write that actually happened"
        );
        assert_eq!(std::fs::read_to_string(&doc).unwrap(), "after\n");
        assert!(!st.is_dirty(), "the tab is clean once the write lands");

        window.destroy();
    });
}

/// The whole save path end to end with the write on GLib's I/O thread pool: the
/// bytes reach the file, the tab goes clean, its crash-recovery snapshot is
/// retired, and the write gate is open for the next save.
///
/// **The snapshot half is the part a unit test cannot reach and the part most
/// worth pinning.** `refresh_dirty_status` — the choke point that used to retire
/// the snapshot — is window-scoped and acts on whichever tab is ACTIVE. That was
/// the same tab while the write was synchronous; with the main loop running
/// during the write it need not be, so `save_window` now syncs the written tab's
/// swap itself. Mutation: removing that `sync_tab_swap(&st)` leaves the snapshot
/// on disk here, and the next crash would offer already-saved work back as
/// "unsaved".
#[gtktest::test]
fn a_save_reaches_disk_retires_the_snapshot_and_reopens_the_gate() {
    let dir = tempfile::tempdir().unwrap();
    crate::session::with_state_home_for_test(dir.path(), || {
        let doc = dir.path().join("doc.md");
        std::fs::write(&doc, "original\n").unwrap();

        let app = gtk::Application::new(
            Some("com.extollit.scribobulate.integrationtest.asyncsave"),
            gtk::gio::ApplicationFlags::NON_UNIQUE,
        );
        app.register(gtk::gio::Cancellable::NONE)
            .expect("register before building any window");
        let window = crate::window::new_window(&app, "IT", "original\n", Some(&doc));
        let st = state(&window).expect("state registered after new_window");

        st.editor_buf.set_text("edited on the way past\n");
        assert!(st.is_dirty(), "precondition: the buffer differs from disk");
        let snapshot = crate::swapfile::swap_path(Some(&doc), &st.doc_id())
            .expect("a swap path resolves under the test state home");
        assert!(
            crate::docio::settle(|| snapshot.exists()),
            "precondition: a dirty document carries a crash-recovery snapshot"
        );

        // A SECOND tab, switched to the instant the save is issued. `spawn_local`
        // does not run its future until the loop iterates and `focus_page` is
        // synchronous, so the written document is guaranteed to be a BACKGROUND
        // tab by the time the write completes — which is the case the two
        // tab-scoped calls in `save_window` exist for, and the only way to reach
        // it deterministically.
        let other = crate::window::create_tab_in_window(&window, "elsewhere", None, false, false)
            .and_then(crate::winstate::tab_by_id)
            .expect("a second tab");
        // `create_tab_in_window` switches to the tab it makes, so come back to
        // the document first: Save always means "the active document", and
        // invoking it on the untitled tab would open a Save As chooser instead.
        let chrome = crate::winstate::chrome(&window).expect("chrome registered");
        chrome.tabs.focus_page(&st.content_box);
        // Pins the WIRING the re-read guard's own test cannot see, since that one
        // installs a landed save's state by hand: that the real write path
        // announces itself. Mutation: removing `st.write_epoch.bump()` from
        // `save_window` fails the assertion below, and with it every guard read a
        // later save issues starts believing a baseline it may not be about.
        let before_save = st.write_epoch.observe();
        save_with_guard(&window);
        chrome.tabs.focus_page(&other.content_box);
        assert_ne!(
            state(&window).map(|t| t.id),
            Some(st.id),
            "precondition: the document being written is no longer the active tab"
        );
        assert!(
            crate::docio::settle(|| !st.is_dirty()),
            "the save must land: both the guard read and the write are off the \
             main thread now, so this needs the loop to run"
        );

        assert_eq!(
            std::fs::read_to_string(&doc).unwrap(),
            "edited on the way past\n",
            "the edited buffer must actually reach the file"
        );
        assert!(
            !st.write_epoch.is_current(before_save),
            "a landed save must announce that it moved the baseline, or a guard \
             read already out compares pre-write bytes against it and accuses the \
             user of a conflict they caused themselves (TDD 5.7)"
        );
        // A test ASSERTING on the gate, not branching on it to write — the
        // distinction clippy.toml's ban draws.
        #[expect(clippy::disallowed_methods)]
        let still_busy = st.write_gate.is_busy();
        assert!(
            !still_busy,
            "the write gate must reopen, or Save is dead for this document for \
             the rest of the session — silently"
        );
        assert!(
            crate::docio::settle(|| !snapshot.exists()),
            "a saved document is clean, so its crash-recovery snapshot must go: \
             leaving it resurrects already-saved work as unsaved after a crash"
        );

        window.destroy();
    });
}

/// Save All is enabled when any tab needs writing, and saves every titled
/// dirty tab (TDD 4.12). Mutation: dropping the `any(needs_close_prompt)`
/// gate leaves Save All tracking only the active tab.
#[gtktest::test]
fn save_all_saves_every_dirty_titled_tab_and_tracks_any_dirty() {
    let dir = tempfile::tempdir().unwrap();
    crate::session::with_state_home_for_test(dir.path(), || {
        let a_path = dir.path().join("a.md");
        let b_path = dir.path().join("b.md");
        std::fs::write(&a_path, "a0\n").unwrap();
        std::fs::write(&b_path, "b0\n").unwrap();

        let app = gtk::Application::new(
            Some("com.extollit.scribobulate.integrationtest.saveall"),
            gtk::gio::ApplicationFlags::NON_UNIQUE,
        );
        app.register(gtk::gio::Cancellable::NONE).expect("register");
        let window = crate::window::new_window(&app, "IT", "a0\n", Some(&a_path));
        let tab_a = state(&window).expect("tab A");
        let tab_b_id =
            crate::window::create_tab_in_window(&window, "b0\n", Some(&b_path), false, false)
                .expect("tab B");
        let tab_b = winstate::tab_by_id(tab_b_id).expect("tab B registered");
        // create_tab_in_window opens with the given source as clean baseline.
        tab_a.editor_buf.set_text("a1\n");
        tab_b.editor_buf.set_text("b1\n");
        // Active is B; A is dirty in the background — Save All must still enable.
        update_save_action_state(&window);
        assert!(
            window
                .lookup_action("save-all")
                .and_then(|a| a.downcast::<gtk::gio::SimpleAction>().ok())
                .is_some_and(|a| a.is_enabled()),
            "Save All enabled when a background tab is dirty"
        );

        let chrome = winstate::chrome(&window).expect("chrome");
        chrome.tabs.focus_page(&tab_a.content_box);
        save_all(&window);
        assert!(
            crate::docio::settle(|| !tab_a.is_dirty() && !tab_b.is_dirty()),
            "Save All must write every dirty titled tab"
        );
        assert_eq!(std::fs::read_to_string(&a_path).unwrap(), "a1\n");
        assert_eq!(std::fs::read_to_string(&b_path).unwrap(), "b1\n");
        update_save_action_state(&window);
        assert!(
            window
                .lookup_action("save-all")
                .and_then(|a| a.downcast::<gtk::gio::SimpleAction>().ok())
                .is_some_and(|a| !a.is_enabled()),
            "Save All disabled when every tab is clean"
        );

        window.destroy();
    });
}

/// Every tooltip in `widget`'s subtree — where the preview states why an image is not
/// shown (`renderer::image_placeholder_tooltip`).
fn tooltips_under(widget: &gtk::Widget, out: &mut Vec<String>) {
    if let Some(tip) = widget.tooltip_text() {
        out.push(tip.to_string());
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        tooltips_under(&c, out);
        child = c.next_sibling();
    }
}

/// **A Save As into another folder re-resolves the preview's images there**
/// (Document-Identity CAM row 5, column B; TDD 14.3).
///
/// The preview resolves relative images against the document's folder when it renders,
/// and a save does not re-render, so the pane kept the OLD folder's picture — one now
/// outside the document's folder, which 14.3 would block — until something else redrew
/// it. MEASURED before the fix: no placeholder after the Save As, "Image not found" only
/// after a view-mode round trip. Mutation: removing the `rerender_for_new_folder` call
/// fails the second assertion.
#[gtktest::test]
fn save_as_into_another_folder_re_resolves_the_previews_images() {
    let dir = tempfile::tempdir().unwrap();
    crate::session::with_state_home_for_test(dir.path(), || {
        let (from, to) = (dir.path().join("from"), dir.path().join("to"));
        std::fs::create_dir_all(&from).unwrap();
        std::fs::create_dir_all(&to).unwrap();
        let picture =
            gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, 4, 4)
                .expect("allocate pixbuf");
        picture.fill(0xff_00_00_ff);
        picture
            .savev(from.join("pic.png"), "png", &[])
            .expect("save png");
        let md = "# Title\n\n![alt](pic.png)\n";
        let doc = from.join("doc.md");
        std::fs::write(&doc, md).unwrap();

        let app = test_app("saveasimagedir");
        let window = crate::window::new_window(&app, "IT", md, Some(&doc));
        let st = state(&window).expect("state");
        let placeholders = || {
            let mut out = Vec::new();
            if let Some(pane) = st.split.preview_scroller() {
                tooltips_under(pane.upcast_ref(), &mut out);
            }
            out
        };
        assert!(
            !crate::docio::settle(|| !placeholders().is_empty()),
            "precondition: the picture beside the document loads: {:?}",
            placeholders()
        );

        assert!(drive_save_as(&window, &st, to.join("doc.md")));
        assert!(
            crate::docio::settle(|| placeholders()
                .iter()
                .any(|t| t.starts_with("Image not found"))),
            "the new folder has no pic.png, so the preview must say so rather than keep \
             the old folder's picture: {:?}",
            placeholders()
        );
        window.destroy();
    });
}
