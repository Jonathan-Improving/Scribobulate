use super::*;
use crate::window::new_window;

/// Pump the main loop until `done` or a 2s bound elapses; reports whether it
/// converged. The snapshot write is genuinely async — GIO dispatches it to a
/// thread pool — so the completion arrives on a later main-context turn and
/// cannot be asserted synchronously. `crate::testpump::until_or_for` under
/// `Clock::Worker` (M31); `2_000 * 1ms` matches this function's old ceiling.
fn pump_until(done: impl FnMut() -> bool) -> bool {
    crate::testpump::until_or_for(
        crate::testpump::Clock::Worker,
        std::time::Duration::from_millis(2_000),
        done,
    )
}

fn swap_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let swap_dir = dir.join("scribobulate").join("swap");
    let Ok(entries) = std::fs::read_dir(swap_dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "swap"))
        .collect()
}

/// The invariant's positive half, end to end through the real widget: editing a
/// buffer must actually put a recoverable snapshot on disk.
///
/// A unit test on `sync_action` proves the decision; only this proves the decision is
/// wired to a live `GtkTextBuffer`, reaches the filesystem, and produces a file the
/// codec can read back. Those are different failure modes and POLICY requires both.
#[gtktest::test]
fn a_dirty_buffer_produces_a_readable_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    crate::session::with_state_home_for_test(dir.path(), || {
        let app = super::gtk_integration_tests::test_app("com.extollit.scribobulate.it.swapwrite");
        let win = new_window(&app, "IT", "original", None);
        let tab = winstate::state(&win).expect("the window has a tab");

        tab.editor_buf.set_text("original plus unsaved work");
        // Do not wait out the 3 s debounce in a test: the focus-loss flush is a
        // production path, so driving it here exercises real code rather than
        // reaching past it.
        flush_now(&tab);
        // Synchronise on the production in-flight gate, NOT on the file appearing.
        // Those are different moments — see the atomicity test below — and a test
        // that waits for the wrong one reads a half-written file.
        assert!(
            pump_until(|| !tab.swap.in_flight.get()),
            "the snapshot write must complete"
        );

        let files = swap_files(dir.path());
        assert_eq!(files.len(), 1, "one document, one snapshot: {files:?}");
        let bytes = std::fs::read(&files[0]).expect("readable");
        let (header, body) = crate::swapfile::decode(&bytes).expect("decodes");
        assert_eq!(body, "original plus unsaved work", "the buffer's own text");
        assert_eq!(
            header.doc_id,
            tab.doc_id(),
            "filed under this tab's identity"
        );
        assert!(
            header.untitled,
            "an unsaved document is flagged as untitled"
        );
    });
}

/// **Overwriting an existing snapshot never exposes a partial file** — the property
/// the whole mechanism leans on, measured rather than assumed.
///
/// GTK4Rs/AP-167 established from the GLib source that `replace_contents_async` is atomic
/// only under the right flags. This pins the behaviour we actually depend on, and it
/// also records the boundary that source read did not make obvious: the guarantee
/// covers **replacing**, not **creating**. A first-ever write streams into the
/// destination directly, so the file is observably 0 bytes for a moment; only once a
/// destination exists does GIO take the temp-and-rename path. A crash inside that
/// first window leaves a partial file, which the codec rejects as damaged rather than
/// mis-recovering — safe degradation, and the reason this asserts the replace case
/// specifically rather than pretending both are atomic.
#[gtktest::test]
fn overwriting_a_snapshot_never_exposes_a_partial_file() {
    let dir = tempfile::tempdir().unwrap();
    crate::session::with_state_home_for_test(dir.path(), || {
        let app = super::gtk_integration_tests::test_app("com.extollit.scribobulate.it.swapatomic");
        let win = new_window(&app, "IT", "original", None);
        let tab = winstate::state(&win).expect("the window has a tab");

        tab.editor_buf.set_text("first snapshot");
        flush_now(&tab);
        assert!(
            pump_until(|| !tab.swap.in_flight.get()),
            "first write lands"
        );
        let file = swap_files(dir.path()).pop().expect("a snapshot exists");
        let first_len = std::fs::read(&file).unwrap().len();
        assert!(
            first_len > 0,
            "precondition: the first snapshot has content"
        );

        // Overwrite with a longer payload, sampling the destination throughout.
        tab.editor_buf
            .set_text("a second snapshot, deliberately longer than the first one was");
        flush_now(&tab);
        let mut smallest = usize::MAX;
        for _ in 0..2_000 {
            if !tab.swap.in_flight.get() {
                break;
            }
            smallest = smallest.min(std::fs::read(&file).map(|b| b.len()).unwrap_or(0));
            glib::MainContext::default().iteration(false);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        let settled = std::fs::read(&file).unwrap();
        assert!(
            settled.len() > first_len,
            "the new snapshot is the longer one"
        );
        if smallest != usize::MAX {
            assert!(
                smallest >= first_len,
                "the destination shrank to {smallest} bytes mid-write (previous \
                 snapshot was {first_len}) — an overwrite must never expose a \
                 truncated file, or a crash mid-snapshot destroys the recovery it \
                 was taken for (GTK4Rs/AP-167)"
            );
        }
        crate::swapfile::decode(&settled).expect("the settled file decodes");
    });
}

/// `flush_now` promises to take the snapshot **and cancel the debounce it
/// replaces**. A `SourceId` that is dropped rather than removed leaves the timeout
/// armed, and it then fires a second snapshot of text already on disk.
///
/// The debounce is 3 s at ordinary document sizes, far too long to sit and wait for.
/// `next_delay_ms` returns 0 once the maximum-latency deadline has passed and
/// `request_snapshot` leaves an already-set deadline alone, so pre-expiring the
/// deadline arms the timer for 0 ms and a leak shows itself on the next iteration
/// instead of three seconds later.
///
/// Mutation: put `cancel_pending(tab)` back after a `pending.take()` guard in
/// `flush_now` and this fails - which is how the defect was confirmed.
#[gtktest::test]
fn flushing_cancels_the_debounce_it_replaces() {
    let dir = tempfile::tempdir().unwrap();
    crate::session::with_state_home_for_test(dir.path(), || {
        let app = super::gtk_integration_tests::test_app("com.extollit.scribobulate.it.swapflush");
        let win = new_window(&app, "IT", "original", None);
        let tab = winstate::state(&win).expect("the window has a tab");

        tab.swap
            .deadline
            .set(Some(glib::monotonic_time() - 1_000_000));
        tab.editor_buf.set_text("dirty enough to want a snapshot");
        assert!(
            !tab.swap.coalesced.get(),
            "precondition: nothing is coalesced before the flush"
        );

        flush_now(&tab);
        assert!(
            tab.swap.in_flight.get(),
            "precondition: the flush started a write of its own, so a second one is \
             attributable to the leaked debounce and nothing else"
        );

        // A leaked 0 ms debounce dispatches somewhere in here. Landing DURING the
        // flush own write sets `coalesced`; landing after it raises `in_flight` a
        // second time. Both are the write this flush was supposed to have cancelled,
        // so watch for either rather than betting on the ordering.
        let mut settled = false;
        let mut second_write = None;
        for _ in 0..400 {
            if tab.swap.coalesced.get() {
                second_write = Some("coalesced during the flush own write");
                break;
            }
            if settled && tab.swap.in_flight.get() {
                second_write = Some("started after the flush own write settled");
                break;
            }
            settled |= !tab.swap.in_flight.get();
            glib::MainContext::default().iteration(false);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(
            second_write.is_none(),
            "flush_now left its debounce armed: a second snapshot {} - the timeout \
             was dropped without being removed, so cancelling it did nothing",
            second_write.unwrap_or_default()
        );
    });
}

/// The invariant's negative half: editing *back* to the saved content removes the
/// snapshot, with nothing having been taught about undo specifically.
///
/// This is the case a two-rule design (delete on save, delete on discard) silently
/// gets wrong, which is why the invariant is expressed once rather than per path.
#[gtktest::test]
fn editing_back_to_the_baseline_removes_the_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    crate::session::with_state_home_for_test(dir.path(), || {
        let app = super::gtk_integration_tests::test_app("com.extollit.scribobulate.it.swapclean");
        let win = new_window(&app, "IT", "original", None);
        let tab = winstate::state(&win).expect("the window has a tab");

        tab.editor_buf.set_text("dirtied");
        flush_now(&tab);
        assert!(pump_until(|| !tab.swap.in_flight.get()), "the write lands");
        assert!(
            !swap_files(dir.path()).is_empty(),
            "precondition: the dirty document has a snapshot"
        );

        // Back to exactly the baseline — the document is clean again.
        tab.editor_buf.set_text("original");
        assert!(!tab.is_dirty(), "precondition: the document is clean again");
        // Drive the PRODUCTION path, not `sync_tab_swap` directly. Calling the
        // choke point by hand proves the function and says nothing about whether
        // anything calls it — the masking GTK4Rs/AP-78 warns about. Mutation-tested:
        // with the invariant unwired, the buffer-change path still deletes here (it
        // enforces the same rule from the other side), so the assertion that
        // actually pins the wiring is the discard-recovery test in `swaprecovery` —
        // recorded here because a mutation run going red is not by itself evidence
        // that THIS guard fired (GEP-11).
        refresh_dirty_status(&win);

        assert!(
            pump_until(|| swap_files(dir.path()).is_empty()),
            "a clean document may not have a snapshot: {:?}",
            swap_files(dir.path())
        );
    });
}

/// A snapshot is readable only by its owner.
///
/// It holds verbatim document text, including from documents the user has
/// deliberately made owner-only — so this asserts the mode rather than trusting that
/// the flag was passed, which is the assertion that would have caught the mode being
/// re-applied over ours on a later overwrite (GTK4Rs/AP-167).
#[gtktest::test]
fn a_snapshot_is_owner_only() {
    let dir = tempfile::tempdir().unwrap();
    crate::session::with_state_home_for_test(dir.path(), || {
        let app = super::gtk_integration_tests::test_app("com.extollit.scribobulate.it.swapmode");
        let win = new_window(&app, "IT", "original", None);
        let tab = winstate::state(&win).expect("the window has a tab");

        tab.editor_buf.set_text("secret");
        flush_now(&tab);
        assert!(pump_until(|| !swap_files(dir.path()).is_empty()), "written");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let files = swap_files(dir.path());
            let mode = std::fs::metadata(&files[0]).unwrap().permissions().mode();
            assert_eq!(
                mode & 0o077,
                0,
                "a swap file must not be group- or world-readable (mode {mode:o})"
            );
        }
        #[cfg(not(unix))]
        println!(
            "SKIPPED [TDD 22.13]: POSIX mode bits are not the privacy mechanism on \
             this platform; the state directory's ACL is (see session::create_state_dir)"
        );
    });
}

/// Discarding a dirty tab takes its snapshot with it, immediately.
///
/// The one deletion that cannot come through the dirtiness choke point — the tab is
/// still dirty as it is destroyed — so it is also the one a future refactor is most
/// likely to drop. Without it the next launch resurrects work the user explicitly
/// threw away.
#[gtktest::test]
fn discarding_a_dirty_tab_removes_its_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    crate::session::with_state_home_for_test(dir.path(), || {
        let app =
            super::gtk_integration_tests::test_app("com.extollit.scribobulate.it.swapdiscard");
        let win = new_window(&app, "IT", "original", None);
        let tab = winstate::state(&win).expect("the window has a tab");

        tab.editor_buf
            .set_text("work the user is about to throw away");
        flush_now(&tab);
        assert!(
            pump_until(|| !swap_files(dir.path()).is_empty()),
            "precondition: the dirty document has a snapshot"
        );

        discard_tab_swap(&tab);
        assert!(
            swap_files(dir.path()).is_empty(),
            "a discarded tab's snapshot must be gone immediately, not eventually: {:?}",
            swap_files(dir.path())
        );
        assert!(
            tab.is_dirty(),
            "and the tab is still dirty — which is exactly why the invariant cannot \
             be the mechanism here"
        );
    });
}

/// **A Discard wins over a snapshot write already in flight** (TDD 22.18). The write
/// is handed to GIO and its completion can only arrive on a later main-context turn,
/// so discarding in the same turn as the flush puts the Discard deterministically
/// between the write's start and its promote, the window a human click can hit on a
/// slow volume.
///
/// Mutation: make `finish_snapshot` pass `still_wanted = true` and the discarded text
/// is renamed into place after the delete.
#[gtktest::test]
fn a_discard_during_an_in_flight_write_leaves_no_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    crate::session::with_state_home_for_test(dir.path(), || {
        let app = super::gtk_integration_tests::test_app(
            "com.extollit.scribobulate.it.swapdiscardflight",
        );
        let win = new_window(&app, "IT", "original", None);
        let tab = winstate::state(&win).expect("the window has a tab");

        tab.editor_buf
            .set_text("work the user is about to throw away");
        flush_now(&tab);
        assert!(
            tab.swap.in_flight.get(),
            "precondition: the write is in flight"
        );
        discard_tab_swap(&tab);
        assert!(
            pump_until(|| !tab.swap.in_flight.get()),
            "the in-flight write must complete"
        );
        assert_eq!(
            swap_files(dir.path()),
            Vec::<std::path::PathBuf>::new(),
            "the write that was in flight across the Discard must not promote"
        );
        win.destroy();
    });
}

/// A window a test opens inside the state-home redirect is closed, with its dirty
/// tab's snapshot discarded, before the redirect lifts — so it cannot snapshot into
/// a later test's directory. On a failing test too, since the teardown is a `Drop`.
///
/// Mutation: make `close_windows_opened_since_for_test` return at once and the
/// window survives the redirect, holding its snapshot.
#[gtktest::test]
fn the_state_home_redirect_closes_what_the_test_opened() {
    let dir = tempfile::tempdir().unwrap();
    let before = super::toplevels_for_test().len();
    crate::session::with_state_home_for_test(dir.path(), || {
        let app =
            super::gtk_integration_tests::test_app("com.extollit.scribobulate.it.swapteardown");
        let win = new_window(&app, "IT", "original", None);
        let tab = winstate::state(&win).expect("the window has a tab");
        tab.editor_buf.set_text("unsaved work");
        flush_now(&tab);
        assert!(
            pump_until(|| !tab.swap.in_flight.get()),
            "the snapshot lands"
        );
        assert_eq!(
            swap_files(dir.path()).len(),
            1,
            "precondition: a snapshot exists"
        );
    });
    assert_eq!(
        super::toplevels_for_test().len(),
        before,
        "the window the test opened must be closed before the redirect lifts"
    );
    assert!(
        swap_files(dir.path()).is_empty(),
        "and its snapshot discarded rather than left for a later recovery"
    );
}

/// Run the production Save As write (`save::adopt_and_save`) to completion.
fn save_as(win: &ApplicationWindow, tab: &Rc<TabState>, path: std::path::PathBuf) -> bool {
    let outcome: Rc<std::cell::Cell<Option<bool>>> = Rc::default();
    let sink = Rc::clone(&outcome);
    let (win, tab) = (win.clone(), Rc::clone(tab));
    glib::MainContext::default().spawn_local(async move {
        sink.set(Some(
            super::super::save::adopt_and_save(&win, &tab, path).await,
        ));
    });
    assert!(
        crate::docio::settle(|| outcome.get().is_some()),
        "the Save As must complete"
    );
    outcome.get() == Some(true)
}

/// **A Save As retires the snapshot it was preceded by, though that snapshot carries the
/// OLD name** (TDD 22.3, "including under a new name"; Document-Identity CAM row 3,
/// columns A and B).
///
/// Opening the chooser deactivates the window, so the focus flush files a snapshot under
/// the name the document has *before* the save — `untitled-…` for column A. The save then
/// adopts the new path before the write, and a delete that recomputed the name from it
/// removed nothing, orphaning the snapshot; the next launch offered the pre-save text back
/// as a recovered document. MEASURED in the operator's own session before the fix.
/// Mutation: making `delete_snapshot` remove the recomputed name again fails both halves.
#[gtktest::test]
fn save_as_removes_the_snapshot_filed_under_the_previous_name() {
    let dir = tempfile::tempdir().unwrap();
    crate::session::with_state_home_for_test(dir.path(), || {
        let app = super::gtk_integration_tests::test_app("com.extollit.scribobulate.it.swapsaveas");
        let win = new_window(&app, "IT", "", None);
        let tab = winstate::state(&win).expect("the window has a tab");

        for (step, name) in [
            ("A: untitled adopts a path", "first.md"),
            ("B: re-point", "second.md"),
        ] {
            tab.editor_buf.set_text(&format!("unsaved before {name}"));
            flush_now(&tab);
            assert!(
                pump_until(|| !tab.swap.in_flight.get()),
                "{step}: the write lands"
            );
            assert_eq!(
                swap_files(dir.path()).len(),
                1,
                "{step}: precondition: one snapshot"
            );

            assert!(
                save_as(&win, &tab, dir.path().join(name)),
                "{step}: the save succeeds"
            );
            assert!(
                !tab.is_dirty(),
                "{step}: precondition: the saved document is clean"
            );
            assert!(
                pump_until(|| swap_files(dir.path()).is_empty()),
                "{step}: a saved document may leave no snapshot behind, under any name: {:?}",
                swap_files(dir.path())
            );
        }
        win.destroy();
    });
}

/// **A dirty document whose name changes keeps exactly one snapshot**: the next write,
/// filed under the new name, removes the one under the old name once it has landed
/// (Document-Identity CAM row 3, column B — a Rename permitted while dirty, or a failed
/// Save As handing its path back).
///
/// The path is changed by hand because no production path does this to a dirty document
/// today; the test pins `record_promoted`, which is what keeps that true the day one does.
/// Mutation: dropping the removal in `record_promoted` leaves both files.
#[gtktest::test]
fn a_snapshot_under_a_new_name_supersedes_the_one_under_the_old_name() {
    let dir = tempfile::tempdir().unwrap();
    crate::session::with_state_home_for_test(dir.path(), || {
        let doc = dir.path().join("before.md");
        std::fs::write(&doc, "original").unwrap();
        let app =
            super::gtk_integration_tests::test_app("com.extollit.scribobulate.it.swaprepoint");
        let win = new_window(&app, "IT", "original", Some(&doc));
        let tab = winstate::state(&win).expect("the window has a tab");

        tab.editor_buf.set_text("dirty under the old name");
        flush_now(&tab);
        assert!(
            pump_until(|| !tab.swap.in_flight.get()),
            "the first write lands"
        );
        let old = swap_files(dir.path());
        assert_eq!(old.len(), 1, "precondition: one snapshot");

        *tab.path.borrow_mut() = Some(dir.path().join("after.md"));
        tab.editor_buf.set_text("dirty under the new name");
        flush_now(&tab);
        assert!(
            pump_until(|| !tab.swap.in_flight.get()),
            "the second write lands"
        );

        let now = swap_files(dir.path());
        assert_eq!(now.len(), 1, "one dirty document, one snapshot: {now:?}");
        assert_ne!(now, old, "and it is the one under the new name");
        discard_tab_swap(&tab);
        win.destroy();
    });
}
