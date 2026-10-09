//! The right-click context menu attached to the preview/editor text views.

use super::*;
/// Dismiss the per-invocation context popover from inside a child button's `clicked`
/// handler — but DEFERRED to a one-shot idle. The popover is `set_parent`/`popup`'d
/// per right-click and `unparent`s itself on `closed`; calling `popdown()` (→ `closed`
/// → `unparent`) synchronously inside the button's click tears the popover subtree down
/// while the pointer press→release dispatch is still in flight, so GTK then (a) finishes
/// "active"-state accounting on the now-unparented button → `Broken accounting of active
/// state for widget (GtkPopover)`, and (b) drops its still-held event/crossing refs on
/// finalized widgets → `g_object_unref: G_IS_OBJECT` (one per node). Running the popdown
/// on idle moves the whole teardown out of the dispatch. (GTK4Rs/AP-30;
/// researcher-verified, gtk-4-6.)
fn dismiss_context_popover(po: &gtk::Popover) {
    let po = po.clone();
    gtk::glib::idle_add_local_once(move || po.popdown());
}
/// Give `popover` its menu `body` inside a vertical scroller, so the menu's MINIMUM
/// height no longer equals its full height.
///
/// On GTK ≥ 4.14 a popover closes itself on the next frame when its surface is given
/// less room than its minimum size (GTK4Rs/AP-86). With the rows set straight in the
/// popover, the minimum was the whole menu (about 500 pt), so a right-click in the band
/// of the screen where the menu fits neither above nor below the pointer opened the
/// menu and closed it within one frame — the reader saw nothing, and the band depends
/// on screen height and window position, not on the document. Inside the scroller the
/// menu keeps its natural height wherever it fits (`propagate_natural_height`) and
/// scrolls where it does not, which is what GTK's own `GtkPopoverMenu` does. Every
/// right-click menu (this one and the tab menu) goes through here (TDD 9.39).
pub(crate) fn install_menu_body(popover: &gtk::Popover, body: &impl IsA<gtk::Widget>) {
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroller.set_propagate_natural_height(true);
    scroller.set_propagate_natural_width(true);
    scroller.set_child(Some(body));
    popover.set_child(Some(&scroller));
}

/// Attach the right-click context menu to any text view (preview GtkTextView or
/// editor GtkSourceView).  Finds the inner GtkTextView if `container` is a
/// GtkScrolledWindow.  Table-cell label data is read from qdata at click time so
/// this can be called once per view and survives buffer swaps (re_render).
///
/// The capture-phase GestureClick suppresses the view's built-in popup on both
/// the preview and the editor, providing a single consistent menu across all modes.
pub(crate) fn attach_context_menu(container: &gtk::Widget) {
    let Some(view) = find_text_view(container) else {
        return;
    };

    // ctx_cell: the table-cell GtkLabel the user right-clicked on, or None for
    // plain document text.  Read by the Select All handler to select within a cell.
    let ctx_cell: Rc<RefCell<Option<Label>>> = Rc::new(RefCell::new(None));
    let ctx_r = Rc::clone(&ctx_cell);
    let view_weak = view.downgrade();

    let right_click = GestureClick::new();
    right_click.set_button(3);
    right_click.set_propagation_phase(gtk::PropagationPhase::Capture);
    right_click.connect_pressed(move |gesture, _, x, y| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        let Some(view) = view_weak.upgrade() else {
            return;
        };

        // Read table labels from qdata at click time so the list stays current
        // after re_render() swaps the buffer without recreating the view widget.
        let table_labels: Rc<RefCell<Vec<Label>>> = crate::preview::scrib_labels(&view)
            .unwrap_or_else(|| Rc::new(RefCell::new(Vec::new())));

        // Determine whether the click landed on a table-cell GtkLabel.
        *ctx_r.borrow_mut() = {
            let mut w = view.pick(x, y, gtk::PickFlags::DEFAULT);
            let mut found = None;
            while let Some(node) = w {
                if let Ok(label) = node.clone().dynamic_cast::<Label>() {
                    if table_labels.borrow().iter().any(|tl| tl == &label) {
                        found = Some(label);
                        break;
                    }
                }
                w = node.parent();
            }
            found
        };

        // Each context-menu row: [label]  [dim accel hint].  `marked` is a
        // `_`-marked label; `access_markup` renders it as Pango markup with the
        // access char underlined — a plain popover never gets mnemonics-visible, so
        // we draw the underline ourselves rather than via use-underline
        // (GTK4Rs/T-1).  Text-only matches the menu bar (icons ignored —
        // GTK4Rs/AP-11); GtkPopoverMenu has a spurious scrollbar on 4.6, so this stays plain
        // GtkPopover + GtkButton (ScrAP-9), and Change Case is a GtkStack page rather
        // than a nested popover surface (GTK4Rs/AP-69).
        let make_text_btn = |marked: &str, accel: &str| -> gtk::Button {
            let name_lbl = Label::new(None);
            name_lbl.set_markup(&crate::app::access_markup(marked).1);
            name_lbl.set_halign(gtk::Align::Start);
            name_lbl.set_hexpand(true);
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            row.set_hexpand(true);
            row.append(&name_lbl);
            if !accel.is_empty() {
                let hint = Label::new(Some(accel));
                hint.set_halign(gtk::Align::End);
                hint.add_css_class("dim-label");
                hint.set_margin_start(24);
                row.append(&hint);
            }
            let btn = gtk::Button::new();
            btn.set_child(Some(&row));
            btn.add_css_class("flat");
            btn.set_halign(gtk::Align::Fill);
            btn
        };

        let popover = gtk::Popover::new();
        popover.set_has_arrow(false);

        // Single-surface two-page GtkStack (flat menu + Change Case submenu) and one
        // Capture/Local ShortcutController delivering bare-letter access keys — the
        // researcher-confirmed public recipe for a non-model popover (GTK4Rs/T-1/GTK4Rs/AP-69).
        let stack = gtk::Stack::new();
        stack.set_vhomogeneous(false);
        stack.set_interpolate_size(true);
        stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
        let key_controller = gtk::ShortcutController::new();
        key_controller.set_propagation_phase(gtk::PropagationPhase::Capture);
        key_controller.set_scope(gtk::ShortcutScope::Local);
        // Register a button's bare-letter access key, live only while `page` shows
        // (so `u` is Undo on "main" yet UPPER CASE on "change-case").
        let add_key = {
            let ctrl = key_controller.clone();
            let stack = stack.clone();
            move |btn: &gtk::Button, marked: &str, page: &'static str| {
                if let Some(ch) = crate::app::access_markup(marked).0 {
                    let sw = stack.downgrade();
                    let gate = move || {
                        sw.upgrade()
                            .is_some_and(|s| s.visible_child_name().as_deref() == Some(page))
                    };
                    if let Some(sc) = crate::app::access_shortcut(btn, ch, gate) {
                        ctrl.add_shortcut(sc);
                    }
                }
            }
        };

        // Window for action-state lookups; sensitivity then mirrors the menu bar
        // automatically when actions are enabled/disabled (ScrAP-9).
        let win_ref = view
            .root()
            .and_then(|r| r.dynamic_cast::<ApplicationWindow>().ok());

        // Arm Copy Link Location with the link this right-click landed on, BEFORE
        // the rows below read `is_enabled()`. This is the only surface that knows
        // WHICH link the reader means, and it is what makes the row usable in the
        // read-only preview, which has no caret to gate on (`window::copylink`).
        // Disarmed on `closed`, at the bottom of this handler.
        if let Some(w) = win_ref.as_ref() {
            set_context_link(w, link_at_pointer(&view, x, y));
        }

        // Build buttons from EDIT_CMDS; accel hints derived at runtime via
        // accelerator_get_label so they are always platform-correct.
        let edit_btns: Vec<gtk::Button> = EDIT_CMDS
            .iter()
            .map(|cmd| {
                // Same accel-hint derivation the toolbar tooltips use (SSOT).
                let hint = crate::app::accel_hint(cmd.accel).unwrap_or_default();
                make_text_btn(&crate::app::mnem(cmd.label), &hint)
            })
            .collect();

        for (i, cmd) in EDIT_CMDS.iter().enumerate() {
            let bare = cmd
                .action
                .find('.')
                .map_or(cmd.action, |p| &cmd.action[p + 1..]);
            edit_btns[i].set_sensitive(
                win_ref
                    .as_ref()
                    .and_then(|w| w.lookup_action(bare))
                    .is_some_and(|a| a.is_enabled()),
            );

            // Select All has a per-cell branch for table cells; all other commands
            // route through activate_action.
            if cmd.action == "win.select-all" {
                let ctx = Rc::clone(&ctx_r);
                let action = cmd.action;
                // Weak: the popover owns this button (via popup_box), so a strong
                // clone here would form a reference cycle that never drops (leak
                // on every right-click). See ANTI-PATTERNS.md.
                edit_btns[i].connect_clicked(glib::clone!(
                    #[weak(rename_to = po)]
                    popover,
                    #[weak(rename_to = v)]
                    view,
                    #[strong]
                    ctx,
                    move |_| {
                        dismiss_context_popover(&po);
                        if let Some(label) = ctx.borrow().as_ref() {
                            label.select_region(0, -1);
                        } else if v.activate_action(action, None).is_err() {
                            // `log`, not `eprintln!`: this is a GUI-only path, and
                            // the Windows release build is `windows_subsystem =
                            // "windows"` with no console attached — a context-menu
                            // item silently doing nothing would leave no trace at
                            // all there (POLICY § Logging).
                            log::error!("activate_action(\"{action}\") failed");
                        }
                    }
                ));
            } else {
                let action = cmd.action;
                edit_btns[i].connect_clicked(glib::clone!(
                    #[weak(rename_to = po)]
                    popover,
                    #[weak(rename_to = v)]
                    view,
                    move |_| {
                        dismiss_context_popover(&po);
                        if v.activate_action(action, None).is_err() {
                            // `log`, not `eprintln!`: this is a GUI-only path, and
                            // the Windows release build is `windows_subsystem =
                            // "windows"` with no console attached — a context-menu
                            // item silently doing nothing would leave no trace at
                            // all there (POLICY § Logging).
                            log::error!("activate_action(\"{action}\") failed");
                        }
                    }
                ));
            }
        }

        // Register each edit row's bare-letter access key (live on the "main" page).
        for (i, cmd) in EDIT_CMDS.iter().enumerate() {
            add_key(&edit_btns[i], &crate::app::mnem(cmd.label), "main");
        }

        // ── "main" page: EDIT_CMDS rows with section separators, then the editor
        //    section (Insert Emoji + the Change Case submenu row). ──
        let main_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        for (i, btn) in edit_btns.iter().enumerate() {
            if EDIT_CMDS[i].section_start {
                main_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
            }
            main_box.append(btn);
        }
        main_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

        // Annotate — same editor section as the Edit menu. Bind by NAME
        // so GTK drives activation AND sensitivity from the single `win.annotate`
        // action (POLICY SSOT / ScrAP-9). Accel hint from the INLINE_ACCEL
        // table so the shown shortcut matches Ctrl+Alt+M.
        let annotate_marked = crate::app::mnem("Annotate");
        let annotate_hint = crate::app::inline_accel("win.annotate")
            .and_then(crate::app::accel_hint)
            .unwrap_or_default();
        let annotate_btn = make_text_btn(&annotate_marked, &annotate_hint);
        annotate_btn.set_action_name(Some("win.annotate"));
        annotate_btn.connect_clicked(glib::clone!(
            #[weak(rename_to = po)]
            popover,
            move |_| {
                dismiss_context_popover(&po);
            }
        ));
        main_box.append(&annotate_btn);
        add_key(&annotate_btn, &annotate_marked, "main");

        let emoji_marked = crate::app::mnem("Insert Emoji");
        let emoji_btn = make_text_btn(&emoji_marked, "");
        // Bind to the action by NAME so GTK manages BOTH activation and the enabled
        // state from the single `win.insert-emoji` source of truth (POLICY). The
        // extra clicked handler only closes the popover.
        emoji_btn.set_action_name(Some("win.insert-emoji"));
        emoji_btn.connect_clicked(glib::clone!(
            #[weak(rename_to = po)]
            popover,
            move |_| {
                dismiss_context_popover(&po);
            }
        ));
        main_box.append(&emoji_btn);
        add_key(&emoji_btn, &emoji_marked, "main");

        // Change Case is a real submenu (a second stack page) so its four variants
        // keep the SAME access keys as the menu bar's Change Case submenu (U/l/T/c) —
        // GTK4Rs/AP-69. This row slides to that page (it never dismisses).
        let change_case_marked = format!("{} ▸", crate::app::mnem("Change Case"));
        let change_case_btn = make_text_btn(&change_case_marked, "");
        {
            let stack = stack.clone();
            change_case_btn.connect_clicked(move |_| {
                stack.set_visible_child_name("change-case");
            });
        }
        main_box.append(&change_case_btn);
        add_key(&change_case_btn, &change_case_marked, "main");

        // ── "change-case" page: a Back row + the four case variants. ──
        let case_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let back_btn = make_text_btn("◀ Back", "");
        {
            let stack = stack.clone();
            back_btn.connect_clicked(move |_| stack.set_visible_child_name("main"));
        }
        case_box.append(&back_btn);
        case_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

        // All four variants gate on the same change-case action (disabled together
        // when there is no selection or the editor is not visible).
        let case_enabled = win_ref
            .as_ref()
            .and_then(|w| w.lookup_action("change-case"))
            .is_some_and(|a| a.is_enabled());
        for (marked, variant) in [
            (crate::app::mnem("UPPER CASE"), "upper"),
            (crate::app::mnem("lower case"), "lower"),
            (crate::app::mnem("Title Case"), "title"),
            (crate::app::mnem("tOGGLE cASE"), "toggle"),
        ] {
            let btn = make_text_btn(&marked, "");
            btn.set_sensitive(case_enabled);
            let v_str = variant.to_string();
            btn.connect_clicked(glib::clone!(
                #[weak(rename_to = po)]
                popover,
                #[weak(rename_to = v)]
                view,
                move |_| {
                    dismiss_context_popover(&po);
                    let _ = v.activate_action("win.change-case", Some(&v_str.to_variant()));
                }
            ));
            case_box.append(&btn);
            add_key(&btn, &marked, "change-case");
        }

        // Left arrow returns from the submenu to the main page (Back's keyboard peer).
        {
            let sw = stack.downgrade();
            let action = gtk::CallbackAction::new(move |_, _| {
                if let Some(s) = sw.upgrade() {
                    if s.visible_child_name().as_deref() == Some("change-case") {
                        s.set_visible_child_name("main");
                        return gtk::glib::Propagation::Stop;
                    }
                }
                gtk::glib::Propagation::Proceed
            });
            if let Some(left) = gtk::gdk::Key::from_name("Left") {
                let trigger = gtk::KeyvalTrigger::new(left, gtk::gdk::ModifierType::empty());
                key_controller.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(action)));
            }
        }

        stack.add_named(&main_box, Some("main"));
        stack.add_named(&case_box, Some("change-case"));
        stack.set_visible_child_name("main");
        install_menu_body(&popover, &stack);
        popover.add_controller(key_controller);

        let Some(root) = view.root() else { return };
        let (px, py) = view.translate_coordinates(&root, x, y).unwrap_or((x, y));
        popover.set_parent(&root);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(px as i32, py as i32, 1, 1)));
        // Weak (never a strong clone): this closure is owned by the popover, which
        // lives in the window's widget tree — a strong capture would be the
        // uncollectable cycle GTK4Rs/AP-63 is about.
        let win_weak = win_ref.as_ref().map(|w| w.downgrade());
        popover.connect_closed(move |p| {
            // Disarm the right-clicked link with the popover it belongs to, so the
            // pointer target cannot outlive the menu and leave the menu-bar and
            // toolbar surfaces enabled for a link nobody is pointing at.
            if let Some(w) = win_weak.as_ref().and_then(|w| w.upgrade()) {
                set_context_link(&w, None);
            }
            p.unparent();
        });
        popover.popup();
    });
    view.add_controller(right_click);
}

#[cfg(all(test, feature = "gtk-integration-tests"))]
mod gtk_integration_tests {
    use super::*;

    /// Taller than any monitor, so the popover cannot be given its natural height on
    /// either side of its anchor — the situation a right-click near the middle of a
    /// screen shorter than twice the menu puts the real context menu in.
    const TALLER_THAN_ANY_MONITOR: i32 = 20_000;

    /// Wall-clock settle: a popover's surface is sized and its self-dismissal decided
    /// on a frame-clock tick, which turns of the loop alone never advance
    /// (GTK4Rs/AP-261).
    fn pump_for(ms: u64) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(ms);
        while std::time::Instant::now() < deadline {
            while glib::MainContext::default().iteration(false) {}
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    fn popped_over(
        window: &gtk::Window,
        install: impl Fn(&gtk::Popover, &gtk::Box),
    ) -> gtk::Popover {
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.set_size_request(100, TALLER_THAN_ANY_MONITOR);
        let popover = gtk::Popover::new();
        install(&popover, &body);
        popover.set_parent(window);
        popover.popup();
        pump_for(500);
        popover
    }

    /// popdown-then-unparent (GTK4Rs/AP-123), with GTK's 500 ms tooltip hover timeout
    /// let run out in between. The 20000 px popover covers the pointer, so GTK's
    /// tooltip binds to the popover's surface and arms that timeout; on GTK 4.6, if it
    /// fires after `unparent()` has destroyed the surface, it raises a
    /// `gdk_surface_get_device_position` critical, fatal under the suite.
    fn close(popover: &gtk::Popover) {
        popover.popdown();
        pump_for(600);
        popover.unparent();
    }

    /// The context menu must open wherever the reader right-clicks. On GTK ≥ 4.14 a
    /// popover closes itself on the next frame when the screen gives it less room than
    /// its MINIMUM size (GTK4Rs/AP-86), so a menu whose minimum is its full height
    /// opened and vanished in the band of the screen where it fits neither above nor
    /// below the pointer.
    ///
    /// Two halves, because they reach different platforms:
    /// * the **effect** (the menu stays open) is only expressible where GTK
    ///   self-dismisses. Whether it does is MEASURED on a bare popover first, never read
    ///   off the version number: GTK 4.6 clips a too-tall popover, and GTK ≥ 4.23.1, on
    ///   any backend, gives a popover whose minimum equals its natural size its full
    ///   height off-screen, so neither closes it, while 4.14–4.22 does. Where the bare
    ///   popover stays open that half is announced as skipped; where it closes, the
    ///   control has proved the rig can fail;
    /// * the **state** that causes it (the body's minimum height does not grow with
    ///   its content) holds on every GTK and is asserted everywhere.
    #[gtktest::test]
    fn a_context_menu_taller_than_the_screen_still_opens() {
        let window = gtk::Window::new();
        window.set_default_size(400, 300);
        // Something must hold focus, as it always does in the app: a popover opened
        // over a window with no focus widget trips `gtk_widget_is_ancestor` while
        // saving the focus it will restore, a critical that is fatal on Linux.
        let focus_target = gtk::Button::with_label("focus");
        window.set_child(Some(&focus_target));
        window.present();
        focus_target.grab_focus();
        pump_for(300);
        assert!(window.is_mapped(), "no display: the window never mapped");

        // The control: a bare popover whose minimum exceeds the screen. Its fate decides
        // whether this GTK and backend can express the defect at all.
        let bare = popped_over(&window, |p, b| p.set_child(Some(b)));
        let self_dismisses = !bare.is_visible();
        if !self_dismisses {
            println!(
                "SKIPPED [9.39]: effect half — GTK {}.{}.{} on this backend keeps a popover \
                 taller than the screen open instead of closing it",
                gtk::major_version(),
                gtk::minor_version(),
                gtk::micro_version()
            );
        }
        close(&bare);

        let menu = popped_over(&window, install_menu_body);
        if self_dismisses {
            assert!(
                menu.is_visible(),
                "the context menu closed itself because the screen could not fit it"
            );
        }
        let body = menu.child().expect("the menu has a body");
        let (min_h, _, _, _) = body.measure(gtk::Orientation::Vertical, -1);
        assert!(
            min_h < TALLER_THAN_ANY_MONITOR / 10,
            "the menu body's minimum height ({min_h}) follows its content, so a screen \
             shorter than the menu dismisses it"
        );
        close(&menu);
        window.destroy();
    }
}
