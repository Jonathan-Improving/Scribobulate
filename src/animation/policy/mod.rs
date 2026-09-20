//! Play Animations policy (TDD 27.5-27.7).
//!
//! Nothing here animates anything — that is later work. This module owns the single
//! decision anything that plays a frame must ask first: **should this animation
//! actually be running right now?**
//!
//! Three independent inputs feed that decision:
//! - The reader's own choice, carried by the process-wide `app.play-animations`
//!   stateful boolean `GAction` (POLICY § Architecture rules: one `GAction` is the
//!   single source of truth). The action itself is registered in
//!   `app::appactions::add_play_animations_action` — this module only reads it.
//! - GTK's own "reduce animations" desktop setting, `gtk-enable-animations` on
//!   [`gtk::Settings`], read LIVE (never cached) so a setting flipped mid-session, or
//!   between two launches, takes effect immediately (TDD 27.7).
//! - The OS-level "reduce motion" preference on platforms where GTK does not surface
//!   one of its own — [`crate::platform::system_reduced_motion`], which answers `None`
//!   on any platform with no source (Linux, where `gtk-enable-animations` above already
//!   carries it, and macOS until its own module lands). This is deliberately a THIRD,
//!   independent input rather than folded into the second: `gtk-enable-animations` is
//!   GTK's own property, sourced from GTK on Linux and X11/Wayland portals; the
//!   platform façade is this project's own reading of an OS setting GTK never touches
//!   on Windows/macOS at all, and conflating the two would make one input's default
//!   (`true` with no `GtkSettings`) silently stand in for the other's absence (`None`,
//!   meaning "no opinion") — which are not the same thing (TDD 27.7).
//!
//! [`effective_play`] is the pure reconciliation of the three ("reduce animations" and
//! "reduce motion" both always win), [`current`] reads all three live inputs for one
//! caller, and [`watch`] is the change-notification subscription the animated paintables use
//! to know when to re-ask.
//!
//! **A directory, not a file**, for the same reason as `platform/win32/` and
//! `platform/mac/`: this crossed POLICY's file-size soft limit. The split is by CAUSE —
//! this file is the decision core (pure functions plus the live readers built directly
//! on them), [`watch`] is the live-subscription MECHANISM built on top of that core
//! (connecting three independent change signals to one re-computed answer) — and every
//! name callers used before the split (`policy::watch`, `policy::PolicyWatch`,
//! `policy::EnableAnimationsGuard`) is re-exported below so no caller can tell it
//! happened.

use gtk::prelude::*;

mod watch;

#[cfg(all(test, feature = "gtk-integration-tests"))]
pub(crate) use watch::EnableAnimationsGuard;
pub(crate) use watch::{watch, PolicyWatch};

/// The process-wide `app.*` action name carrying the reader's Play Animations choice
/// (TDD 27.5). Shared between `app::appactions` (which registers it) and this module
/// (which only reads it) so the name cannot drift between the two.
pub(crate) const ACTION_NAME: &str = "play-animations";

/// Play Animations is on for a first launch (TDD 27.6) — the session's own
/// `#[serde(default)]` for the persisted field mirrors this constant so "no saved
/// choice yet" and "the reader chose on" restore identically.
pub(crate) const DEFAULT_CHOICE: bool = true;

/// Whether an animation should actually play, given the reader's own Play Animations
/// choice, whether GTK's own "reduce animations" setting currently permits animation,
/// and whether the OS itself is asking for reduced motion. Pure — no display, no
/// globals, no I/O — so it is unit-tested directly (TDD 27.7): EITHER "reduce" input
/// always wins, regardless of the reader's choice or of the other "reduce" input.
///
/// `system_reduced_motion` is a plain `bool`, not the `Option<bool>`
/// [`crate::platform::system_reduced_motion`] returns — callers resolve "no source on
/// this platform" to `false` (no reduction asked for) before reaching this function,
/// so the pure core never has to special-case an absent platform opinion; see
/// [`reduced_motion_of`].
pub(crate) fn effective_play(
    choice: bool,
    system_animations_enabled: bool,
    system_reduced_motion: bool,
) -> bool {
    choice && system_animations_enabled && !system_reduced_motion
}

/// The `app.play-animations` action itself, if it has been registered. `None` is not
/// normally reachable once `add_play_animations_action` has run at startup, but every
/// reader here treats it as live state rather than an invariant — a picture asking
/// "should I play?" must degrade to the default rather than panic if it somehow runs
/// before the action exists.
fn action(app: &gtk::Application) -> Option<gtk::gio::Action> {
    app.lookup_action(ACTION_NAME)
}

/// The reader's raw Play Animations choice, straight off `app.play-animations`'s own
/// state — **never** the effective (reduce-animations-adjusted) value. This is what
/// `session.rs` persists: the override is never saved as though the reader had chosen
/// it (TDD 27.7), so a system setting changed between runs takes effect on the next
/// launch rather than being baked into the saved choice.
pub(crate) fn reader_choice(app: &gtk::Application) -> bool {
    choice_of(action(app).as_ref())
}

/// The boolean state of the Play Animations action, or [`DEFAULT_CHOICE`] when it
/// is absent or carries no boolean. The one reading of the choice; [`reader_choice`]
/// and [`watch`] both go through it.
fn choice_of(action: Option<&gtk::gio::Action>) -> bool {
    action
        .and_then(|a| a.state())
        .and_then(|v| v.get::<bool>())
        .unwrap_or(DEFAULT_CHOICE)
}

/// `gtk-enable-animations`, or GTK's own default (`true`) with no settings object.
/// The one reading of the system setting; [`system_animations_enabled`] and
/// [`watch`] both go through it.
fn enabled_of(settings: Option<&gtk::Settings>) -> bool {
    settings.is_none_or(|s| s.is_gtk_enable_animations())
}

/// Resolve the platform façade's `Option<bool>` — "does the OS want reduced motion,
/// or does this platform have no opinion at all" — down to the plain `bool`
/// [`effective_play`]'s pure core takes. `None` (no source on this platform) means
/// "not reduced", the same direction GTK's own `gtk-enable-animations` defaults to
/// with no `GtkSettings` object: an absent opinion must never itself freeze
/// animation. The one reading of the platform source; [`system_reduced_motion`] and
/// [`watch`] both go through it.
fn reduced_motion_of(system_reduced_motion: Option<bool>) -> bool {
    system_reduced_motion.unwrap_or(false)
}

/// Whether GTK's system "reduce animations" setting currently permits animation.
/// Read LIVE off [`gtk::Settings::default`] on every call — never cached — so a
/// setting flipped while the app is running takes effect immediately (TDD 27.7). A
/// display with no settings object (not normally reachable once GTK is initialised)
/// answers `true`, matching GTK's own default for the property.
fn system_animations_enabled() -> bool {
    enabled_of(gtk::Settings::default().as_ref())
}

/// Whether the OS itself is currently asking for reduced motion, resolved to a plain
/// `bool` via [`reduced_motion_of`]. Read LIVE off
/// [`crate::platform::system_reduced_motion`] on every call — never cached — for the
/// same reason [`system_animations_enabled`] reads `GtkSettings` live: a setting
/// flipped while the app is running (Windows) or between launches must take effect
/// immediately, never only at the next `cargo build`.
fn system_reduced_motion() -> bool {
    reduced_motion_of(crate::platform::system_reduced_motion())
}

/// The current effective play state for `app`: the reader's choice AND GTK's own
/// system setting AND the OS-level reduced-motion preference. This is what a picture
/// asks at the moment it decides whether to animate.
pub(crate) fn current(app: &gtk::Application) -> bool {
    effective_play(
        reader_choice(app),
        system_animations_enabled(),
        system_reduced_motion(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The full truth table `effective_play` must honour (TDD 27.7) over all three
    /// inputs: EITHER "reduce animations" (GTK) or "reduce motion" (the platform
    /// façade) always wins, in every combination, regardless of the reader's own
    /// choice and regardless of the other "reduce" input. Only the one all-clear row
    /// plays; every row with at least one "reduce" input set is frozen.
    ///
    /// This is also the mutation-test anchor named in the task brief: dropping the
    /// new `system_reduced_motion` term from the conjunction in `effective_play`
    /// turns every `!effective_play(_, _, true)` assertion below red, because the
    /// function would then answer `choice && system_animations_enabled` and ignore
    /// its third argument entirely.
    #[test]
    fn effective_play_truth_table() {
        // choice, gtk-enable-animations, system-reduced-motion ⇒ expected
        let cases = [
            (true, true, false, true),
            (true, true, true, false),
            (true, false, false, false),
            (true, false, true, false),
            (false, true, false, false),
            (false, true, true, false),
            (false, false, false, false),
            (false, false, true, false),
        ];
        for (choice, system_animations_enabled, system_reduced_motion, expected) in cases {
            assert_eq!(
                effective_play(choice, system_animations_enabled, system_reduced_motion),
                expected,
                "choice={choice} gtk-enable-animations={system_animations_enabled} \
                 reduce-motion={system_reduced_motion} must play={expected}",
            );
        }
    }

    /// [`reduced_motion_of`]'s own polarity: an absent platform opinion (`None` — no
    /// source on this platform) must resolve to "not reduced", never to a guessed
    /// reduction, mirroring how [`enabled_of`] treats a missing `GtkSettings`.
    #[test]
    fn reduced_motion_of_treats_no_source_as_not_reduced() {
        assert!(!reduced_motion_of(None));
        assert!(!reduced_motion_of(Some(false)));
        assert!(reduced_motion_of(Some(true)));
    }

    /// **What this host's own reduce-motion source says, and that the app agrees.**
    ///
    /// On a host with no source, behaviour is UNCHANGED by this input's existence:
    /// the façade answers `None` exactly as it did before the input was added, so
    /// folding it through `reduced_motion_of` lands on `false`, which is the value
    /// [`current`]'s conjunction always had for this term. No display needed:
    /// neither function touches GTK.
    ///
    /// **One test, branching at RUNTIME**, because the two platforms have different
    /// host facts and neither may be compiled away. This was a `#[cfg(not(windows))]`
    /// pair — a Windows-only arm and an everywhere-else arm — which is what POLICY
    /// § Testing forbids verbatim: a `cfg`'d-off test does not skip loudly, it
    /// vanishes, and the seat that most needs to know a check did not run is the one
    /// that cannot see it is missing. Nothing is skipped here either, because both
    /// arms carry a real assertion; a `SKIPPED` line is for a check that genuinely
    /// cannot drive, not for one whose expected answer differs.
    ///
    /// Mutation test, no-source hosts: claim the façade sees reduced motion on Linux
    /// (make `platform`'s Linux fallback answer `Some(true)` instead of `None`) and
    /// this goes red. Windows: reduce the Win32 arm to the `None` stub the other
    /// platforms use, or have `SystemParametersInfoW` fail and be swallowed, and it
    /// goes red there.
    ///
    /// Which WAY Windows leans is deliberately not asserted — that is the reader's
    /// OS setting, and pinning it would fail whenever "Show animations in Windows"
    /// is legitimately switched off. Do NOT "repair" the no-source arm into
    /// `system_reduced_motion() == reduced_motion_of(platform::system_reduced_motion())`
    /// either: that is the function's definition spelled out, so it holds under every
    /// mutation of either side and can never fail.
    #[test]
    fn the_platform_facade_answers_what_this_host_is_able_to_know() {
        let facade = crate::platform::system_reduced_motion();
        if cfg!(windows) {
            // Windows reads `SPI_GETCLIENTAREAANIMATION`, so it must hold an opinion
            // rather than abstain — the whole risk of an FFI no Linux seat can run.
            assert!(
                facade.is_some(),
                "Windows reads SPI_GETCLIENTAREAANIMATION and must answer Some(_); \
                 None means the FFI failed or the arm regressed to the no-source stub"
            );
        } else {
            // Linux and macOS have no source of their own: `GtkSettings` already
            // carries the desktop's answer on Linux, and macOS's façade is deferred.
            assert_eq!(
                facade, None,
                "this host has no platform reduced-motion source"
            );
            assert!(
                !system_reduced_motion(),
                "no source must resolve to \"not reduced\", not a guess"
            );
        }
    }
}
