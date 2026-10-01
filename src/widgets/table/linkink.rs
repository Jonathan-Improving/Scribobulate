//! **The ink of selected text inside a table cell** — drawn in the selection's own text
//! colour, as selected body text is. Two different selections reach a cell, and each
//! needed its own lever.
//!
//! **1. A selection made inside the cell — its LINKS were wrong, and under System its
//! whole text.** A `GtkTextView`
//! draws every selected glyph in its `text selection` node's `color`, a tag's
//! `foreground` included, so a selected body link reads as selected text. A `GtkLabel`
//! turns each `link` node's `color` into a Pango foreground ATTRIBUTE over the link's
//! range, then paints the selection by drawing the layout in the `selection` node's
//! colour — which is only the layout's default, and a run carrying its own foreground
//! attribute keeps it. So a selected cell link stayed link-coloured on the selection
//! fill. No CSS reaches it: GTK sets no state on a `link` node while it is selected and
//! the `selection` node has no `link` child, so `link:selected` and `selection link`
//! match nothing (measured). The lever that works is the label's own `attributes`,
//! which GTK lays over the link attributes it builds: a foreground over exactly the
//! selected text wins there and nowhere else, so the unselected part of a half-selected
//! link keeps its link colour. The ink laid there is the BODY's selected-text ink, not
//! the label's own, because GTK 4.6's Adwaita inks a label's selection white over the
//! same pale fill the body's dark selected text sits on (see [`cell_selection_ink`]).
//!
//! **2. A body selection that spans the table — the whole cell was wrong.** The cells
//! are not selected at all (a table is one `U+FFFC` to the buffer's selection), but the
//! text view paints its selection fill under that character, which is the whole table,
//! and the body cells are transparent — so the cells LOOK selected while every glyph in
//! them keeps its unselected ink on the fill. The preview's selection handler marks the
//! table ([`super::ScribTableWidget::set_in_body_selection`]) and each body cell then
//! inks its whole text as the body's selected text is inked. A header cell is left
//! alone: its own fill sits over the selection's, so its ink is still on its own fill.
//!
//! MEASURED (GTK 4.6.9, Xvfb, every built-in reading theme and three desktop themes):
//! case 1's links hit every theme, and under System on Adwaita a cell's plain selected
//! text was white on the pale fill at ~1.5:1; case 2 put the System theme's cell link at
//! 3.1:1 on Adwaita's fill and Pixel Quest's cell text and links near the fill's own colour.
//!
//! **The colour is READ, never chosen.** On a themed page both selection nodes take
//! `palette.selection_fg` from `preview::css`; under the System reading theme the app
//! states nothing and the DESKTOP theme decides, differently per theme. Any colour picked
//! here would be a second owner of a value the stylesheet already owns, and wrong under
//! System. So the ink is resolved from stand-in nodes with the same names and parents as
//! the real ones — `textview > text > selection` for both cases (`label > selection`
//! only for a label outside any view); the real nodes are private to GTK. The ink therefore matches the selected
//! text beside it by construction, on every theme and in every window state.
//!
//! The ink is one LAYER of the cell's attributes ([`super::cellattrs`]), so it composes
//! with find's match washes rather than replacing them.
//!
//! Re-applied only when the inked range or the ink actually changes, and the ink is read
//! once per cell until the window's state changes, so a drag costs one attribute
//! replacement per step that moves the selection and no style resolution.
//! Case 1 is driven by [`refresh_cell_selection_ink`], case 2 by the table.

use super::cellattrs::{set_layer, Layer};
use crate::saferizer::qdata_key::QdataKey;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{glib, pango, Label};
use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

/// The qdata a cell's ink state lives under, so the table can reach it for case 2.
const INK_STATE: QdataKey<Rc<InkState>> = QdataKey::new("scrib-cell-ink");

/// Make `label`'s selected text take the selection's ink — see the module header.
pub(crate) fn track_selection_ink(label: &Label) {
    let state = Rc::new(InkState {
        body_ink: std::cell::Cell::new(None),
        own_ink: std::cell::Cell::new(None),
        applied: RefCell::new(None),
    });
    INK_STATE.set(label, state.clone());
    // The ink follows the window's state as the selected text beside it does: a desktop
    // theme may restyle `selection` while the window is unfocused (CAM Document
    // Rendering row 18), and nothing about the selection changes then — so the body's
    // ink is re-read too, which is the one read made per cell rather than per table.
    label.connect_state_flags_changed(move |label, _| {
        // A label leaving its window (a re-render, a tab closing, a test tearing its
        // fixture down) has its state flags changed on the way out; re-inking then would
        // parent probes and re-parse markup on a widget tree that is being dismantled.
        if label.root().is_none() {
            return;
        }
        if state.body_ink.get().is_some() {
            state.body_ink.set(body_selection_ink(label));
        }
        state.own_ink.set(None);
        state.refresh(label);
    });
}

thread_local! {
    /// The one cell that last took case-1 ink, so it can be given it back. One is
    /// enough: a label's selection claims PRIMARY, and claiming it clears the previous
    /// owner's selection, so two cells never hold a selection at once.
    static INKED: RefCell<glib::WeakRef<Label>> = RefCell::new(glib::WeakRef::new());
}

/// Case 1's trigger: re-ink after a cell's selection may have changed. Called from the
/// preview's PRIMARY-clipboard `changed` listener, because a `GtkLabel` has no selection
/// signal at all — `cursor-position` and `selection-bound` are never notified by a
/// selection change at GTK 4.6 (MEASURED), and PRIMARY is what a label's selection
/// writes on every change (GTK4Rs/AP-28).
///
/// A hot path (CAM Hot-path row 9), so it touches at most two cells rather than every
/// cell in the document: the one holding focus — a label takes focus when it is pressed
/// or keyed, which is how its selection changes — and the one that last took ink, which
/// is the only other cell that can be carrying any.
pub(crate) fn refresh_cell_selection_ink(view: &impl IsA<gtk::Widget>) {
    let focused = view
        .root()
        .and_then(|root| root.focus())
        .and_then(|widget| widget.downcast::<Label>().ok());
    let previous = INKED.with(|inked| inked.borrow().upgrade());
    let mut still_inked = None;
    for label in previous.iter().chain(focused.iter()) {
        if let Some(state) = INK_STATE.get(label) {
            state.refresh(label);
            if state.applied.borrow().is_some() {
                still_inked = Some(label.clone());
            }
        }
    }
    INKED.with(|inked| inked.borrow().set(still_inked.as_ref()));
}

/// Case 2: ink `label` as the body's selected text (`Some(ink)`, read once per selection
/// change by [`body_selection_ink`]) or return it to its own ink (`None`), when a body
/// selection starts or stops spanning its table. A label built without
/// [`track_selection_ink`] is left alone.
pub(crate) fn set_body_selection_ink(label: &Label, ink: Option<gtk::gdk::RGBA>) {
    if let Some(state) = INK_STATE.get(label) {
        state.body_ink.set(ink);
        state.refresh(label);
    }
}

/// What one cell knows: the body's selection ink while a body selection spans its
/// table, the ink its own selection takes (read on first use and dropped when the
/// window's state changes, so a drag does not re-read it per step), and what it last
/// applied, so an unchanged answer costs nothing.
struct InkState {
    body_ink: std::cell::Cell<Option<gtk::gdk::RGBA>>,
    own_ink: std::cell::Cell<Option<gtk::gdk::RGBA>>,
    applied: RefCell<Option<Applied>>,
}

#[derive(PartialEq)]
struct Applied {
    ranges: Vec<Range<usize>>,
    ink: gtk::gdk::RGBA,
}

impl InkState {
    fn refresh(&self, label: &Label) {
        let next = if let Some(ink) = self.body_ink.get() {
            Some(Applied {
                ranges: std::iter::once(0..label.text().len()).collect(),
                ink,
            })
        } else {
            self.own_selection(label)
        };
        if *self.applied.borrow() == next {
            return;
        }
        set_layer(label, Layer::Ink, next.as_ref().map(ink_attributes));
        self.applied.replace(next);
    }

    /// Case 1: the cell's own selected text, links included, in the ink the body's
    /// selected text takes — see [`cell_selection_ink`].
    fn own_selection(&self, label: &Label) -> Option<Applied> {
        let text = label.text();
        let (a, b) = label.selection_bounds()?;
        let range = selected_bytes(&text, a, b)?;
        let ink = match self.own_ink.get() {
            Some(ink) => ink,
            None => {
                let ink = cell_selection_ink(label);
                self.own_ink.set(Some(ink));
                ink
            }
        };
        Some(Applied {
            ranges: vec![range],
            ink,
        })
    }
}

/// The byte range a label selection between characters `a` and `b` covers, in either
/// order; `None` when it is empty. Pure, so it is the part under unit test.
pub(crate) fn selected_bytes(text: &str, a: i32, b: i32) -> Option<Range<usize>> {
    let (a, b) = (byte_at(text, a), byte_at(text, b));
    (a != b).then(|| a.min(b)..a.max(b))
}

/// The ink a cell's own selected text takes: the BODY's selected-text ink when the cell
/// sits in a preview, else (a label outside any text view) its own `selection` node's.
///
/// Not the label's own node, because a desktop theme may ink the two selections
/// differently over the SAME fill. GTK 4.6's Adwaita gives `label > selection` and
/// `textview > text > selection` one pale fill (`$selected_text_bg_color`, the accent at
/// 30%) but states `color: $selected_fg_color` (white) on the label's alone, so a
/// label's selected text was white at ~1.5:1 while the body's beside it stayed the
/// ordinary dark text. On a themed page the two rules are the same `palette.selection_fg`
/// (`preview::css`), and Breeze states the same fill and ink on both, so reading the
/// body's changes nothing there and fixes Adwaita.
fn cell_selection_ink(label: &Label) -> gtk::gdk::RGBA {
    body_selection_ink(label).unwrap_or_else(|| {
        probe_ink(
            label.upcast_ref(),
            &[glib::Object::new::<SelectionNodeProbe>().upcast()],
        )
    })
}

/// The byte offset of character `index` in `text` — a label's selection is counted in
/// characters, a Pango attribute in bytes. Clamped to the end.
fn byte_at(text: &str, index: i32) -> usize {
    usize::try_from(index)
        .ok()
        .and_then(|i| text.char_indices().nth(i))
        .map_or(text.len(), |(byte, _)| byte)
}

fn ink_attributes(applied: &Applied) -> pango::AttrList {
    let to_u16 = |c: f32| (c.clamp(0.0, 1.0) * f32::from(u16::MAX)).round() as u16;
    let ink = &applied.ink;
    let list = pango::AttrList::new();
    for range in &applied.ranges {
        let (Ok(start), Ok(end)) = (u32::try_from(range.start), u32::try_from(range.end)) else {
            continue;
        };
        let mut fg = pango::AttrColor::new_foreground(
            to_u16(ink.red()),
            to_u16(ink.green()),
            to_u16(ink.blue()),
        );
        fg.set_start_index(start);
        fg.set_end_index(end);
        list.insert(fg);
        let mut alpha = pango::AttrInt::new_foreground_alpha(to_u16(ink.alpha()));
        alpha.set_start_index(start);
        alpha.set_end_index(end);
        list.insert(alpha);
    }
    list
}

/// The colour the body's selected text is drawn in, for the preview `inside` sits in:
/// the `color` of `textview > text > selection`. `None` for a widget not (yet) in a view.
pub(crate) fn body_selection_ink(inside: &impl IsA<gtk::Widget>) -> Option<gtk::gdk::RGBA> {
    let view = inside.ancestor(gtk::TextView::static_type())?;
    Some(probe_ink(
        &view,
        &[
            glib::Object::new::<TextNodeProbe>().upcast(),
            glib::Object::new::<SelectionNodeProbe>().upcast(),
        ],
    ))
}

/// The `color` the last node of `chain` resolves to, each node parented under the one
/// before it and the first under `parent` — the CSS path GTK's private subnode sits at.
/// The stand-ins are hidden and parented only for the read, so they never lay out, paint
/// or outlive this call; each carries `parent`'s state, as GTK's own subnodes do.
fn probe_ink(parent: &gtk::Widget, chain: &[gtk::Widget]) -> gtk::gdk::RGBA {
    let flags = parent.state_flags();
    let mut above = parent;
    for node in chain {
        node.set_visible(false);
        node.set_parent(above);
        node.set_state_flags(flags, true);
        above = node;
    }
    let ink = above.style_context().color();
    for node in chain.iter().rev() {
        node.unparent();
    }
    ink
}

glib::wrapper! {
    /// A widget whose only job is to resolve a `selection` node's style — see
    /// [`probe_ink`].
    pub(crate) struct SelectionNodeProbe(ObjectSubclass<imp::SelectionNodeProbe>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

glib::wrapper! {
    /// Stands in for a text view's `text` node, so a [`SelectionNodeProbe`] under it
    /// resolves as the body's selection does.
    pub(crate) struct TextNodeProbe(ObjectSubclass<imp::TextNodeProbe>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub(crate) struct SelectionNodeProbe;

    #[glib::object_subclass]
    impl ObjectSubclass for SelectionNodeProbe {
        const NAME: &'static str = "ScribSelectionNodeProbe";
        type Type = super::SelectionNodeProbe;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("selection");
        }
    }

    impl ObjectImpl for SelectionNodeProbe {}
    impl WidgetImpl for SelectionNodeProbe {}

    #[derive(Default)]
    pub(crate) struct TextNodeProbe;

    #[glib::object_subclass]
    impl ObjectSubclass for TextNodeProbe {
        const NAME: &'static str = "ScribTextNodeProbe";
        type Type = super::TextNodeProbe;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("text");
        }
    }

    impl ObjectImpl for TextNodeProbe {}
    impl WidgetImpl for TextNodeProbe {}
}

#[cfg(test)]
mod tests {
    use super::{byte_at, ink_attributes, selected_bytes, Applied};
    use gtk::pango;

    #[test]
    fn each_inked_range_gets_the_ink_and_its_alpha_and_nothing_else() {
        let applied = Applied {
            ranges: vec![2..5, 9..12],
            ink: gtk::gdk::RGBA::new(1.0, 0.0, 0.5, 0.5),
        };
        let got: Vec<_> = ink_attributes(&applied)
            .attributes()
            .into_iter()
            .map(|a| (a.type_(), a.start_index(), a.end_index()))
            .collect();
        let (fg, alpha) = (
            pango::AttrType::Foreground,
            pango::AttrType::ForegroundAlpha,
        );
        assert_eq!(
            got,
            vec![(fg, 2, 5), (alpha, 2, 5), (fg, 9, 12), (alpha, 9, 12)]
        );
        let first = ink_attributes(&applied).attributes()[0].clone();
        let c = first
            .downcast::<pango::AttrColor>()
            .expect("a foreground")
            .color();
        assert_eq!((c.red(), c.green(), c.blue()), (u16::MAX, 0, 32768));
    }

    #[test]
    fn a_cell_selection_maps_to_one_byte_range_in_either_direction() {
        let text = "é a ☑ b";
        assert_eq!(selected_bytes(text, 1, 4), Some(2..5));
        assert_eq!(selected_bytes(text, 4, 1), Some(2..5));
        assert_eq!(selected_bytes(text, 3, 3), None);
    }

    #[test]
    fn a_selection_in_characters_is_mapped_to_bytes() {
        let text = "é a ☑ b";
        assert_eq!(byte_at(text, 0), 0);
        assert_eq!(byte_at(text, 1), 2);
        assert_eq!(byte_at(text, 4), 5);
        assert_eq!(byte_at(text, 99), text.len());
        assert_eq!(byte_at(text, -1), text.len());
    }
}

#[cfg(all(test, feature = "gtk-integration-tests"))]
mod gtk_integration_tests {
    use super::{probe_ink, refresh_cell_selection_ink, SelectionNodeProbe, TextNodeProbe};
    use crate::widgets::table::{cell_markup_label, link_markup_open, ScribTableWidget};
    use gtk::prelude::*;
    use gtk::{glib, pango, Label};

    /// Every foreground attribute in `list`, as `(start..end, rgb)`.
    fn foregrounds(list: Option<pango::AttrList>) -> Vec<(std::ops::Range<u32>, [u16; 3])> {
        list.map(|l| l.attributes())
            .unwrap_or_default()
            .into_iter()
            .filter(|a| a.type_() == pango::AttrType::Foreground)
            .filter_map(|a| {
                let range = a.start_index()..a.end_index();
                let c = a.downcast::<pango::AttrColor>().ok()?.color();
                Some((range, [c.red(), c.green(), c.blue()]))
            })
            .collect()
    }

    fn rgb(c: gtk::gdk::RGBA) -> [u16; 3] {
        let f = |v: f32| (v.clamp(0.0, 1.0) * f32::from(u16::MAX)).round() as u16;
        [f(c.red()), f(c.green()), f(c.blue())]
    }

    fn link_cell(caption: &str) -> Label {
        let label = cell_markup_label(&format!(
            "a {}{caption}</a> b",
            link_markup_open("https://example.com")
        ));
        label.set_selectable(true);
        label
    }

    /// Case 1, outside a view: selecting part of a cell's link gives exactly the selected
    /// text the ink of the label's own `selection` node, and dropping the selection takes
    /// it away — even once focus has left the cell. Mutation: inking the whole link rather than its
    /// selected part, or forgetting the last-inked cell, fails this.
    #[gtktest::test]
    fn the_selected_part_of_a_cell_link_takes_the_selection_ink() {
        let label = link_cell("link");
        let ink = rgb(probe_ink(
            label.upcast_ref(),
            &[glib::Object::new::<SelectionNodeProbe>().upcast()],
        ));
        let window = gtk::Window::new();
        window.set_child(Some(&label));
        GtkWindowExt::set_focus(&window, Some(&label));
        // "a link b": the link is bytes 2..6; select from "in" (char 3) to the end.
        label.select_region(3, 8);
        refresh_cell_selection_ink(&label);
        assert_eq!(foregrounds(label.attributes()), vec![(3..8, ink)]);
        // Focus elsewhere: the cell that took the ink is still the one given it back.
        GtkWindowExt::set_focus(&window, None::<&gtk::Widget>);
        label.select_region(0, 0);
        refresh_cell_selection_ink(&label);
        assert!(foregrounds(label.attributes()).is_empty());
        window.destroy();
    }

    /// Case 2: a body selection spanning the table inks every body cell's whole text in
    /// the BODY's selection ink, leaves a header cell alone, and clearing it leaves no
    /// ink anywhere in the cell's layout.
    ///
    /// The last assertion is the GTK 4.6 trap `cellattrs::force_cell_repaint` exists for:
    /// on a cell without a link, clearing the attributes left the old foreground in the
    /// layout and on screen (MEASURED; GTK4Rs/AP-45). Mutation: dropping that repaint
    /// fails it.
    #[gtktest::test]
    fn a_body_selection_over_a_table_inks_its_body_cells_and_clears_cleanly() {
        let head = cell_markup_label("Head");
        head.add_css_class("cell-head");
        let plain = cell_markup_label("plain <b>bold</b>");
        let linked = link_cell("link");
        let table = ScribTableWidget::new(vec![
            vec![head.clone().upcast()],
            vec![plain.clone().upcast()],
            vec![linked.clone().upcast()],
        ]);
        // Parented directly and unparented below: the cells only need a text view
        // ANCESTOR to read the body's ink from, and `add_overlay` leaves a child the view
        // cannot `remove` (it warns that the table is not its child).
        let view = gtk::TextView::new();
        table.set_parent(&view);
        let ink = rgb(probe_ink(
            view.upcast_ref(),
            &[
                glib::Object::new::<TextNodeProbe>().upcast(),
                glib::Object::new::<SelectionNodeProbe>().upcast(),
            ],
        ));

        table.set_in_body_selection(super::body_selection_ink(&table));
        let whole = |l: &Label| u32::try_from(l.text().len()).expect("short text");
        assert_eq!(
            foregrounds(plain.attributes()),
            vec![(0..whole(&plain), ink)]
        );
        assert_eq!(
            foregrounds(linked.attributes()),
            vec![(0..whole(&linked), ink)]
        );
        assert!(
            head.attributes().is_none(),
            "a header cell's ink sits on its own fill and must not change"
        );

        // Lay both cells out while inked, as a paint would — the trap needs a layout
        // to have been built with the ink in it.
        let _ = (plain.layout(), linked.layout());

        table.set_in_body_selection(None);
        for cell in [&plain, &linked] {
            assert!(cell.attributes().is_none());
            assert!(
                !foregrounds(cell.layout().attributes())
                    .iter()
                    .any(|(_, c)| *c == ink),
                "{:?} still carries the selection ink in its layout",
                cell.text()
            );
        }
        table.unparent();
    }

    /// Case 1, in a preview: a cell's own selected plain text takes the BODY's selected
    /// ink, not the label's `selection` node's — the two differ under System on GTK
    /// 4.6's Adwaita (white over the same pale fill the body's dark ink sits on).
    /// Mutation: probing `label > selection` instead fails this on Adwaita.
    #[gtktest::test]
    fn a_cell_selection_in_a_view_takes_the_body_selection_ink() {
        let plain = cell_markup_label("plain text");
        plain.set_selectable(true);
        let view = gtk::TextView::new();
        plain.set_parent(&view);
        let window = gtk::Window::new();
        window.set_child(Some(&view));
        let body = rgb(super::body_selection_ink(&plain).expect("inside a view"));
        GtkWindowExt::set_focus(&window, Some(&plain));
        plain.select_region(0, 5);
        refresh_cell_selection_ink(&plain);
        assert_eq!(foregrounds(plain.attributes()), vec![(0..5, body)]);
        plain.select_region(0, 0);
        refresh_cell_selection_ink(&plain);
        assert!(plain.attributes().is_none());
        plain.unparent();
        window.destroy();
    }
}
