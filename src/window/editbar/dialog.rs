//! The small modal input form (`input_form`) shared by the Tier-2 Insert dialogs
//! and Go To Line, plus the "Browse…" file chooser (`BrowseSpec`/`open_file_chooser`)
//! and, for Insert Link, the "Section" heading picker (`super::section`).

use super::super::*;

/// A "Browse…" affordance for one field of an [`input_form`] (the Insert Link / Image
/// URL): a button (mnemonic Alt+R) that opens a file chooser rooted at `start_dir` and
/// writes the chosen path into that field.
pub(in crate::window) struct BrowseSpec {
    /// Index of the field whose row gets the Browse button.
    pub(in crate::window) field: usize,
    /// Folder the chooser opens in (the loaded document's directory), or None for the
    /// chooser default.
    pub(in crate::window) start_dir: Option<std::path::PathBuf>,
    /// Restrict the chooser to image files (Insert Image); false offers any file
    /// (Insert Link).
    pub(in crate::window) image_filter: bool,
}

/// Live validation for one field: the confirm control is insensitive, and the
/// reason visible, whenever the field's current text cannot be accepted.
///
/// Shaped exactly like [`BrowseSpec`] — an optional per-field capability — because
/// it is the same kind of thing, and because the alternative was a second dialog
/// helper. A form that refuses *after* the user commits is a different (worse)
/// interaction from one whose confirm control is simply not available yet, and only
/// the second is what Rename's contract asks for (TDD 24.9).
pub(in crate::window) struct LiveValidate {
    /// Index of the field re-checked on every keystroke.
    pub(in crate::window) field: usize,
    /// `Err(reason)` when the current text cannot be confirmed. The reason is shown
    /// verbatim, so it is a sentence for a reader, not a debug string.
    pub(in crate::window) check: FieldCheck,
}

/// A field's live-validation rule. Named rather than written inline so the struct
/// above reads as a pair of intentions instead of a type signature.
pub(in crate::window) type FieldCheck = Box<dyn Fn(&str) -> Result<(), String>>;

/// The optional per-field capabilities an [`input_form`] can carry.
///
/// One parameter rather than two because they are the same kind of thing — a
/// capability attached to one field by index — and because a form helper with eight
/// positional arguments is a call site nobody can read (or get right) without
/// counting `None`s.
#[derive(Default)]
pub(in crate::window) struct FieldExtras {
    /// A "Browse…" button on one field (Insert Link / Image).
    pub(in crate::window) browse: Option<BrowseSpec>,
    /// Live validation gating the confirm control (Rename — TDD 24.9).
    pub(in crate::window) validate: Option<LiveValidate>,
    /// A "Section" heading picker on one field (Insert Link).
    pub(in crate::window) section: Option<super::section::SectionSpec>,
}

/// The Browse button's label; `r` is its mnemonic letter.
const BROWSE_LABEL: &str = "B_rowse…";

/// Give each button in `buttons` the platform's shortcut for its mnemonic letter, where
/// the toolkit's own Alt+letter cannot reach it (`accel::dialog_button_accel` — on macOS
/// Option+letter types a character, so the button is pressed with Command+letter).
///
/// Bubble phase, so a focused field's own bindings answer first; none of them takes a
/// Command+letter that a dialog button uses. The letter is read from the button's own
/// label, so a relabelled button cannot keep a stale shortcut.
fn bind_button_letters(
    dialog: &gtk::Window,
    buttons: &[gtk::Widget],
    platform: crate::accel::Platform,
) {
    let controller = gtk::ShortcutController::new();
    for button in buttons {
        let label = button
            .downcast_ref::<gtk::Button>()
            .and_then(|b| b.label())
            .or_else(|| {
                button
                    .downcast_ref::<gtk::MenuButton>()
                    .and_then(|b| b.label())
            });
        let Some(letter) = label.as_deref().and_then(crate::accel::mnemonic_letter) else {
            continue;
        };
        let Some(accel) = crate::accel::dialog_button_accel(letter, platform) else {
            continue;
        };
        let Some(trigger) = gtk::ShortcutTrigger::parse_string(&accel) else {
            continue;
        };
        let target = button.downgrade();
        // Always consumed, even when the button cannot act. On macOS an unhandled key
        // goes on to the native menu bar, where Command+S is Save — of the document
        // behind this modal dialog, which is not what a keystroke in the dialog means.
        let action = gtk::CallbackAction::new(move |_, _| {
            if let Some(b) = target.upgrade().filter(|b| b.is_sensitive()) {
                b.activate();
            }
            gtk::glib::Propagation::Stop
        });
        controller.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(action)));
    }
    dialog.add_controller(controller);
}

/// The fewest characters any prompt field shows, whatever buttons share its row.
const MIN_FIELD_CHARS: i32 = 24;

/// Run a small modal input form: one labelled `GtkEntry` per field, plus Cancel /
/// `confirm_label`. On confirm, calls `on_ok(window, values)` with the entry texts
/// in field order; Cancel or Escape closes without acting. A plain `GtkWindow` (no
/// `GtkDialog`, which is deprecated ≥ 4.10) keeps `clippy -D warnings` clean. Used
/// by the Tier-2 Insert Link / Image / Table commands (POLICY single-source-of-truth:
/// they are `win.format` targets like the inline wraps), Go To Line, and **Rename**
/// (`window::rename`, the caller outside this module and so the one enumerations miss) —
/// `confirm_label` keeps the button's verb accurate for each ("Insert" vs "Go").
///
/// `focus_field` is the index of the field that takes the initial keyboard focus.
/// It is a caller's choice rather than "always field 0" because the right answer
/// depends on what the *caller* pre-filled: a dialog whose first field is already
/// seeded (Insert Link over a selection — the caption is filled in, the URL is not)
/// should land the caret on the field the user still has work to do in, not make
/// them Tab past a field that is already correct. An out-of-range index falls back
/// to the first field.
pub(in crate::window) fn input_form(
    window: &ApplicationWindow,
    title: &str,
    fields: &[(&str, String)],
    extras: FieldExtras,
    confirm_label: &str,
    focus_field: usize,
    on_ok: impl Fn(&ApplicationWindow, Vec<String>) + 'static,
) {
    let dialog = gtk::Window::builder()
        .transient_for(window)
        .modal(true)
        .title(title)
        .default_width(380)
        .resizable(false)
        .build();

    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 8);
    vbox.set_margin_top(12);
    vbox.set_margin_bottom(12);
    vbox.set_margin_start(12);
    vbox.set_margin_end(12);

    let FieldExtras {
        browse,
        validate,
        mut section,
    } = extras;
    // The buttons that carry a mnemonic, handed forward to `bind_button_letters` rather
    // than re-found by walking the finished dialog (GTK4Rs/AP-10).
    let mut lettered: Vec<gtk::Widget> = Vec::new();
    let entries: Vec<gtk::Entry> = fields
        .iter()
        .enumerate()
        .map(|(i, (label, initial))| {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let lbl = gtk::Label::new(Some(*label));
            lbl.set_xalign(0.0);
            lbl.set_width_chars(9);
            row.append(&lbl);
            // Named from the row's own label, which is the field's identity here. It
            // is the visible label AND the accessible name deliberately: the two are
            // the same string, and this site is why the constructor takes the name as a
            // required argument rather than an option. It previously wired macOS word
            // navigation and named nothing, relying on the adjacent `GtkLabel` — a
            // defensible choice that was nowhere stated and indistinguishable from an
            // omission, and one the a11y tree-walk guard never sees because it walks
            // windows, not this dialog.
            let entry = crate::widgets::textfield::named_entry(label, initial);
            // A floor on the field, not a width for it: a row that also carries Browse…
            // and Section would otherwise squeeze its field to a few characters inside
            // the dialog's default width. With the floor the non-resizable dialog widens
            // to fit instead; rows without buttons are already wider and unchanged.
            entry.set_width_chars(MIN_FIELD_CHARS);
            // Enter triggers the default widget (Insert) from any field.
            entry.set_activates_default(true);
            row.append(&entry);
            // The browse field (the Link / Image URL) gets a "Browse…" button —
            // mnemonic Alt+R, active while the dialog is focused — that opens a file
            // chooser and fills the entry with the chosen path.
            if browse.as_ref().is_some_and(|b| b.field == i) {
                let start_dir = browse.as_ref().and_then(|b| b.start_dir.clone());
                let image_filter = browse.as_ref().is_some_and(|b| b.image_filter);
                let browse_btn = gtk::Button::with_mnemonic(BROWSE_LABEL);
                let entry_for_browse = entry.clone();
                browse_btn.connect_clicked(gtk::glib::clone!(
                    #[weak(rename_to = parent)]
                    dialog,
                    move |_| {
                        open_file_chooser(
                            &parent,
                            start_dir.clone(),
                            &entry_for_browse,
                            image_filter,
                        );
                    }
                ));
                row.append(&browse_btn);
                lettered.push(browse_btn.upcast());
            }
            // The section field (the Link URL) also gets a "Section" picker — mnemonic
            // Alt+S — whose chosen heading replaces the field's text, as Browse does.
            if section.as_ref().is_some_and(|s| s.field == i) {
                if let Some(spec) = section.take() {
                    let section_btn = super::section::section_button(spec, &entry);
                    row.append(&section_btn);
                    lettered.push(section_btn.upcast());
                }
            }
            vbox.append(&row);
            entry
        })
        .collect();

    // The refusal line, between the fields and the buttons. Built unconditionally
    // but only ever populated by a `LiveValidate`, so an unvalidated form is
    // pixel-identical to what it was before this parameter existed.
    let reason = gtk::Label::new(None);
    reason.set_xalign(0.0);
    reason.set_wrap(true);
    reason.set_visible(false);
    reason.add_css_class("dim-label");
    vbox.append(&reason);

    let btn_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    btn_row.set_halign(gtk::Align::End);
    btn_row.set_margin_top(8);
    let cancel = gtk::Button::with_label("Cancel");
    let ok = gtk::Button::with_label(confirm_label);
    ok.add_css_class("suggested-action");
    btn_row.append(&cancel);
    btn_row.append(&ok);
    vbox.append(&btn_row);

    dialog.set_child(Some(&vbox));
    bind_button_letters(&dialog, &lettered, crate::accel::host());
    dialog.set_default_widget(Some(&ok));

    // Gate the confirm control on the field's current text, and run it ONCE before
    // the dialog is shown as well as on every keystroke. The initial run is the
    // load-bearing half: a Rename dialog opens pre-filled with the document's
    // CURRENT name, which is itself a refusal (`Unchanged`), so a form that only
    // validated on change would open offering a confirm that cannot succeed.
    if let Some(LiveValidate { field, check }) = validate {
        if let Some(entry) = entries.get(field).cloned() {
            let ok_btn = ok.clone();
            let reason_lbl = reason.clone();
            let apply = move |text: &str| match check(text) {
                Ok(()) => {
                    ok_btn.set_sensitive(true);
                    reason_lbl.set_visible(false);
                }
                Err(why) => {
                    ok_btn.set_sensitive(false);
                    reason_lbl.set_text(&why);
                    reason_lbl.set_visible(true);
                }
            };
            apply(&entry.text());
            // The entry owns this handler, so the closure must not hold the entry
            // strongly — the handler's own emitter argument is the entry (GTK4Rs/AP-63).
            entry.connect_changed(move |e| apply(&e.text()));
        }
    }

    // Weak self-refs throughout: a strong `dialog.clone()` captured in a handler
    // the dialog itself owns (as a descendant widget's signal) forms a reference
    // cycle that never drops, leaking the dialog window on every Insert
    // Link/Image/Table invocation.
    cancel.connect_clicked(gtk::glib::clone!(
        #[weak(rename_to = d)]
        dialog,
        move |_| {
            d.close();
        }
    ));
    // Escape cancels.
    let key = gtk::EventControllerKey::new();
    {
        let d = dialog.downgrade();
        key.connect_key_pressed(move |_, keyval, _, _| {
            if keyval == gtk::gdk::Key::Escape {
                if let Some(d) = d.upgrade() {
                    d.close();
                }
                gtk::glib::Propagation::Stop
            } else {
                gtk::glib::Propagation::Proceed
            }
        });
    }
    dialog.add_controller(key);

    {
        let d = dialog.downgrade();
        let win_weak = window.downgrade();
        let entries = entries.clone();
        ok.connect_clicked(move |_| {
            let values: Vec<String> = entries.iter().map(|e| e.text().to_string()).collect();
            if let Some(w) = win_weak.upgrade() {
                on_ok(&w, values);
            }
            if let Some(d) = d.upgrade() {
                d.close();
            }
        });
    }

    dialog.present();
    // `grab_focus` on a GtkEntry also selects its text, so a pre-filled focused
    // field is ready to be typed over rather than appended to.
    if let Some(entry) = entries.get(focus_field).or_else(|| entries.first()) {
        entry.grab_focus();
    }
}

/// Open a native file chooser rooted at `start_dir`, writing the chosen file's path
/// into `entry` (relative to `start_dir` when the file lives under it, else absolute).
/// With `image_filter`, the chooser defaults to image files. Backs the Insert Link /
/// Image URL "Browse…" buttons.
fn open_file_chooser(
    parent: &gtk::Window,
    start_dir: Option<std::path::PathBuf>,
    entry: &gtk::Entry,
    image_filter: bool,
) {
    let chooser = FileChooserNative::new(
        Some(if image_filter {
            "Select Image"
        } else {
            "Select File"
        }),
        Some(parent),
        FileChooserAction::Open,
        Some("Select"),
        Some("Cancel"),
    );
    if image_filter {
        let images = gtk::FileFilter::new();
        images.set_name(Some("Images"));
        for pat in [
            "*.png", "*.jpg", "*.jpeg", "*.gif", "*.svg", "*.webp", "*.bmp",
        ] {
            images.add_pattern(pat);
        }
        chooser.add_filter(&images);
    }
    let all = gtk::FileFilter::new();
    all.set_name(Some("All files"));
    all.add_pattern("*");
    chooser.add_filter(&all);
    // Start in doc dir if provided, otherwise the last-visited dialog dir.
    let effective_dir = start_dir
        .clone()
        .or_else(|| crate::app::LAST_DIALOG_DIR.with(|c| c.borrow().clone()));
    if let Some(dir) = &effective_dir {
        let _ = chooser.set_current_folder(Some(&gtk::gio::File::for_path(dir)));
    }
    let entry = entry.clone();
    crate::saferizer::native_dialog::NativeDialogHolder::show(&chooser, move |ch, resp| {
        if resp == ResponseType::Accept {
            if let Some(path) = ch.file().and_then(|f| f.path()) {
                remember_dialog_dir(&path);
                // Document-relative when there is a base folder (security +
                // portability — same containment as image loading), absolute for an
                // untitled buffer; encoded either way (`links::path_for_insert`).
                let text = crate::links::path_for_insert(&path, start_dir.as_deref());
                entry.set_text(&text);
            }
        }
        ch.destroy();
    });
}

#[cfg(all(test, feature = "gtk-integration-tests"))]
mod tests {
    use super::*;
    use crate::window::testkit::test_app;

    /// Collect every `GtkEntry` in `root`'s widget subtree.
    fn entries_in(root: &gtk::Widget) -> Vec<gtk::Entry> {
        let mut out = Vec::new();
        if let Some(e) = root.downcast_ref::<gtk::Entry>() {
            out.push(e.clone());
        }
        let mut child = root.first_child();
        while let Some(c) = child {
            out.extend(entries_in(&c));
            child = c.next_sibling();
        }
        out
    }

    /// Every prompt field announces its own name.
    ///
    /// Asserted on the assembled dialog rather than on the constructor, because the
    /// constructor already has its own test and this site's failure mode was not "the
    /// constructor is wrong" — it was that this site never called one. The field sits
    /// beside a visible `GtkLabel`, which is a defensible reason to omit an accessible
    /// name and is indistinguishable from having forgotten one; and `a11y`'s tree-walk
    /// guard cannot arbitrate, because it walks WINDOWS and this is a transient dialog.
    #[gtktest::test]
    fn every_prompt_field_carries_an_accessible_name() {
        let app = test_app("com.extollit.scribobulate.integrationtest.promptnames");
        let window = crate::window::new_window(&app, "IT", "# doc\n", None);
        input_form(
            &window,
            "Insert Link",
            &[("Text", String::new()), ("URL", String::from("seed"))],
            FieldExtras::default(),
            "Insert",
            0,
            |_, _| {},
        );

        // A separate toplevel, and NOT in `app.windows()`: `input_form` builds a plain
        // `gtk::Window` with only `transient_for` set, so it is never registered with the
        // GApplication. `gtk::Window::toplevels()` is the list that does contain it.
        let toplevels = gtk::Window::toplevels();
        let dialog = (0..toplevels.n_items())
            .filter_map(|i| toplevels.item(i).and_downcast::<gtk::Window>())
            .find(|w| w.title().is_some_and(|t| t == "Insert Link"))
            .expect("the prompt dialog is among the toplevel windows");
        let fields = entries_in(dialog.upcast_ref::<gtk::Widget>());
        assert_eq!(fields.len(), 2, "expected the two declared prompt fields");
        for (i, field) in fields.iter().enumerate() {
            assert!(
                crate::a11y::has_name(field),
                "prompt field {i} has no accessible name — build it with \
                 `widgets::textfield::named_entry`"
            );
        }
        assert_eq!(
            fields[1].text(),
            "seed",
            "a field's initial text was dropped"
        );
        dialog.destroy();
    }

    /// On the Mac a dialog's lettered buttons are pressed with Command+letter, because
    /// Option+letter types a character there; elsewhere the toolkit's Alt mnemonic does
    /// it and nothing extra is bound. Driven through the shortcut's own action, so the
    /// Mac half is proved from any host.
    #[gtktest::test]
    fn lettered_buttons_take_command_letter_on_the_mac_only() {
        use crate::accel::Platform;
        let shortcuts = |platform| {
            let window = gtk::Window::new();
            let browse = gtk::Button::with_mnemonic(BROWSE_LABEL);
            let section = gtk::MenuButton::new();
            section.set_label("_Section");
            section.set_use_underline(true);
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            row.append(&browse);
            row.append(&section);
            window.set_child(Some(&row));
            bind_button_letters(
                &window,
                &[browse.clone().upcast(), section.upcast()],
                platform,
            );
            let controller = window
                .observe_controllers()
                .into_iter()
                .filter_map(Result::ok)
                .find_map(|c| c.downcast::<gtk::ShortcutController>().ok())
                .expect("a shortcut controller is installed");
            let found: Vec<gtk::Shortcut> = (0..controller.n_items())
                .filter_map(|i| controller.item(i).and_downcast::<gtk::Shortcut>())
                .collect();
            (window, browse, found)
        };

        let (window, _, other) = shortcuts(Platform::Other);
        assert!(
            other.is_empty(),
            "the toolkit's mnemonic serves; nothing is added"
        );
        window.destroy();

        let (window, browse, mac) = shortcuts(Platform::Mac);
        window.present();
        let spelled: Vec<String> = mac
            .iter()
            .filter_map(|s| s.trigger())
            .map(|t| t.to_str().to_string())
            .collect();
        assert_eq!(spelled, ["<Meta>r", "<Meta>s"]);
        let clicked = std::rc::Rc::new(std::cell::Cell::new(false));
        let seen = clicked.clone();
        browse.connect_clicked(move |_| seen.set(true));
        let action = mac[0].action().expect("an action");
        assert!(action.activate(
            gtk::ShortcutActionFlags::empty(),
            &window,
            None::<&gtk::glib::Variant>
        ));
        // A button's activation is a timed press that clicks when it ends, and only on
        // a realized button — so the window is shown and the press given wall-clock time.
        crate::testpump::until(
            crate::testpump::Clock::Frame,
            "Command+R pressed Browse",
            || clicked.get(),
        );
        crate::testpump::destroy_after_tooltip_delay(&window, || {});
    }
}
