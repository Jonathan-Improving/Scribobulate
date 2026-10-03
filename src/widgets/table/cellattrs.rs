//! **The one writer of a table cell's Pango attribute overlay.**
//!
//! A `GtkLabel` has ONE attribute list (`set_attributes`), laid over its markup's own,
//! and two features draw on a cell through it: find's match washes (backgrounds,
//! `window::find`) and the selected-text ink (foregrounds, [`super::linkink`]). Each
//! writing the list directly would erase the other's — a find wash vanishing the moment
//! a cell's selection is inked, and the ink vanishing when find clears — so each owns a
//! LAYER here, and the label is given their composition. `clippy.toml` bans the raw
//! setter everywhere else.
//!
//! Every write ends in [`force_cell_repaint`], because a cell sits inside an anchored
//! widget and `GtkTextView` does not repaint an anchored child when a descendant's
//! attributes only shrink or go away (GTK4Rs/AP-45): without it a cleared wash, or a
//! cleared ink, stays on screen.

use crate::saferizer::qdata_key::QdataKey;
use gtk::pango;
use gtk::Label;
use std::cell::RefCell;
use std::rc::Rc;

/// Which feature a layer belongs to. Composed in this order, so a later layer wins
/// where both set the same attribute (they do not today: find sets backgrounds, the
/// ink sets foregrounds).
#[derive(Clone, Copy)]
pub(crate) enum Layer {
    /// `window::find`'s match washes.
    Find,
    /// The selected-text ink ([`super::linkink`]).
    Ink,
}

#[derive(Default)]
struct Layers {
    find: Option<pango::AttrList>,
    ink: Option<pango::AttrList>,
}

impl Layers {
    fn slot(&mut self, layer: Layer) -> &mut Option<pango::AttrList> {
        match layer {
            Layer::Find => &mut self.find,
            Layer::Ink => &mut self.ink,
        }
    }

    /// The composed list, or `None` when no layer carries anything.
    fn composed(&self) -> Option<pango::AttrList> {
        let layers = [&self.find, &self.ink];
        if layers.iter().all(|l| l.is_none()) {
            return None;
        }
        let out = pango::AttrList::new();
        for list in layers.into_iter().flatten() {
            for attr in list.attributes() {
                out.insert(attr);
            }
        }
        Some(out)
    }
}

const LAYERS: QdataKey<Rc<RefCell<Layers>>> = QdataKey::new("scrib-cell-attr-layers");

/// Whether `label` currently carries anything on `layer`.
pub(crate) fn has_layer(label: &Label, layer: Layer) -> bool {
    LAYERS
        .get(label)
        .is_some_and(|layers| layers.borrow_mut().slot(layer).is_some())
}

/// Replace `layer` on `label` with `list` (`None` clears it), give the label the
/// composition of every layer, and force the repaint a removal needs.
pub(crate) fn set_layer(label: &Label, layer: Layer, list: Option<pango::AttrList>) {
    let layers = LAYERS.get(label).unwrap_or_else(|| {
        let fresh = Rc::new(RefCell::new(Layers::default()));
        LAYERS.set(label, fresh.clone());
        fresh
    });
    *layers.borrow_mut().slot(layer) = list;
    let composed = layers.borrow().composed();
    #[expect(clippy::disallowed_methods)] // The one sanctioned writer — see the module header.
    label.set_attributes(composed.as_ref());
    force_cell_repaint(label);
}

/// Force a `GtkTextView`-anchored cell `GtkLabel` to re-snapshot after its overlay was
/// added, recoloured, or removed. A `set_attributes` change that shrinks or removes ink
/// does NOT repaint an anchored child on its own (GTK4Rs/AP-45), and a same-string
/// `set_markup` is a no-op (GTK4Rs/AP-92). Toggle a transient no-attr `<span>` wrapper: a
/// markup string that differs (so the child re-snapshots) but renders pixel-identically
/// — no glyphs, no size change, so no reflow and no scroll shift — then revert to the
/// clean markup so no wrapper accumulates. `set_markup` does not clear a
/// `set_attributes` overlay (ScrAP-36), so the layers just applied survive the toggle,
/// and it keeps the label's own selection (measured), so an ink applied mid-drag does
/// not end the drag's selection.
fn force_cell_repaint(label: &Label) {
    let markup = label.label();
    label.set_markup(&format!("<span>{markup}</span>"));
    label.set_markup(markup.as_str());
}

#[cfg(test)]
mod tests {
    use super::{Layer, Layers};
    use gtk::pango;

    fn one(attr: pango::Attribute) -> Option<pango::AttrList> {
        let list = pango::AttrList::new();
        list.insert(attr);
        Some(list)
    }

    #[test]
    fn the_composition_holds_every_layer_in_order_and_nothing_when_all_are_clear() {
        let mut layers = Layers::default();
        assert!(layers.composed().is_none());
        *layers.slot(Layer::Ink) = one(pango::AttrColor::new_foreground(0, 0, 0).into());
        *layers.slot(Layer::Find) = one(pango::AttrColor::new_background(0, 0, 0).into());
        let kinds: Vec<_> = layers
            .composed()
            .expect("two layers")
            .attributes()
            .iter()
            .map(|a| a.type_())
            .collect();
        assert_eq!(
            kinds,
            vec![pango::AttrType::Background, pango::AttrType::Foreground]
        );
        *layers.slot(Layer::Find) = None;
        *layers.slot(Layer::Ink) = None;
        assert!(layers.composed().is_none());
    }
}

#[cfg(all(test, feature = "gtk-integration-tests"))]
mod gtk_integration_tests {
    use super::{has_layer, set_layer, Layer};
    use gtk::{pango, Label};

    fn list(attr: pango::Attribute) -> pango::AttrList {
        let list = pango::AttrList::new();
        list.insert(attr);
        list
    }

    fn kinds(label: &Label) -> Vec<pango::AttrType> {
        label
            .attributes()
            .map(|l| l.attributes().iter().map(|a| a.type_()).collect())
            .unwrap_or_default()
    }

    /// Find's wash and the selection ink share one cell without erasing each other, and
    /// clearing one leaves the other. Mutation: writing a layer's list straight to the
    /// label (last writer wins) fails the first assertion.
    #[gtktest::test]
    fn a_find_wash_and_a_selection_ink_compose_on_one_cell() {
        let label = Label::new(Some("alpha beta"));
        set_layer(
            &label,
            Layer::Find,
            Some(list(
                pango::AttrColor::new_background(0, 0, u16::MAX).into(),
            )),
        );
        set_layer(
            &label,
            Layer::Ink,
            Some(list(
                pango::AttrColor::new_foreground(u16::MAX, 0, 0).into(),
            )),
        );
        assert_eq!(
            kinds(&label),
            vec![pango::AttrType::Background, pango::AttrType::Foreground]
        );
        set_layer(&label, Layer::Ink, None);
        assert_eq!(kinds(&label), vec![pango::AttrType::Background]);
        assert!(has_layer(&label, Layer::Find) && !has_layer(&label, Layer::Ink));
        set_layer(&label, Layer::Find, None);
        assert!(label.attributes().is_none());
    }
}
