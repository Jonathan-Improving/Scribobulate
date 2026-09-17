//! The live-subscription half of [`super`]'s policy: [`PolicyWatch`] and [`watch`]
//! connect the three independent change signals `super::current` reads (the reader's
//! action, `gtk-enable-animations`, and the platform reduced-motion façade) to one
//! re-computed callback, plus the `gtk-integration-tests` support
//! ([`EnableAnimationsGuard`]) and coverage that exercises it.
//!
//! Split out of `policy`'s single file once it crossed POLICY's 500-line soft limit —
//! see that module's doc comment for why the split falls here rather than somewhere
//! else. Everything here reaches back into `super` for the pure decision core
//! (`effective_play`, `choice_of`, `enabled_of`, `reduced_motion_of`) exactly as it did
//! before the split; nothing in `super` reaches forward into this file.

use gtk::glib;
use gtk::prelude::*;

use super::{action, choice_of, effective_play, enabled_of, reduced_motion_of};

/// A live subscription to the effective play state. Fires `f(new_effective)` whenever
/// `app.play-animations`'s state, `gtk-enable-animations`, OR the platform's
/// reduced-motion source changes, always recomputed from all three live sources at
/// the moment of firing (never a stale value threaded through the notify signal's own
/// arguments), so a callback never sees a state [`super::current`] itself would
/// disagree with.
///
/// Dropping the [`PolicyWatch`] disconnects all three. A playing animation's lifetime
/// is shorter than the application's or the desktop's settings object, so without
/// this a subscriber outliving its own picture would be a leak (POLICY weak-capture
/// rule, ScrAP-60/ScrAP-155) — every picture that calls [`watch`] holds the returned
/// guard for exactly as long as it needs the callback, and no longer.
pub(crate) struct PolicyWatch {
    action: Option<gtk::gio::Action>,
    action_handler: Option<glib::SignalHandlerId>,
    settings: Option<gtk::Settings>,
    settings_handler: Option<glib::SignalHandlerId>,
    // Held ONLY to be dropped — hence the underscore rather than an `#[allow]`: the
    // platform subscription tears itself down in its own `Drop`, so this field is a
    // guard and nothing ever reads it. Not an `Option` either:
    // `platform::watch_reduced_motion` always returns a guard, even where the
    // underlying subscription is a no-op (POLICY § Platform seams — the façade never
    // asks a caller to branch on platform), so there is nothing to `Option`-wrap the
    // way the two GTK-object-backed handlers above need to be.
    _reduced_motion: crate::platform::ReducedMotionWatch,
}

impl Drop for PolicyWatch {
    fn drop(&mut self) {
        if let (Some(action), Some(id)) = (self.action.take(), self.action_handler.take()) {
            action.disconnect(id);
        }
        if let (Some(settings), Some(id)) = (self.settings.take(), self.settings_handler.take()) {
            settings.disconnect(id);
        }
        // `_reduced_motion`'s own `Drop` runs the platform's teardown; nothing to do
        // for it here beyond letting the field drop normally.
    }
}

pub(crate) fn watch(app: &gtk::Application, f: impl Fn(bool) + 'static) -> PolicyWatch {
    let action = action(app);
    let settings = gtk::Settings::default();

    // Recomputed from ALL THREE live sources on every firing (never a value carried
    // in the notify signal's own argument), so `watch`'s callback and a fresh call to
    // `current()` can never disagree about "the current effective state".
    let emit: std::rc::Rc<dyn Fn()> = {
        let action = action.clone();
        let settings = settings.clone();
        std::rc::Rc::new(move || {
            f(effective_play(
                choice_of(action.as_ref()),
                enabled_of(settings.as_ref()),
                reduced_motion_of(crate::platform::system_reduced_motion()),
            ));
        })
    };

    let action_handler = action.as_ref().map(|a| {
        let emit = emit.clone();
        a.connect_state_notify(move |_| emit())
    });
    let settings_handler = settings.as_ref().map(|s| {
        let emit = emit.clone();
        s.connect_gtk_enable_animations_notify(move |_| emit())
    });
    let reduced_motion = crate::platform::watch_reduced_motion({
        let emit = emit.clone();
        move |_| emit()
    });

    PolicyWatch {
        action,
        action_handler,
        settings,
        settings_handler,
        _reduced_motion: reduced_motion,
    }
}

/// Serializes every `gtk-integration-tests` test — in this module and elsewhere
/// (`window::lifecycle`'s own coverage of the same guard reuses this) — that
/// mutates the process-global `gtk-enable-animations` `GtkSettings` property.
/// `libtest` runs the whole gtktest suite in one process, so two tests racing this
/// property would flake exactly like `session::ENV_LOCK` guards `XDG_STATE_HOME`
/// against the same hazard (POLICY § Unit tests: "a test that installs
/// PROCESS-global state restores it before it returns").
#[cfg(all(test, feature = "gtk-integration-tests"))]
static SETTINGS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// RAII guard over `gtk-enable-animations`: [`Self::set`] records the value it
/// finds and applies the requested one; [`Drop`] always puts the original value
/// back, so an early return or a panicking assertion can never leak a forced
/// "reduce animations" state into every test that runs after this one in the same
/// process.
#[cfg(all(test, feature = "gtk-integration-tests"))]
pub(crate) struct EnableAnimationsGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    settings: gtk::Settings,
    previous: bool,
}

#[cfg(all(test, feature = "gtk-integration-tests"))]
impl EnableAnimationsGuard {
    /// Take the lock, record the current value, and force `gtk-enable-animations`
    /// to `enabled` for the guard's lifetime.
    pub(crate) fn set(enabled: bool) -> Self {
        let lock = SETTINGS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let settings = gtk::Settings::default().expect("a display exists under gtktest");
        let previous = settings.is_gtk_enable_animations();
        settings.set_gtk_enable_animations(enabled);
        Self {
            _lock: lock,
            settings,
            previous,
        }
    }

    /// Change the live value again while still holding the lock (and therefore
    /// still guaranteeing the ORIGINAL pre-`set` value is what gets restored) —
    /// for a test that needs to flip the setting mid-body, e.g. to observe a
    /// `notify::gtk-enable-animations` subscriber fire.
    pub(crate) fn set_live(&self, enabled: bool) {
        self.settings.set_gtk_enable_animations(enabled);
    }
}

#[cfg(all(test, feature = "gtk-integration-tests"))]
impl Drop for EnableAnimationsGuard {
    fn drop(&mut self) {
        self.settings.set_gtk_enable_animations(self.previous);
    }
}

#[cfg(all(test, feature = "gtk-integration-tests"))]
mod gtk_tests {
    use super::super::ACTION_NAME;
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn test_app(suffix: &str) -> gtk::Application {
        crate::window::testkit::test_app_suffixed(&format!("policy.{suffix}"))
    }

    /// Adds a bare `app.play-animations`-shaped stateful boolean action, exactly
    /// the shape `app::appactions::add_play_animations_action` registers (that
    /// function itself is `pub(super)` to `app`, and reads the real session — this
    /// mirrors its shape without either dependency, so `policy`'s own tests stay
    /// self-contained).
    fn add_bare_action(app: &gtk::Application, initial: bool) {
        let action = gtk::gio::SimpleAction::new_stateful(ACTION_NAME, None, &initial.to_variant());
        action.connect_change_state(|act, value| {
            let Some(value) = value else { return };
            act.set_state(value);
        });
        app.add_action(&action);
    }

    /// `current` is the conjunction, live, off a real action and a real
    /// `GtkSettings` (TDD 27.7): the reader can turn it off on their own, and the
    /// system setting overrides an ON choice regardless of what the reader picked.
    #[gtktest::test]
    fn current_reflects_choice_and_system_setting_together() {
        let settings = EnableAnimationsGuard::set(true);
        let app = test_app("current");
        add_bare_action(&app, true);
        assert!(super::super::current(&app), "on + system-enabled ⇒ plays");

        app.change_action_state(ACTION_NAME, &false.to_variant());
        assert!(!super::super::current(&app), "the reader turned it off");

        app.change_action_state(ACTION_NAME, &true.to_variant());
        settings.set_live(false);
        assert!(
            !super::super::current(&app),
            "system reduce-animations wins over an ON choice (TDD 27.7)"
        );
    }

    /// TDD 27.7's persistence half: `reader_choice` must answer ONLY off the
    /// action's own state, so a caller that persists it (session.rs) never bakes
    /// in the system setting's current say. Pinned as its own test rather than
    /// only inferred from `current`'s: this IS the exact guard `session.rs`
    /// depends on to keep the two independent.
    #[gtktest::test]
    fn reader_choice_ignores_the_system_setting() {
        let _settings = EnableAnimationsGuard::set(false); // reduce-animations ON
        let app = test_app("readerchoice");
        add_bare_action(&app, true); // the reader chose ON

        assert!(
            super::super::reader_choice(&app),
            "reader_choice must read ONLY the action, never the system setting"
        );
        assert!(
            !super::super::current(&app),
            "yet the EFFECTIVE state is frozen by reduce-animations — the two must diverge here"
        );
    }

    /// `watch` fires with the EFFECTIVE state on the reader's own toggle (the
    /// action's `notify::state`).
    #[gtktest::test]
    fn watch_fires_with_effective_state_on_action_toggle() {
        let _settings = EnableAnimationsGuard::set(true);
        let app = test_app("watchtoggle");
        add_bare_action(&app, true);

        let seen: Rc<RefCell<Vec<bool>>> = Rc::new(RefCell::new(Vec::new()));
        let _watch = watch(&app, {
            let seen = seen.clone();
            move |effective| seen.borrow_mut().push(effective)
        });

        app.change_action_state(ACTION_NAME, &false.to_variant());
        assert_eq!(&*seen.borrow(), &[false]);
        app.change_action_state(ACTION_NAME, &true.to_variant());
        assert_eq!(&*seen.borrow(), &[false, true]);
    }

    /// `watch` fires on the OTHER live source too: the system setting alone,
    /// with no action change at all (TDD 27.7 — this is what lets an animation
    /// react to "reduce animations" being toggled while the app is running).
    #[gtktest::test]
    fn watch_fires_on_system_setting_change_too() {
        let settings = EnableAnimationsGuard::set(true);
        let app = test_app("watchsettings");
        add_bare_action(&app, true);

        let seen: Rc<RefCell<Vec<bool>>> = Rc::new(RefCell::new(Vec::new()));
        let _watch = watch(&app, {
            let seen = seen.clone();
            move |effective| seen.borrow_mut().push(effective)
        });

        settings.set_live(false);
        assert_eq!(&*seen.borrow(), &[false]);
        settings.set_live(true);
        assert_eq!(&*seen.borrow(), &[false, true]);
    }

    /// `watch` fires on the THIRD live source too: the platform's reduced-motion
    /// façade, with no action or `GtkSettings` change at all.
    ///
    /// A test cannot flip a real OS setting on any host. So instead of faking a
    /// Windows poll or a macOS notification, this drives the subscriber list that
    /// `platform::watch_reduced_motion` keeps on EVERY platform (see
    /// `platform::listeners`) via `platform::testing::inject_reduced_motion`. What it proves
    /// is that `PolicyWatch` really did subscribe to the platform façade and
    /// re-fires when THAT source signals a change — the one wiring the façade adds
    /// — not what value it reports (Linux has no real value to report, and
    /// `current`'s own test above already covers that side separately).
    #[gtktest::test]
    fn watch_fires_on_platform_reduced_motion_change_too() {
        let _settings = EnableAnimationsGuard::set(true);
        let app = test_app("watchreducedmotion");
        add_bare_action(&app, true);

        let seen: Rc<RefCell<Vec<bool>>> = Rc::new(RefCell::new(Vec::new()));
        let _watch = watch(&app, {
            let seen = seen.clone();
            move |effective| seen.borrow_mut().push(effective)
        });

        crate::platform::testing::inject_reduced_motion(Some(true));
        assert_eq!(
            seen.borrow().len(),
            1,
            "watch must re-fire when the platform source signals a change"
        );
        crate::platform::testing::inject_reduced_motion(Some(false));
        assert_eq!(seen.borrow().len(), 2);
    }

    /// Dropping the [`PolicyWatch`] disconnects ALL THREE handlers — no further
    /// callbacks from any source. This is the leak guard itself (POLICY
    /// weak-capture rule, ScrAP-60/ScrAP-155): a picture's watch must not keep
    /// firing into a callback whose closure may capture that same picture, after
    /// the picture (and its `PolicyWatch`) are gone.
    #[gtktest::test]
    fn dropping_policy_watch_disconnects_all_three_handlers() {
        let settings = EnableAnimationsGuard::set(true);
        let app = test_app("dropwatch");
        add_bare_action(&app, true);

        let seen: Rc<RefCell<Vec<bool>>> = Rc::new(RefCell::new(Vec::new()));
        let watch_guard = watch(&app, {
            let seen = seen.clone();
            move |effective| seen.borrow_mut().push(effective)
        });
        app.change_action_state(ACTION_NAME, &false.to_variant());
        assert_eq!(
            seen.borrow().len(),
            1,
            "sanity: the watch is live before drop"
        );

        drop(watch_guard);

        app.change_action_state(ACTION_NAME, &true.to_variant());
        settings.set_live(false);
        crate::platform::testing::inject_reduced_motion(Some(true));
        assert_eq!(
            seen.borrow().len(),
            1,
            "dropping PolicyWatch must disconnect ALL THREE handlers — no callback after drop"
        );
    }
}
