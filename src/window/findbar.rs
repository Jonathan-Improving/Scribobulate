//! Find / replace bar signal wiring: open/close, Escape, next/prev, Enter and
//! Shift+Enter, search-changed, occurrences-count, Replace and Replace All. The
//! bar widgets are built in `chrome.rs` (shared chrome, `WindowChrome`); the
//! search engine (`GtkSourceSearchContext`/`SearchSettings`) is per-tab
//! (`TabState.search_context`/`search_settings` — per-tab).
//!
//! Every closure below that acts on the search engine fetches it fresh via
//! `state(window)` rather than capturing a fixed clone, so it always operates
//! on whichever tab is *currently* active — with one tab (true through the
//! end of Phase 2) this always resolves to the same object as before, but the
//! call sites are already correct for Phase 3's second tab. The one exception
//! is the `occurrences-count` notify connection, which is inherently bound to
//! one specific `GtkSourceSearchContext` instance at connect time (a GObject
//! signal can't "dynamically" listen to "whichever is active") — it is wired
//! once per tab, at tab-creation time, to that tab's own context, which is
//! already correct: it only ever reports on its own tab's search activity.
//!
//! QA round-1 H2: it resolves its target window AND label fresh from the tab's own `content_box` on every fire (`tabs::resolve_tab_window` + `winstate::chrome`) instead of a captured `window`/`match_count_label` pair, which would go stale the moment the tab moves to a different window.
use super::*;

/// Wire the window-shared find bar widgets carried in `chrome`. Every closure
/// looks the active tab's search engine up fresh via `state(window)`, so this
/// takes no per-tab `search_context` — a tab's own `occurrences-count` handler
/// is wired per-tab in `assemble_tab_core`, not here (this bar is built once per
/// window, not once per tab).
pub(super) fn wire_find_bar(window: &ApplicationWindow, chrome: &Chrome) {
    let find_bar_revealer = &chrome.find_bar_revealer;
    let find_entry = &chrome.find_entry;
    let replace_row = &chrome.replace_row;
    let match_count_label = &chrome.match_count_label;
    let replace_entry = &chrome.replace_entry;
    let replace_btn = &chrome.replace_btn;
    let replace_all_btn = &chrome.replace_all_btn;
    let close_find_btn = &chrome.close_find_btn;
    let find_prev_btn = &chrome.find_prev_btn;
    let find_next_btn = &chrome.find_next_btn;
    let find_bar = &chrome.find_bar;
    let find_history_btn = &chrome.find_history_btn;
    let replace_history_btn = &chrome.replace_history_btn;
    // ── win.find / win.find-replace actions ──────────────────────────────────
    // Both open the revealer; find-replace additionally reveals the replace row.
    {
        let open_find_bar: Rc<dyn Fn(bool)> = Rc::new({
            let win = window.downgrade();
            let fr = find_bar_revealer.clone();
            let fe = find_entry.clone();
            let rr = replace_row.clone();
            let mc = match_count_label.clone();
            move |replace_mode: bool| {
                let Some(w) = win.upgrade() else { return };
                if let Some(st) = state(&w) {
                    st.find_replace_mode.set(replace_mode);
                    // Replace row is only usable in edit/split; disable in preview.
                    let mode = current_mode(&w);
                    let editor_visible = mode.is_editor_visible();
                    rr.set_visible(replace_mode);
                    rr.set_sensitive(editor_visible);
                    if replace_mode && !editor_visible {
                        crate::a11y::describe(&rr, Some("Replace is unavailable in preview mode."));
                    } else {
                        crate::a11y::describe(&rr, None);
                    }
                    // Show the match count label only if there is a search string.
                    mc.set_visible(!st.chrome().find_entry.text().is_empty());
                    st.search_context.set_highlight(true);
                }
                fr.set_reveal_child(true);
                fe.grab_focus();
                // Select all text in the entry so the next keystroke replaces it.
                fe.select_region(0, -1);
                // In preview mode: re-apply highlights if the bar is reopened with
                // existing text. `search-changed` only fires on a text *change*, so
                // if the user dismisses and reopens without editing the search term
                // the signal is silent and the highlights stay absent (the in-place
                // clear on dismiss removed them). Mirror the search-changed logic here.
                let FindTarget::Preview(view) = find_target(&w) else {
                    return;
                };
                let text = fe.text();
                let Some(st) = state(&w) else { return };
                if text.is_empty() {
                    return;
                }
                resync_preview_find(&w, &st, &view, text.as_str());
            }
        });

        let find_action = SimpleAction::new("find", None);
        {
            let ofc = Rc::clone(&open_find_bar);
            find_action.connect_activate(move |_, _| ofc(false));
        }
        window.add_action(&find_action);

        let find_replace_action = SimpleAction::new("find-replace", None);
        {
            let ofc = Rc::clone(&open_find_bar);
            find_replace_action.connect_activate(move |_, _| ofc(true));
        }
        window.add_action(&find_replace_action);
    }

    // ── Close find bar (button, ancestor Escape, and GtkSearchEntry's own
    //    Escape keybinding all funnel through this one closure — single
    //    source of truth, see ANTI-PATTERNS.md re: GtkSearchEntry Escape). ──
    let close_find_bar: Rc<dyn Fn()> = Rc::new({
        let win = window.downgrade();
        let fr = find_bar_revealer.clone();
        move || {
            fr.set_reveal_child(false);
            let Some(w) = win.upgrade() else { return };
            if let Some(st) = state(&w) {
                st.search_context.set_highlight(false);
            }
            clear_preview_highlight(&w);
            if let Some(st) = state(&w) {
                // Return focus to the editor (or let it stay on preview).
                let mode = current_mode(&w);
                if mode.is_editor_visible() {
                    st.editor.grab_focus();
                }
            }
        }
    });

    {
        let cfb = Rc::clone(&close_find_bar);
        close_find_btn.connect_clicked(move |_| cfb());
    }

    // ── Escape key in find bar ────────────────────────────────────────────────
    // Catches Escape bubbling up from any plain widget in the bar (e.g.
    // replace_entry, a bare GtkEntry). It does NOT catch Escape from
    // find_entry — see the stop-search connection below.
    {
        let cfb = Rc::clone(&close_find_bar);
        let key_ctrl = gtk::EventControllerKey::new();
        key_ctrl.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                cfb();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        find_bar.add_controller(key_ctrl);
    }

    // ── Escape key in find_entry specifically ─────────────────────────────────
    // find_entry is a GtkSearchEntry, which has its own class keybinding
    // (GDK_KEY_Escape -> "stop-search") that fires and stops propagation
    // while the entry itself has focus — the ancestor find_bar's
    // EventControllerKey above never sees the event. Confirmed against GTK
    // source (gtksearchentry.c: gtk_widget_class_add_binding_signal(...,
    // GDK_KEY_Escape, 0, "stop-search", ...)). Hook the signal the widget
    // actually provides for this instead of fighting the binding.
    {
        let cfb = Rc::clone(&close_find_bar);
        find_entry.connect_stop_search(move |_| cfb());
    }

    // ── Find-next / find-prev buttons ─────────────────────────────────────────
    find_next_btn.connect_clicked(glib::clone!(
        #[weak(rename_to = w)]
        window,
        move |_| {
            let Some(st) = state(&w) else { return };
            record_committed_query(&st);
            find_step(&w, &st.search_context, SearchDir::Forward);
        }
    ));
    find_prev_btn.connect_clicked(glib::clone!(
        #[weak(rename_to = w)]
        window,
        move |_| {
            let Some(st) = state(&w) else { return };
            record_committed_query(&st);
            find_step(&w, &st.search_context, SearchDir::Backward);
        }
    ));

    // ── Enter / Shift+Enter in find_entry ────────────────────────────────────
    find_entry.connect_activate(glib::clone!(
        #[weak(rename_to = w)]
        window,
        move |_| {
            let Some(st) = state(&w) else { return };
            record_committed_query(&st);
            find_step(&w, &st.search_context, SearchDir::Forward);
        }
    ));

    // ── Shift+Enter in find_entry (find previous) ─────────────────────────────
    {
        let win = window.downgrade();
        let key_ctrl2 = gtk::EventControllerKey::new();
        key_ctrl2.connect_key_pressed(move |_, key, _, mods| {
            if key == gtk::gdk::Key::Return && mods.contains(gtk::gdk::ModifierType::SHIFT_MASK) {
                if let Some(w) = win.upgrade() {
                    if let Some(st) = state(&w) {
                        record_committed_query(&st);
                        find_step(&w, &st.search_context, SearchDir::Backward);
                    }
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        find_entry.add_controller(key_ctrl2);
    }

    // ── Search-changed: update search settings and match count ────────────────
    find_entry.connect_search_changed(glib::clone!(
        #[weak(rename_to = w)]
        window,
        move |entry| {
            let Some(st) = state(&w) else { return };
            // Remember this tab's own query (operator decision Q13) so switching
            // away and back repopulates it rather than showing another tab's term.
            *st.find_query.borrow_mut() = entry.text().to_string();
            refresh_find(&w, &st);
        }
    ));

    // ── win.find-match-case / win.find-whole-word / win.find-regex ────────────
    // The three match options, one stateful boolean action each. **Uncommon commands**
    // (CAM § Uncommon commands): an Edit-menu item and a find-bar toggle, no toolbar
    // section and no context-menu entry — they qualify a query that only exists while
    // the bar is open.
    //
    // One action per option is what makes the find-bar toggle and the menu item the
    // same control rather than two that have to be kept in step (POLICY "One action per
    // command"). The option itself lives on the TAB, not on the action: the action's
    // state is a mirror of it, resynced on a tab switch exactly as `show-unsafe-images`
    // is.
    for (name, accessor) in FindOptions::ACTIONS {
        super::viewactions::register_bool_action(window, name, false, move |window, on| {
            let Some(st) = state(window) else { return };
            let mut options = st.find_options.get();
            accessor.set(&mut options, on);
            st.find_options.set(options);
            refresh_find(window, &st);
        });
    }

    // ── win.find-in-selection ────────────────────────────────────────────────
    // The fourth toggle, and the one that is NOT a match option (`FindOptions` says
    // why): it does not change what counts as a match, it bounds where matching is
    // applied — and it is a held reference into the document rather than a boolean, so
    // it is neither persisted nor carried in the same struct.
    //
    // It scopes FINDING. Replace is then an action on what finding produced, which is
    // why this is not a replace feature and is not editor-only.
    let in_selection =
        super::viewactions::register_bool_action(window, FIND_IN_SELECTION, false, |window, on| {
            let Some(st) = state(window) else { return };
            if on {
                capture_find_scope(window, &st);
            } else {
                *st.find_scope.borrow_mut() = None;
            }
            refresh_find(window, &st);
        });
    // Nothing is selected in a freshly built window, so the control starts unavailable
    // and the selection wiring raises it.
    in_selection.set_enabled(false);

    // ── The two history drop-downs ───────────────────────────────────────────
    // Each field's recent entries, most recent first. **Built on demand**, not resynced:
    // the buttons are the WINDOW's and the histories are the TAB's, so a model built
    // once and refreshed on a tab switch is one more mirror to keep true. A
    // `create_popup_func` runs at the moment the reader presses the button, which makes
    // "the active tab's own list" true by construction rather than by upkeep.
    for (btn, field) in [
        (find_history_btn, HistoryField::Find),
        (replace_history_btn, HistoryField::Replace),
    ] {
        btn.set_create_popup_func(glib::clone!(
            #[weak(rename_to = w)]
            window,
            move |btn| {
                btn.set_menu_model(state(&w).map(|st| build_history_menu(&st, field)).as_ref());
            }
        ));
    }

    // One parameterised action for both drop-downs' rows: the chosen text is the
    // target, so a row is not a closure and the menu can be rebuilt freely.
    let pick = SimpleAction::new(PICK_HISTORY, Some(glib::VariantTy::STRING));
    pick.connect_activate(glib::clone!(
        #[weak(rename_to = w)]
        window,
        move |_, value| {
            let Some(chosen) = value.and_then(|v| v.get::<String>()) else {
                return;
            };
            let Some((field, text)) = HistoryField::split_target(&chosen) else {
                log::error!("find: a history row carried an unreadable target");
                return;
            };
            let Some(st) = state(&w) else { return };
            let chrome = st.chrome();
            match field {
                // Choosing a search term searches for it immediately — the reader
                // opened the list to get back to a search, not to fill a box.
                HistoryField::Find => {
                    chrome.find_entry.set_text(text);
                    record_committed_query(&st);
                    refresh_find(&w, &st);
                }
                // A replacement is not an action on its own; it fills the field and
                // waits, exactly as typing it would.
                HistoryField::Replace => chrome.replace_entry.set_text(text),
            }
        }
    ));
    window.add_action(&pick);

    // ── Replace button ────────────────────────────────────────────────────────
    let re = replace_entry.clone();
    replace_btn.connect_clicked(glib::clone!(
        #[weak(rename_to = w)]
        window,
        move |_| {
            let Some(st) = state(&w) else { return };
            record_committed_query(&st);
            record_committed_replacement(&st);
            replace_current_match(&w, &st, re.text().as_str());
        }
    ));

    // ── Replace All button ────────────────────────────────────────────────────
    let re = replace_entry.clone();
    replace_all_btn.connect_clicked(glib::clone!(
        #[weak(rename_to = w)]
        window,
        move |_| {
            let Some(st) = state(&w) else { return };
            record_committed_query(&st);
            record_committed_replacement(&st);
            replace_all_matches(&w, &st, re.text().as_str());
        }
    ));
}

/// The `win.` action a history row activates, carrying the chosen text as its target.
///
/// One action for both drop-downs rather than one each: a row's payload is the text,
/// and which field it came from is a prefix on that text rather than a second action
/// name to keep in step with a second menu builder.
pub(crate) const PICK_HISTORY: &str = "pick-find-history";

/// Which field a history belongs to.
///
/// The `win.` action's target has to say, because one action serves both drop-downs and
/// a bare string cannot. Encoded as a one-character prefix on the text rather than as a
/// tuple variant: a `GAction` target is a `GVariant`, a string is the cheapest shape
/// that survives a menu model, and the entries themselves are arbitrary text so no
/// separator is safe unless it is at a FIXED position.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum HistoryField {
    Find,
    Replace,
}

impl HistoryField {
    /// This field's marker character. Never a separator to split on — the entry may
    /// contain anything — only the first character of the target.
    fn marker(self) -> char {
        match self {
            HistoryField::Find => 'f',
            HistoryField::Replace => 'r',
        }
    }

    /// The action target for `entry` in this field.
    fn target(self, entry: &str) -> String {
        format!("{}{entry}", self.marker())
    }

    /// Read a target back. `None` for anything that does not begin with a known marker,
    /// which is a target this application did not write.
    fn split_target(target: &str) -> Option<(Self, &str)> {
        let mut chars = target.chars();
        let field = match chars.next()? {
            'f' => HistoryField::Find,
            'r' => HistoryField::Replace,
            _ => return None,
        };
        Some((field, chars.as_str()))
    }

    /// This field's history on `st`, cloned out — never borrowed across a caller that
    /// then touches GTK (ScrAP-53).
    fn history(self, st: &Rc<TabState>) -> crate::window::FindHistory {
        match self {
            HistoryField::Find => st.find_history.borrow().clone(),
            HistoryField::Replace => st.replace_history.borrow().clone(),
        }
    }
}

/// Build the drop-down's model from `st`'s own history for `field`.
///
/// Rows are labelled by [`crate::window::row_label`] — shortened and single-lined,
/// because an entry is a string the reader typed — while the action target carries the
/// entry VERBATIM, so choosing a shortened row still searches for the whole thing.
fn build_history_menu(st: &Rc<TabState>, field: HistoryField) -> gtk::gio::Menu {
    let menu = gtk::gio::Menu::new();
    for entry in field.history(st).entries() {
        let item = gtk::gio::MenuItem::new(Some(&crate::window::row_label(entry)), None);
        item.set_action_and_target_value(
            Some(&format!("win.{PICK_HISTORY}")),
            Some(&field.target(entry).to_variant()),
        );
        menu.append_item(&item);
    }
    menu
}

/// Record the find field's current text as a committed search term.
///
/// **Committed, not typed.** The find field searches as you type, so every prefix of
/// every query is a search that happened; a history fed from `search-changed` is the
/// reader's last query once per keystroke and pushes everything else off the end. The
/// commit points are the ones where the reader has said "this one": Enter, Next, Prev,
/// Replace, Replace All, and choosing an entry from the drop-down itself.
pub(super) fn record_committed_query(st: &Rc<TabState>) {
    let text = st.chrome().find_entry.text();
    st.find_history.borrow_mut().record(text.as_str());
    sync_history_buttons(st);
}

/// The replacement-field counterpart, recorded at the two points a replacement is
/// actually applied.
pub(super) fn record_committed_replacement(st: &Rc<TabState>) {
    let text = st.chrome().replace_entry.text();
    st.replace_history.borrow_mut().record(text.as_str());
    sync_history_buttons(st);
}

/// A drop-down with nothing to offer is insensitive. Called wherever a history changes
/// or a different tab's becomes the active one — a button that opens an empty menu is
/// worse than one that says it has nothing.
pub(super) fn sync_history_buttons(st: &Rc<TabState>) {
    let chrome = st.chrome();
    chrome
        .find_history_btn
        .set_sensitive(!st.find_history.borrow().is_empty());
    chrome
        .replace_history_btn
        .set_sensitive(!st.replace_history.borrow().is_empty());
}

/// The `win.` action carrying **search in selection**.
///
/// A named constant rather than a literal because four surfaces spell it — the action
/// registration, the find-bar toggle's `action-name`, the Edit-menu item and the
/// sensitivity update — and an action name is a string no compiler checks. The three
/// match options get the same treatment from `FindOptions::ACTIONS`; this one is not in
/// that table because it is not one of them.
pub(crate) const FIND_IN_SELECTION: &str = "find-in-selection";

/// Capture the passage the reader has selected in whichever pane they are looking at.
///
/// The two panes index different spaces and so capture differently — see `FindScope`.
/// Capturing nothing leaves the toggle on with no bound, which would read as "in
/// selection, and everything is in the selection"; the action is insensitive without a
/// selection precisely so that state is unreachable.
fn capture_find_scope(window: &ApplicationWindow, st: &Rc<TabState>) {
    let captured = match find_target(window) {
        FindTarget::Editor => st.editor_buf.selection_bounds().map(|(start, end)| {
            // LEFT gravity on the start and RIGHT on the end, so text inserted at
            // either boundary lands INSIDE the passage rather than escaping it. Replace
            // All inserts at exactly those boundaries when the first or last match is
            // flush with them.
            FindScope::Editor {
                start: st.editor_buf.create_mark(None, &start, true),
                end: st.editor_buf.create_mark(None, &end, false),
            }
        }),
        FindTarget::Preview(view) => view.buffer().selection_bounds().map(|(start, end)| {
            FindScope::Preview(PreviewScope {
                key: RenderKey {
                    view_serial: view.instance_serial(),
                    generation: view.render_generation(),
                },
                start: start.offset(),
                end: end.offset(),
            })
        }),
        FindTarget::PreviewUnresolved => None,
    };
    *st.find_scope.borrow_mut() = captured;
}

/// Drop the captured passage and untick the toggle, from a path that discovered the
/// bound no longer resolves.
///
/// `set_action_state`, never `change_state`: the handler would clear a bound this has
/// already cleared and re-run the search that is in the middle of discovering it — and
/// the caller is `find`'s own scope resolution, so that is a re-entrant search.
pub(super) fn release_find_scope(window: &ApplicationWindow, st: &Rc<TabState>) {
    *st.find_scope.borrow_mut() = None;
    set_action_state(window, FIND_IN_SELECTION, &false.to_variant());
    update_find_scope_sensitivity(window);
}

/// Whether the in-selection control is usable: there is a passage to capture, or one is
/// already captured so the reader can let it go.
///
/// **The one option whose availability depends on the document rather than the query**,
/// which is why it has a sensitivity rule at all and the other three do not. Called
/// from every place a selection can change — the same three hooks `win.copy` is driven
/// from, since a selection is a selection.
pub(crate) fn update_find_scope_sensitivity(window: &ApplicationWindow) {
    let Some(st) = state(window) else { return };
    let captured = st.find_scope.borrow().is_some();
    let selectable = match find_target(window) {
        FindTarget::Editor => st.editor_buf.has_selection(),
        FindTarget::Preview(view) => view.buffer().has_selection(),
        FindTarget::PreviewUnresolved => false,
    };
    let enabled = captured || selectable;
    set_action_enabled(window, FIND_IN_SELECTION, enabled);
    crate::a11y::describe(
        &st.chrome().find_entry,
        (!enabled).then_some("Select a passage to confine the search to it."),
    );
}

/// Re-run the active tab's search under its current query AND options, and put the
/// outcome in the readout.
///
/// **One function for two triggers that are the same event.** Typing in the field and
/// ticking *match case* both change what the reader is asking for; nothing downstream
/// can tell them apart, and the moment they were two code paths one of them was going to
/// forget to push the options onto the editor's engine — which shows up as the editor
/// pane ignoring a toggle the preview pane honours.
///
/// Pushes the query and the options onto `SearchSettings` unconditionally, even in
/// pure-preview mode, so a later switch to edit or split finds the editor's engine
/// already asking the same question rather than the last one it was told.
pub(super) fn refresh_find(window: &ApplicationWindow, st: &Rc<TabState>) {
    let chrome = st.chrome();
    let label = &chrome.match_count_label;
    let text = chrome.find_entry.text();
    push_options_to_engine(st);
    // **The query the editor's engine is given is not always the query the reader
    // typed.** For a whole-word REGULAR EXPRESSION the application wraps it — see
    // `matcher::editor_pattern` — because GtkSourceView's own wrapper is ungrouped and
    // binds each `\b` to one branch of an alternation. Every other case passes through
    // unchanged.
    let pattern = crate::window::find::editor_pattern(text.as_str(), st.find_options.get());
    st.search_settings
        .set_search_text((!pattern.is_empty()).then_some(pattern.as_str()));
    label.set_visible(!text.is_empty());
    // The step cursor indexes a list that has just been replaced, whichever pane owns
    // it, so the next Next/Prev starts from the top.
    st.find_cursor.set(FindCursor::None);
    // Pure-preview mode: highlight the preview buffer (the editor engine can't, and the
    // editor isn't visible). Otherwise the source context's occurrences-count
    // notification refreshes the label.
    match find_target(window) {
        FindTarget::Preview(view) => resync_preview_find(window, st, &view, text.as_str()),
        FindTarget::Editor => update_editor_readout(st, 0),
        // Deliberately NOT the editor arm: in pure-preview mode the editor's
        // occurrence count describes a buffer the user cannot see, so showing
        // it would be a confidently wrong number rather than a missing one.
        FindTarget::PreviewUnresolved => set_match_label(label, 0, 0),
    }
}

/// Point the three option GActions at `st`'s own options, without re-entering their
/// handlers.
///
/// The options are the TAB's; the actions are the WINDOW's. Every path that makes a
/// different tab the active one therefore owes this — a tab switch, and a session
/// restore, which is a tab becoming active having never been switched to. Without it
/// the toggles and the Edit-menu ticks describe whichever tab set them last, which is
/// the `show-unsafe-images` lying-mirror defect (`window::tabs::switch`) in a second
/// place.
///
/// `set_state`, never `change_state`: this is a resync onto state that is already
/// correct, and running the handlers would re-run the search for a tab that may not
/// even have its preview built yet.
pub(super) fn adopt_find_options(window: &ApplicationWindow, st: &Rc<TabState>) {
    let options = st.find_options.get();
    for (name, accessor) in FindOptions::ACTIONS {
        set_action_state(window, name, &accessor.get(&options).to_variant());
    }
    // The scope is a tab's too, and its toggle is the window's, so it takes the same
    // resync — and its sensitivity, which the other three do not have, has to be
    // recomputed against whatever the newly active tab has selected.
    set_action_state(
        window,
        FIND_IN_SELECTION,
        &st.find_scope.borrow().is_some().to_variant(),
    );
    update_find_scope_sensitivity(window);
    push_options_to_engine(st);
}

/// Push this tab's options onto its own `GtkSourceSearchSettings`.
///
/// The editor's engine implements two of the three natively; the third — whole word
/// under a regular expression — it implements WRONGLY for an alternation, so the
/// application takes it over (`matcher::editor_pattern`). It is a separate function
/// from [`refresh_find`] because a tab can become active with the find bar CLOSED —
/// there is nothing to recount then, but the engine still has to be holding this tab's
/// options before the bar next opens.
fn push_options_to_engine(st: &Rc<TabState>) {
    let options = st.find_options.get();
    let FindOptions {
        case_sensitive,
        whole_word: _,
        regex,
    } = options;
    st.search_settings.set_case_sensitive(case_sensitive);
    // NOT the reader's `whole_word` outright: for a regular expression the wrapping is
    // the application's, because the engine's own is ungrouped
    // (`matcher::WORD_WRAPPED`), and letting it wrap again would nest one correct
    // bounding inside one incorrect one.
    st.search_settings
        .set_at_word_boundaries(crate::window::find::engine_applies_word_boundaries(options));
    st.search_settings.set_regex_enabled(regex);
}

/// Wire `search_context`'s `occurrences-count` notification to keep its
/// current window's match-count label current. Extracted so a freshly created
/// tab (`window/tabs/`'s tab-creation path) can wire
/// its own search context the same way `wire_find_bar` does for a window's
/// first tab — see the module doc for why this connect is inherently
/// per-`SearchContext`-instance rather than a captured-stale-state hazard.
///
/// Takes `content_box` (this tab's own, stable across a cross-window move),
/// not `window`/`match_count_label` directly — QA round-1 H2: a
/// captured window+label pair keeps updating the ORIGIN window's label after
/// a Move Tab to New Window / cross-window drag. Resolving both fresh via
/// [`tabs::resolve_tab_window`] + `winstate::chrome` on every fire targets
/// whichever window this tab currently belongs to.
///
/// The closure captures NOTHING that strong-references `search_context`
/// itself (QA round-2 N12, researcher-confirmed leak): it used to hold a
/// strong clone (`sc2`) so it could pass it to `update_match_count_label`,
/// which is a self-cycle (`SearchContext` owns this handler; the handler's
/// closure held a strong ref back to the `SearchContext`) — GObjects are
/// refcount-only with no cycle collector, so once `TabState` dropped its own
/// ref the context sat at refcount 1 forever, leaking it and everything it
/// strong-holds (`SearchSettings`, the buffer's tag table). The signal
/// already hands the emitting context to the callback as its first
/// argument — read it from there instead.
///
/// NOTE: skip this update in pure-preview mode — there the label is owned by
/// highlight_preview_matches / preview_find_step (body-text-only count). The
/// editor search_context scans the Markdown *source*, which includes table
/// syntax (`| cell |`), so its occurrences-count would overwrite the correct
/// preview count with a larger number that includes matches the preview
/// buffer can never navigate to (they live in child GtkLabel widgets, not the
/// GtkTextBuffer) — giving "N matches" but clicking Next does nothing.
pub(super) fn wire_occurrences_count(
    content_box: &gtk::Box,
    search_context: &sourceview::SearchContext,
) {
    let cb = content_box.downgrade();
    search_context.connect_notify_local(Some("occurrences-count"), move |sc, _| {
        let Some(w) = resolve_tab_window(&cb) else {
            return;
        };
        if current_mode(&w) == ViewMode::Preview {
            return;
        }
        let Some(st) = state(&w) else { return };
        // QA round-2 N10: this handler is bound to ONE specific tab's own
        // search context (see the module doc above), but reads
        // `current_match`/writes the label via the window's ACTIVE tab —
        // those only coincide when the firing context IS that active tab's
        // own. Harmless today (a background tab's occurrences-count cannot
        // currently change without that tab becoming active first), but the
        // invariant was undocumented and unguarded; make it explicit rather
        // than risk misattributing a future background-tab event.
        if st.search_context.as_ptr() != sc.as_ptr() {
            return;
        }
        let Some(chrome) = winstate::chrome(&w) else {
            return;
        };
        // This handler already returned above unless the editor is the visible pane, so
        // the editor's index is the right space to read — and asking for it by name means
        // a preview cursor can never be misreported here as an editor position.
        update_match_count_label(
            sc,
            &chrome.match_count_label,
            st.find_cursor.get().editor_index(),
        );
    });
}

/// Re-apply the active tab's preview find-match highlights after a lifecycle boundary
/// that rebuilt the preview buffer — a **theme re-render** (`re_render_all_windows`) or a
/// **view-mode switch** (edit↔split↔preview, which builds a fresh `render_and_wire_preview`).
///
/// The preview highlights are `scrib-search-hl` tags on the preview `GtkTextBuffer` (plus
/// Pango attrs on table-cell labels), and both boundaries swap in a BRAND-NEW buffer/labels
/// that carry none of them — so the matches silently vanish and only return when the user
/// next edits the query or steps a match (which re-runs `highlight_preview_matches`). This
/// is the GTK4Rs/AP-47/GTK4Rs/AP-47 "a delta-only signal (`search-changed`) misses a lifecycle boundary"
/// class: the highlight is derived state and must be RECOMPUTED at every boundary that
/// rebuilds its substrate, exactly as `refresh_outline`/`refresh_annotations` already are in
/// both sweeps, and as the tab-switch and bar-reopen paths already re-sync find. Mirrors
/// those re-syncs (reset the current-match index to 0 and refresh the count). No-op when the
/// find bar is closed, the query is empty, or there is no preview (edit mode).
pub(crate) fn refresh_preview_find_highlight(window: &ApplicationWindow) {
    let Some(st) = state(window) else { return };
    let chrome = st.chrome();
    if !chrome.find_bar_revealer.reveals_child() {
        return;
    }
    if chrome.find_entry.text().is_empty() {
        return;
    }
    // **Both directions, not only the one that rebuilds a buffer.** This used to act on
    // the Preview arm alone, on the reasoning that a mode switch builds a fresh preview
    // whose highlights are gone while the editor's engine is untouched. The engine is
    // untouched — and the READOUT is not: it was still showing the count of the pane
    // the reader just left. The two panes legitimately count differently (the preview
    // searches three texts, so an anchored pattern matches in each), so switching
    // Preview→Edit left a number on screen that was true of neither the query nor the
    // pane, and only the next Next or keystroke corrected it. Reported by the macOS
    // seat ratifying the match-options batch; the bug is one arm short, not one signal
    // missed, which is why the fix routes the whole refresh rather than adding a second
    // call beside it.
    refresh_find(window, &st);
}
