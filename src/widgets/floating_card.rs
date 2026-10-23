//! `FloatingCard` — a small card that floats over a pane INSIDE the window, pointing at
//! something in that pane. It is the one way this application shows a transient,
//! selection- or chip-anchored card (the preview's Annotate action, the editor's
//! formatting bar, an annotation's comment card).
//!
//! # Why not a `GtkPopover`
//!
//! A `GtkPopover` is its own window-system surface, and on two of the three platforms
//! that surface is not tied to the window it belongs to:
//!
//!  * **X11 (GTK 4.6.9).** GDK creates every popup as an override-redirect child of the
//!    ROOT window (`gdksurface-x11.c`: `xparent = GDK_SCREEN_XROOTWIN`; `CWOverrideRedirect`
//!    set unconditionally for `GDK_SURFACE_POPUP`) and never sets `WM_TRANSIENT_FOR` on it.
//!    The window manager does not manage it at all, so minimizing the window, switching
//!    virtual desktop, or switching KDE activity leaves the card on screen where it was.
//!    Nothing GDK does on the toplevel's minimize or unmap reaches it, and there is no
//!    public or environment lever that changes this.
//!  * **macOS (GTK 4.22.4).** The popup is a child `NSWindow` of the toplevel. It follows
//!    the window between Spaces, but is left on screen, over other applications, when the
//!    window is minimized.
//!  * Win32 owns the popup `HWND` by the toplevel, which hides with it — the only one of
//!    the three that behaves.
//!
//! A child of the pane's `GtkOverlay` is part of the window's own surface, so nothing the
//! window manager does to the window can leave it behind: there is no second window to
//! leave. That is what the comment entry card has always been, and why it never showed
//! this defect. Measured on all three platforms before the change (GTK4Rs/AP-362).
//!
//! It drops two more popover hazards with it. An in-surface card receives keys through
//! the window, so the application's accelerators keep working while it holds focus
//! (GTK4Rs/AP-266 was a popover being its own `GtkNative`), and it is never placed by
//! screen coordinates, so an anchor off the monitor cannot trip GDK's monitor lookup
//! (GTK4Rs/AP-26).
//!
//! The cost: the card is clipped to the pane it floats over. [`place`] keeps it inside,
//! moving it to the other side of its anchor when there is no room on the preferred one.
//!
//! # The API is a popover's, deliberately
//!
//! `popup`, `popdown`, `set_pointing_to` (in the ANCHOR widget's coordinates, as a
//! popover's are in its parent's), a preferred side, and a `closed` signal emitted from the
//! `hide` vfunc exactly as `GtkPopover` emits it (GTK4Rs/AP-192), so every caller carried
//! over unchanged in meaning. The one addition is [`attach`](FloatingCardExt::attach): a
//! popover needs only a parent; this card needs a parent (the pane's overlay) AND an anchor
//! (the widget whose coordinates `pointing_to` is in), which are different widgets.
//!
//! # How it is placed
//!
//! Not by margins. The host overlay asks each child where it goes on every allocation
//! (`GtkOverlay::get-child-position`), and the card answers from its anchor rectangle
//! translated into the overlay at that moment. So a pane resize re-places it with no help,
//! and a new anchor needs only a `queue_resize` — not `queue_allocate`, which GTK 4.6 can
//! silently drop (GTK4Rs/AP-104). Reading geometry there is safe: it measures only the
//! card's own content and translates between two widgets the overlay is allocating; it
//! never asks a `GtkTextView` to validate anything (GTK4Rs/AP-22).

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

/// Gap in px between the card and the rectangle it points at.
const GAP: i32 = 4;

/// Which side of its anchor a card prefers. A closed set, so the placement arithmetic has
/// no "other" case to forget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    /// Above the anchor, centred on it; below when there is no room above.
    Top,
    /// Left of the anchor, centred on it vertically; right when there is no room left.
    Left,
}

/// An axis-aligned rectangle in the HOST overlay's coordinates — plain data, so the
/// placement rule is testable without a display.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rect {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) w: i32,
    pub(crate) h: i32,
}

/// Where a `card_w` × `card_h` card goes, pointing at `anchor`, inside a `host_w` ×
/// `host_h` overlay: its top-left corner.
///
/// On the preferred side when it fits; on the opposite side when it does not; and in every
/// case clamped so no part of it is outside the host (the host clips it, so a card hanging
/// off an edge would simply be cut). A card larger than the host is pinned to the top-left,
/// which shows its start rather than its middle.
pub(crate) fn place(
    anchor: Rect,
    card_w: i32,
    card_h: i32,
    host_w: i32,
    host_h: i32,
    side: Side,
) -> (i32, i32) {
    let max_x = (host_w - card_w).max(0);
    let max_y = (host_h - card_h).max(0);
    let (x, y) = match side {
        Side::Top => {
            let above = anchor.y - GAP - card_h;
            let y = if above >= 0 {
                above
            } else {
                anchor.y + anchor.h + GAP
            };
            (anchor.x + anchor.w / 2 - card_w / 2, y)
        }
        Side::Left => {
            let left = anchor.x - GAP - card_w;
            let x = if left >= 0 {
                left
            } else {
                anchor.x + anchor.w + GAP
            };
            (x, anchor.y + anchor.h / 2 - card_h / 2)
        }
    };
    (x.clamp(0, max_x), y.clamp(0, max_y))
}

mod imp {
    use super::*;
    use glib::subclass::Signal;
    use std::cell::{Cell, RefCell};
    use std::sync::OnceLock;

    pub(crate) struct FloatingCard {
        /// The widget `pointing_to` is measured in. Weak: the anchor owns the content the
        /// card is about, never the other way round.
        pub(super) anchor: glib::WeakRef<gtk::Widget>,
        /// The overlay the card is a child of, and our `get-child-position` handler on it.
        pub(super) host: RefCell<Option<(glib::WeakRef<gtk::Overlay>, glib::SignalHandlerId)>>,
        /// `(x, y, width, height)` in the anchor's coordinates.
        pub(super) pointing_to: Cell<Option<(i32, i32, i32, i32)>>,
        pub(super) side: Cell<Side>,
        /// The card's content, held so `dispose` can unparent it.
        pub(super) child: RefCell<Option<gtk::Widget>>,
    }

    impl Default for FloatingCard {
        fn default() -> Self {
            Self {
                anchor: Default::default(),
                host: Default::default(),
                pointing_to: Default::default(),
                side: Cell::new(Side::Top),
                child: Default::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FloatingCard {
        const NAME: &'static str = "ScribFloatingCard";
        type Type = super::FloatingCard;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();
            klass.set_css_name("floatingcard");
        }
    }

    impl ObjectImpl for FloatingCard {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| vec![Signal::builder("closed").build()])
        }

        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            // Positioned by the host's `get-child-position`; the alignment only matters
            // before the first anchor arrives, and START keeps an unanchored card from
            // filling the pane.
            obj.set_halign(gtk::Align::Start);
            obj.set_valign(gtk::Align::Start);
            obj.set_visible(false);
        }

        fn dispose(&self) {
            self.obj().disconnect_host();
            if let Some(child) = self.child.borrow_mut().take() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for FloatingCard {
        /// `closed` from `hide`, as `GtkPopover` does — every hide emits it, a caller's own
        /// included (GTK4Rs/AP-192).
        fn hide(&self) {
            self.parent_hide();
            self.obj().emit_by_name::<()>("closed", &[]);
        }
    }
}

glib::wrapper! {
    pub(crate) struct FloatingCard(ObjectSubclass<imp::FloatingCard>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

/// Subclassing hook: a widget that IS a floating card (the annotation comment card).
pub(crate) trait FloatingCardImpl:
    WidgetImpl + ObjectSubclass<Type: IsA<FloatingCard>>
{
}

// SAFETY: `FloatingCard` adds no class struct or vfuncs of its own beyond `GtkWidget`'s,
// so a subclass needs nothing initialised beyond what `WidgetImpl` already sets up.
unsafe impl<T: FloatingCardImpl> IsSubclassable<T> for FloatingCard {}

impl Default for FloatingCard {
    fn default() -> Self {
        Self::new()
    }
}

impl FloatingCard {
    pub(crate) fn new() -> Self {
        glib::Object::new()
    }

    fn disconnect_host(&self) {
        if let Some((weak, id)) = self.imp().host.borrow_mut().take() {
            if let Some(host) = weak.upgrade() {
                host.disconnect(id);
            }
        }
    }

    /// The card's rectangle in `host`, answered from the host's own allocation pass.
    /// `None` (the overlay's default placement) when there is nothing to point at.
    fn position_in(&self, host: &gtk::Overlay) -> Option<gdk::Rectangle> {
        let (ax, ay, aw, ah) = self.imp().pointing_to.get()?;
        let anchor = self.imp().anchor.upgrade()?;
        let (hx, hy) = anchor.translate_coordinates(host, f64::from(ax), f64::from(ay))?;
        let (width, height) = self.size_within(host.width());
        let (x, y) = place(
            Rect {
                x: hx as i32,
                y: hy as i32,
                w: aw,
                h: ah,
            },
            width,
            height,
            host.width(),
            host.height(),
            self.imp().side.get(),
        );
        Some(gdk::Rectangle::new(x, y, width, height))
    }

    /// The card's size inside a host `host_w` wide: its natural width when that fits, else
    /// the host's width (never below its own minimum), with the height its content needs
    /// at that width — so content that can wrap, wraps, rather than being cut off by the
    /// host's clip.
    fn size_within(&self, host_w: i32) -> (i32, i32) {
        let (min_w, nat_w, _, _) = self.measure(gtk::Orientation::Horizontal, -1);
        let width = nat_w.min(host_w).max(min_w);
        let (_, nat_h, _, _) = self.measure(gtk::Orientation::Vertical, width);
        (width, nat_h)
    }
}

/// The card's operations, on the card and on every subclass of it.
pub(crate) trait FloatingCardExt: IsA<FloatingCard> + 'static {
    /// Put the card's content in. Replaces any previous content.
    fn set_child(&self, child: Option<&impl IsA<gtk::Widget>>) {
        let card = self.as_ref();
        if let Some(old) = card.imp().child.borrow_mut().take() {
            old.unparent();
        }
        if let Some(child) = child {
            let child = child.as_ref().clone();
            child.set_parent(card);
            card.imp().child.replace(Some(child));
        }
    }

    /// Float the card over `host`, pointing in `anchor`'s coordinates. Idempotent for the
    /// same host; moving to another host removes it from the previous one first (a card
    /// that follows the active tab is re-attached on every tab switch).
    fn attach(&self, host: &gtk::Overlay, anchor: &impl IsA<gtk::Widget>) {
        let card = self.as_ref();
        card.imp().anchor.set(Some(anchor.as_ref()));
        if card.host().as_ref() == Some(host) {
            return;
        }
        card.detach();
        host.add_overlay(card);
        // Clipped to the pane, and never part of its size request: a card must not be able
        // to grow the pane it floats over.
        host.set_clip_overlay(card, true);
        host.set_measure_overlay(card, false);
        let weak = card.downgrade();
        let id = host.connect_get_child_position(move |host, child| {
            let card = weak.upgrade()?;
            if child != card.upcast_ref::<gtk::Widget>() {
                return None;
            }
            card.position_in(host)
        });
        card.imp().host.replace(Some((host.downgrade(), id)));
    }

    /// Take the card off its host, closing it first. A no-op when it has none.
    fn detach(&self) {
        let card = self.as_ref();
        card.popdown();
        let host = card.host();
        card.disconnect_host();
        if let Some(host) = host {
            host.remove_overlay(card);
        }
    }

    /// The overlay the card floats over, if attached.
    fn host(&self) -> Option<gtk::Overlay> {
        self.as_ref()
            .imp()
            .host
            .borrow()
            .as_ref()
            .and_then(|(weak, _)| weak.upgrade())
    }

    /// The widget `pointing_to` is measured in, if attached and alive.
    fn anchor(&self) -> Option<gtk::Widget> {
        self.as_ref().imp().anchor.upgrade()
    }

    /// Point at `rect`, in the anchor's coordinates; `None` clears it.
    fn set_pointing_to(&self, rect: Option<&gdk::Rectangle>) {
        let card = self.as_ref();
        card.imp()
            .pointing_to
            .set(rect.map(|r| (r.x(), r.y(), r.width(), r.height())));
        card.queue_resize();
    }

    /// What the card is pointing at, in the anchor's coordinates, or `None` when unset.
    fn pointing_to(&self) -> Option<gdk::Rectangle> {
        self.as_ref()
            .imp()
            .pointing_to
            .get()
            .map(|(x, y, w, h)| gdk::Rectangle::new(x, y, w, h))
    }

    /// Which side of its anchor the card prefers.
    fn set_side(&self, side: Side) {
        self.as_ref().imp().side.set(side);
        self.as_ref().queue_resize();
    }

    /// Show the card. A no-op when it is not attached, so a card whose pane has gone cannot
    /// be shown into nothing.
    fn popup(&self) {
        let card = self.as_ref();
        if card.host().is_none() {
            return;
        }
        card.set_visible(true);
        card.queue_resize();
    }

    /// Hide the card; emits `closed`.
    fn popdown(&self) {
        self.as_ref().set_visible(false);
    }

    /// Run `f` whenever the card hides, for whatever reason.
    fn connect_closed<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        // Weak: the handler lives on the card itself, so a strong capture would be a cycle.
        let weak = self.downgrade();
        self.as_ref().connect_local("closed", false, move |_| {
            if let Some(this) = weak.upgrade() {
                f(&this);
            }
            None
        })
    }

    /// Whether the press `gesture` is handling right now, at `(x, y)` in the gesture's
    /// widget, landed on this card. For an outside-click dismissal on an ancestor: a press
    /// on the card's own buttons reaches an ancestor's capture-phase gesture first, and must
    /// not count as "outside".
    fn contains_press(&self, gesture: &impl IsA<gtk::Gesture>, x: f64, y: f64) -> bool {
        let card: &gtk::Widget = self.as_ref().upcast_ref();
        let Some(widget) = gesture.as_ref().widget() else {
            return false;
        };
        widget
            .pick(x, y, gtk::PickFlags::DEFAULT)
            .is_some_and(|hit| &hit == card || hit.is_ancestor(card))
    }
}

impl<T: IsA<FloatingCard>> FloatingCardExt for T {}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn top_sits_above_the_anchor_centred_on_it() {
        assert_eq!(
            place(r(100, 200, 40, 20), 60, 30, 800, 600, Side::Top),
            (90, 200 - GAP - 30)
        );
    }

    #[test]
    fn top_moves_below_when_there_is_no_room_above() {
        assert_eq!(
            place(r(100, 10, 40, 20), 60, 30, 800, 600, Side::Top),
            (90, 10 + 20 + GAP)
        );
    }

    #[test]
    fn left_sits_left_of_the_anchor_centred_on_it() {
        assert_eq!(
            place(r(300, 200, 20, 20), 100, 40, 800, 600, Side::Left),
            (300 - GAP - 100, 190)
        );
    }

    #[test]
    fn left_moves_right_when_there_is_no_room_left() {
        assert_eq!(
            place(r(50, 200, 20, 20), 100, 40, 800, 600, Side::Left),
            (50 + 20 + GAP, 190)
        );
    }

    #[test]
    fn a_card_never_hangs_off_any_edge() {
        // Anchor at the far right and bottom: clamped back inside on both axes.
        let (x, y) = place(r(790, 590, 10, 10), 100, 50, 800, 600, Side::Top);
        assert!(
            x + 100 <= 800 && y + 50 <= 600,
            "({x}, {y}) leaves the host"
        );
        // Anchor off the top-left (negative coordinates): clamped to the origin.
        assert_eq!(
            place(r(-50, -80, 10, 10), 100, 50, 800, 600, Side::Top),
            (0, 0)
        );
    }

    #[test]
    fn a_card_larger_than_the_host_shows_its_start() {
        assert_eq!(
            place(r(100, 100, 10, 10), 900, 700, 800, 600, Side::Top),
            (0, 0)
        );
    }
}

#[cfg(all(test, feature = "gtk-integration-tests"))]
mod gtk_integration_tests {
    use super::*;
    use crate::testpump::{destroy_after_tooltip_delay, until, Clock};

    /// **A card wider than its pane wraps to fit it rather than being cut off.** The pane
    /// clips the card, so a one-row card wider than the pane would leave its last buttons
    /// unreachable — measured on Windows with the editor's formatting bar in a split.
    /// Mutation: size the card at its natural width in `size_within` and this fails.
    #[gtktest::test]
    fn a_card_wider_than_its_pane_wraps_inside_it() {
        let win = gtk::Window::new();
        let host = gtk::Overlay::new();
        let anchor = gtk::Box::new(gtk::Orientation::Vertical, 0);
        host.set_child(Some(&anchor));
        host.set_size_request(300, 300);
        win.set_child(Some(&host));
        win.set_default_size(300, 300);
        win.set_resizable(false);

        let row = crate::widgets::wrapbox::ToolbarWrapBox::new(2, 2);
        let buttons: Vec<gtk::Button> = (0..20)
            .map(|i| gtk::Button::with_label(&format!("B{i}")))
            .collect();
        for b in &buttons {
            row.append(b);
        }
        let card = FloatingCard::new();
        card.set_child(Some(&row));
        card.attach(&host, &anchor);
        card.set_pointing_to(Some(&gdk::Rectangle::new(150, 200, 1, 16)));
        win.present();
        card.popup();
        until(Clock::Frame, "the card to be allocated", || {
            card.is_mapped() && card.width() > 0
        });
        assert!(
            row.preferred_size().1.width() > host.width(),
            "precondition: the card's one-row width must exceed the pane, or nothing is \
             tested"
        );
        let bounds = card
            .compute_bounds(&host)
            .expect("the card is inside its pane");
        assert!(
            bounds.x() >= 0.0 && bounds.x() + bounds.width() <= host.width() as f32,
            "the card must fit inside its pane: {bounds:?} in a {} px pane",
            host.width()
        );
        for b in &buttons {
            let bb = b.compute_bounds(&host).expect("each button is laid out");
            assert!(
                bb.x() + bb.width() <= host.width() as f32,
                "every button must be inside the pane"
            );
        }
        destroy_after_tooltip_delay(&win, || card.popdown());
    }
}
