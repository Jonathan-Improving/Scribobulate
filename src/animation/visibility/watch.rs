//! The wiring half of [`super`]: re-ask [`super::current`] from exactly the
//! signals that can change its answer, coalescing the chatty one onto one idle.
//!
//! # Why every reference back to `host` (and its ancestors) is weak
//!
//! `host` is the `GtkPicture` an `AnimatedPaintable` is set on, and this watch is
//! owned by that same paintable (POLICY weak-capture rule, ScrAP-60/ScrAP-155):
//! `host → paintable → VisibilityWatch`. If [`VisibilityWatch`] (or a closure it
//! installs) held `host` STRONGLY, the cycle would close —
//! `host → paintable → watch → host` — and every picture that ever showed an
//! animation would leak its whole ancestor-to-host chain forever. The same
//! applies to the `GtkScrolledWindow`/`GtkAdjustment`/`GdkToplevel` this module
//! resolves from `host`'s own tree: none of them points back to `host` on its
//! own, but a strong hold on any of them, kept for as long as the paintable
//! lives, would keep the WHOLE PREVIEW PANE alive past the tab that owns it.
//! Every held reference here is a [`glib::WeakRef`], upgraded only for the
//! instant a callback or a disconnect needs it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use super::current;

/// The two signal handlers this module installs on one `GtkAdjustment` axis
/// (`value-changed` and `changed`, per the plan: "coalesced onto ONE idle").
struct AdjustmentHandlers {
    h: glib::WeakRef<gtk::Adjustment>,
    h_value: glib::SignalHandlerId,
    h_changed: glib::SignalHandlerId,
    v: glib::WeakRef<gtk::Adjustment>,
    v_value: glib::SignalHandlerId,
    v_changed: glib::SignalHandlerId,
}

/// State shared between [`VisibilityWatch`] (which tears it down) and the
/// closures it installs (which read and, for the ancestor lookups, populate
/// it). Each field owns its own interior mutability rather than wrapping the
/// whole struct in one `RefCell`, so a closure firing from inside another
/// closure's borrow (unlikely here, but cheap to rule out) cannot panic on a
/// double borrow.
struct Shared {
    f: Rc<dyn Fn(bool)>,
    /// Coalescing flag for the adjustment signals — set the instant the first
    /// one fires, cleared when the idle it scheduled actually runs, so however
    /// many `value-changed`/`changed` signals arrive before the idle is
    /// serviced, [`current`] is asked exactly once.
    idle_scheduled: Cell<bool>,
    /// `None` until an ancestor `GtkScrolledWindow` is found — an anchored
    /// picture's ancestor chain does not change over its life (POLICY: a
    /// collapsed `<details>` around it is spliced away entirely, not
    /// reparented — see the module doc on claim 2), so this resolves at most
    /// once, retried from `host`'s own `map` signal in case the chain was not
    /// yet walkable when [`watch`] was first called.
    adjustments: RefCell<Option<AdjustmentHandlers>>,
    /// `None` until `host`'s toplevel surface exists — realize-time, so this
    /// too is retried from `map` (a widget is never mapped before its window
    /// is realized).
    toplevel: RefCell<Option<(glib::WeakRef<gtk::gdk::Toplevel>, glib::SignalHandlerId)>>,
}

/// A live subscription to every signal that can change [`super::current`]'s
/// answer for one host widget. Dropping it disconnects everything it
/// installed — see the module doc on why every held reference is weak.
pub(crate) struct VisibilityWatch {
    host: glib::WeakRef<gtk::Widget>,
    map_id: Option<glib::SignalHandlerId>,
    unmap_id: Option<glib::SignalHandlerId>,
    shared: Rc<Shared>,
}

impl Drop for VisibilityWatch {
    fn drop(&mut self) {
        if let Some(host) = self.host.upgrade() {
            if let Some(id) = self.map_id.take() {
                host.disconnect(id);
            }
            if let Some(id) = self.unmap_id.take() {
                host.disconnect(id);
            }
        }
        if let Some(adj) = self.shared.adjustments.borrow_mut().take() {
            if let Some(h) = adj.h.upgrade() {
                h.disconnect(adj.h_value);
                h.disconnect(adj.h_changed);
            }
            if let Some(v) = adj.v.upgrade() {
                v.disconnect(adj.v_value);
                v.disconnect(adj.v_changed);
            }
        }
        if let Some((toplevel, id)) = self.shared.toplevel.borrow_mut().take() {
            if let Some(t) = toplevel.upgrade() {
                t.disconnect(id);
            }
        }
    }
}

/// Watch every signal that can change whether `host` is visible, calling
/// `f(new_visible)` whenever one fires. Does **not** call `f` itself at setup
/// — mirrors [`super::super::policy::watch`]'s split: a caller that also needs
/// the value right now asks [`super::current`] directly (this is what
/// `AnimatedPaintable::try_bootstrap` does, so a picture built already
/// scrolled away or backgrounded does not autoplay for one recompute before
/// anything else says otherwise).
pub(crate) fn watch(host: &gtk::Widget, f: impl Fn(bool) + 'static) -> VisibilityWatch {
    let shared = Rc::new(Shared {
        f: Rc::new(f),
        idle_scheduled: Cell::new(false),
        adjustments: RefCell::new(None),
        toplevel: RefCell::new(None),
    });

    resolve_adjustments(host, &shared);
    resolve_toplevel(host, &shared);

    let map_id = host.connect_map(glib::clone!(
        #[strong]
        shared,
        move |w| {
            resolve_adjustments(w, &shared);
            resolve_toplevel(w, &shared);
            recompute_now(w, &shared);
        }
    ));
    let unmap_id = host.connect_unmap(glib::clone!(
        #[strong]
        shared,
        move |w| recompute_now(w, &shared)
    ));

    VisibilityWatch {
        host: host.downgrade(),
        map_id: Some(map_id),
        unmap_id: Some(unmap_id),
        shared,
    }
}

/// Recompute and report immediately — for `map`/`unmap`, which are already
/// low-frequency lifecycle events, not the scroll-driven chatter
/// [`schedule_recompute`] exists to coalesce.
fn recompute_now(host: &gtk::Widget, shared: &Rc<Shared>) {
    (shared.f)(current(host));
}

/// Schedule one coalesced recompute on the next main-context idle, unless one
/// is already pending. The plan's own wording: "coalesced onto ONE idle (do
/// not do work per scroll event)".
fn schedule_recompute(host: &gtk::Widget, shared: &Rc<Shared>) {
    if shared.idle_scheduled.get() {
        return;
    }
    shared.idle_scheduled.set(true);
    let host = host.downgrade();
    let shared = Rc::clone(shared);
    glib::idle_add_local_once(move || {
        shared.idle_scheduled.set(false);
        if let Some(host) = host.upgrade() {
            (shared.f)(current(&host));
        }
    });
}

/// Find `host`'s ancestor `GtkScrolledWindow` (if any — a test fixture, or a
/// future host this renderer does not anchor inside one, has none) and wire
/// both axes' `value-changed` AND `changed`, each coalesced through
/// [`schedule_recompute`]. A no-op if already resolved.
fn resolve_adjustments(host: &gtk::Widget, shared: &Rc<Shared>) {
    if shared.adjustments.borrow().is_some() {
        return;
    }
    let Some(view) = host
        .ancestor(gtk::ScrolledWindow::static_type())
        .and_then(|w| w.downcast::<gtk::ScrolledWindow>().ok())
    else {
        return;
    };
    let hadj = view.hadjustment();
    let vadj = view.vadjustment();
    let h_value = hadj.connect_value_changed(adjustment_handler(host, shared));
    let h_changed = hadj.connect_changed(adjustment_handler(host, shared));
    let v_value = vadj.connect_value_changed(adjustment_handler(host, shared));
    let v_changed = vadj.connect_changed(adjustment_handler(host, shared));
    shared.adjustments.replace(Some(AdjustmentHandlers {
        h: hadj.downgrade(),
        h_value,
        h_changed,
        v: vadj.downgrade(),
        v_value,
        v_changed,
    }));
}

fn adjustment_handler(
    host: &gtk::Widget,
    shared: &Rc<Shared>,
) -> impl Fn(&gtk::Adjustment) + 'static {
    let host = host.downgrade();
    let shared = Rc::clone(shared);
    move |_adj| {
        if let Some(host) = host.upgrade() {
            schedule_recompute(&host, &shared);
        }
    }
}

/// Find `host`'s toplevel surface (if realized yet) and wire `notify::state`
/// for minimize/restore. A no-op if already resolved.
fn resolve_toplevel(host: &gtk::Widget, shared: &Rc<Shared>) {
    if shared.toplevel.borrow().is_some() {
        return;
    }
    let host_weak = host.downgrade();
    let shared_for_handler = Rc::clone(shared);
    let Some((toplevel, id)) = super::connect_toplevel_state(host, move |_toplevel| {
        if let Some(host) = host_weak.upgrade() {
            (shared_for_handler.f)(current(&host));
        }
    }) else {
        return;
    };
    shared.toplevel.replace(Some((toplevel.downgrade(), id)));
}
