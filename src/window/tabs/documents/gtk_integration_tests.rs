use super::super::super::*;
use gtk::prelude::Cast;
use std::path::Path;

/// The toolbar's open-documents combo box must (a) share the ONE per-window
/// `documents_menu` GMenu with the menubar's View ▸ Documents submenu — so the
/// two surfaces can never list different documents — and (b) show the ACTIVE
/// document's name as its label, tracking it across a tab switch. Both are the
/// point of the feature: a second surface for the same fast-switch command, not
/// a parallel one that can drift.
#[gtktest::test]
fn documents_combo_shares_the_menu_model_and_labels_the_active_document() {
    let app = gtk::Application::new(
        Some("com.extollit.scribobulate.integrationtest.docscombo"),
        gtk::gio::ApplicationFlags::NON_UNIQUE,
    );
    app.register(gtk::gio::Cancellable::NONE)
        .expect("register (emits startup) before building any window");

    // A window whose sole tab has a backing path, so the combo shows a real
    // filename rather than "Untitled". `new_window` does not read the file (the
    // Markdown is passed in), so a non-existent path is fine for this test.
    let window = crate::window::new_window(
        &app,
        "IT",
        "# Alpha",
        Some(Path::new("/tmp/scrib-it/alpha.md")),
    );
    let chrome = winstate::chrome(&window).expect("chrome registered");

    // (a) ONE model, two surfaces: the combo's popup binds the very same GMenu
    // object the menubar submenu does — identity, not just an equal copy.
    let bound = chrome
        .documents_btn
        .menu_model()
        .expect("the combo's menu-model is bound after build_window");
    assert_eq!(
        bound.as_ptr() as *const (),
        chrome
            .documents_menu
            .upcast_ref::<gtk::gio::MenuModel>()
            .as_ptr() as *const (),
        "the combo must reuse the window's documents_menu, not a separate model"
    );

    // (b) The label reflects the active document. The initial rebuild is
    // deferred to idle (GTK4Rs/AP-76), so drive the same refresh synchronously here —
    // this is exactly what the idle would call.
    super::refresh_documents_button(&window);
    assert_eq!(
        chrome.documents_btn.label().map(|s| s.to_string()),
        Some("alpha.md".to_string()),
        "the combo labels the sole (active) document"
    );

    // Add a second document and switch to it (defer = false → switch-page fires
    // synchronously → `refresh_tab_surfaces` updates the label). The combo must
    // now name the newly-active document.
    crate::window::create_tab_in_window(
        &window,
        "# Beta",
        Some(Path::new("/tmp/scrib-it/beta.md")),
        false,
        false,
    )
    .expect("second tab created");
    assert_eq!(
        chrome.documents_btn.label().map(|s| s.to_string()),
        Some("beta.md".to_string()),
        "switching to the new tab retargets the combo label"
    );

    // Switch back to the first document: the label follows the active tab.
    let first = winstate::tabs_for_window(&window)
        .into_iter()
        .find(|t| {
            t.path
                .borrow()
                .as_deref()
                .and_then(Path::file_name)
                .is_some_and(|n| n == "alpha.md")
        })
        .expect("first tab still present");
    chrome.tabs.focus_page(&first.content_box);
    assert_eq!(
        chrome.documents_btn.label().map(|s| s.to_string()),
        Some("alpha.md".to_string()),
        "switching back retargets the combo label to the first document"
    );

    window.destroy();
}

/// **Opening a file into a reused blank tab must relabel every derived-view
/// surface, not just the window title (TDD 1.5, TDD 15.18).**
///
/// Drives the REAL reuse path: a `gtk::Application` built exactly as production
/// builds it (`HANDLES_OPEN`, the real `setup_app` wiring) with one window
/// holding a single blank/untouched tab (`File ▸ New Document`'s state) is fed a
/// real file through `app.open(&[file], "interactive")` — the same entry point
/// `File ▸ Open`'s dialog response calls. That reaches `openbatch::build_opened_batch`'s
/// `find_reusable_blank_tab` branch, which loads the file into the blank tab
/// **in place** rather than opening a new tab or window.
///
/// Three surfaces are asserted, and none is implied by another: the tab-strip
/// label (`TabView::tab_label_text`), the Documents combo's button label, and the
/// View ▸ Documents menu's sole item label. All three must name the opened file,
/// not "Untitled" — the label `load_source_into_window` used to leave behind by
/// retitling the window with a bare `set_title` instead of going through
/// `update_window_title` (the one entry point that also relabels the strip and
/// schedules the Documents-menu rebuild).
#[gtktest::test]
fn opening_a_file_into_a_reused_blank_tab_relabels_every_surface() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("reopened.md");
    std::fs::write(&file, "# Reopened\n").unwrap();

    // Built the way production builds it — HANDLES_OPEN plus the real
    // `setup_app` wiring — so this drives the actual `open` handler
    // (`app::openbatch::on_open`) rather than calling it directly, which its
    // `pub(super)` visibility does not allow from here in any case.
    let app = gtk::Application::new(
        Some("com.extollit.scribobulate.integrationtest.reuseblanktab"),
        gtk::gio::ApplicationFlags::HANDLES_OPEN | gtk::gio::ApplicationFlags::NON_UNIQUE,
    );
    crate::app::setup_app(&app);
    app.register(gtk::gio::Cancellable::NONE)
        .expect("register before building a window");

    // File ▸ New Document's state: one window, one blank/untouched tab, no
    // backing path.
    let window = crate::window::new_window(&app, "Scribobulate", crate::app::WELCOME, None);
    crate::testpump::until(
        crate::testpump::Clock::Idle,
        "the new window becomes active",
        || app.active_window().is_some(),
    );
    // Let the coalesced Documents-menu rebuild `new_window`'s own construction
    // schedules (GTK4Rs/AP-76) actually dispatch and settle on "Untitled" before
    // `File ▸ Open` runs — a real user never clicks Open in the same main-loop
    // turn as New Document, and skipping this drain lets that STILL-PENDING
    // construction-time idle mask the defect under test: it would fire only
    // during the wait below, by which point the reuse has already landed, so it
    // would rebuild against the CORRECT state and hide a `load_source_into_window`
    // that never re-schedules it.
    crate::testpump::drain_for(
        crate::testpump::Clock::Idle,
        std::time::Duration::from_millis(300),
    );
    let tabs_before = winstate::tabs_for_window(&window);
    assert_eq!(tabs_before.len(), 1, "precondition: exactly one tab");
    assert!(
        tabs_before[0].path.borrow().is_none(),
        "precondition: the sole tab is blank/pathless"
    );

    // The real entry point `File ▸ Open`'s dialog response calls
    // (`appactions::add_open_action`) — the "interactive" hint is what selects
    // the blank-tab-reuse branch in `build_opened_batch`.
    app.open(&[gtk::gio::File::for_path(&file)], "interactive");

    // The read runs off the main thread (`docio::read_document`); wait for the
    // reuse to land rather than assuming it has by the time `open` returns.
    assert!(
        crate::docio::settle(|| {
            winstate::tabs_for_window(&window).len() == 1
                && state(&window)
                    .and_then(|st| st.path.borrow().clone())
                    .as_deref()
                    == Some(file.as_path())
        }),
        "the file must load into the existing blank tab in place — no new tab, \
             no new window"
    );
    // Flush the coalesced Documents-menu rebuild idle (GTK4Rs/AP-76) — `open`'s
    // own async pass never yields back to the main loop once the read is in
    // hand, but the deferred idle it schedules needs one further turn.
    crate::testpump::drain_for(
        crate::testpump::Clock::Idle,
        std::time::Duration::from_millis(300),
    );

    let chrome = winstate::chrome(&window).expect("chrome registered");
    let tab = state(&window).expect("the reused tab is still active");

    assert_eq!(
        chrome.tabs.tab_label_text(&tab.content_box),
        Some("reopened.md".to_string()),
        "the tab strip must show the opened file's name, not the blank tab's \
             stale label"
    );
    assert_eq!(
        chrome.documents_btn.label().map(|s| s.to_string()),
        Some("reopened.md".to_string()),
        "the toolbar Documents combo must name the opened file"
    );
    assert_eq!(chrome.documents_menu.n_items(), 1, "still a single tab");
    assert_eq!(
        chrome
            .documents_menu
            .item_attribute_value(0, "label", None)
            .and_then(|v| v.str().map(str::to_string)),
        Some("reopened.md".to_string()),
        "the View ▸ Documents menu must name the opened file, not \"Untitled\""
    );

    window.destroy();
}

/// **The window title names the ACTIVE document and counts the others (TDD
/// 15.7).** Three claims, none implied by the others:
///
/// 1. A lone tab is named with no parenthetical at all.
/// 2. Opening a second document adds `(+1 document)` — singular — to the title.
/// 3. A plain tab SWITCH re-aims the title at the newly active document. This is
///    the one the old count-only title had no way to get wrong, and the reason
///    `refresh_tab_surfaces` now retitles: nothing else fires on a switch, so
///    without that call the title would name whichever document happened to be
///    active when the tab SET last changed and then quietly stop tracking.
///
/// Asserted against `window.title()` — the property the desktop actually reads —
/// and against literal strings rather than a second call to the formula: a test
/// that recomputes the formula agrees with any change to it, including a wrong
/// one (GTK4Rs/AP-160's "makes the test agree with itself"). The one-shared-formula
/// claim is `save.rs`'s to make; this test's job is the *content*.
#[gtktest::test]
fn window_title_names_the_active_document_and_counts_the_others() {
    let app = gtk::Application::new(
        Some("com.extollit.scribobulate.integrationtest.wintitle"),
        gtk::gio::ApplicationFlags::NON_UNIQUE,
    );
    app.register(gtk::gio::Cancellable::NONE)
        .expect("register (emits startup) before building any window");

    let window = crate::window::new_window(
        &app,
        "IT",
        "# Alpha",
        Some(Path::new("/tmp/scrib-it/alpha.md")),
    );
    assert_eq!(
        window.title().map(|s| s.to_string()),
        Some("alpha.md — Scribobulate".to_string()),
        "a lone document is named on its own — no sibling count"
    );

    // A second document, switched to on creation (defer = false → switch-page
    // fires synchronously). Singular: one other document, not "(+1 documents)".
    crate::window::create_tab_in_window(
        &window,
        "# Beta",
        Some(Path::new("/tmp/scrib-it/beta.md")),
        false,
        false,
    )
    .expect("second tab created");
    assert_eq!(
        window.title().map(|s| s.to_string()),
        Some("beta.md (+1 document) — Scribobulate".to_string()),
        "the title names the newly active document and counts the other one"
    );

    // A third, then switch back to the first: the title must follow the ACTIVE
    // tab, and the count must follow the tab SET independently of it.
    crate::window::create_tab_in_window(
        &window,
        "# Gamma",
        Some(Path::new("/tmp/scrib-it/gamma.md")),
        false,
        false,
    )
    .expect("third tab created");
    assert_eq!(
        window.title().map(|s| s.to_string()),
        Some("gamma.md (+2 documents) — Scribobulate".to_string()),
        "two other documents pluralise the count"
    );

    let chrome = winstate::chrome(&window).expect("chrome registered");
    let first = winstate::tabs_for_window(&window)
        .into_iter()
        .find(|t| {
            t.path
                .borrow()
                .as_deref()
                .and_then(Path::file_name)
                .is_some_and(|n| n == "alpha.md")
        })
        .expect("first tab still present");
    chrome.tabs.focus_page(&first.content_box);
    assert_eq!(
        window.title().map(|s| s.to_string()),
        Some("alpha.md (+2 documents) — Scribobulate".to_string()),
        "a plain tab switch re-aims the title at the now-active document, \
             keeping the same sibling count"
    );

    window.destroy();
}
