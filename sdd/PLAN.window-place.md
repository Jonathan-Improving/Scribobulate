# Plan: Restore each window's place on the desktop

## Problem

Session restore (TDD §15.10) brings back each window's zoom, chrome and tabs, but not
**where** the window was. On a KDE desktop with several virtual desktops, activities and
monitors, every restored window lands on the current desktop and activity, wherever KWin's
placement policy puts it, and the reader has to move each one back by hand.

Size isn't restored correctly either, which already breaks TDD §15.10 ("every window
reopens with the same size"). Maximized and full screen aren't saved at all, and no rubric
covers them. This plan fixes size along with the rest, because everything is applied at
the same moment, while the window is first shown.

**Window place** (the term this plan introduces) is everything about where and how a
window sits on the desktop: its virtual desktop, its KDE activities, its monitor and
position on that monitor, its size, and its window states. The states are maximized, full
screen, shaded, kept above or below other windows, and shown without titlebar and frame.
The goal is to save each window's place with the session and restore it at startup.

### Root cause

**Size.** The session saves `Window::width()/height()`, which is the window's current
allocation. It restores them as the default size. GTK documents that the allocation does
not round-trip and that `gtk_window_get_default_size` is the value to save (`gtkwindow.c`,
`gtk_window_set_default_size` doc). It also leaves the default size untouched while the
window is maximized, full screen or tiled. Saving a maximized window therefore records
the maximized pixel size, and restore opens a normal window that large. KWin then treats a
window that fills the work area as maximized and keeps that huge rectangle as its
un-maximize size. This is not a client-side-decoration mismatch: Scribobulate's X11 windows
use KWin's frame (`GTK_CSD` is set only on Windows, and the app has no custom titlebar).

**Everything else.** GTK4 has no API for desktops, activities, positions or the X11-only
states, and has no XSMP session management, which is how KWin's own session restore would
otherwise put the windows back. On X11 each part is a window property that KWin reads when
it first manages the window (`X11Window::manage`, KWin 5.27.11):

| Part of the place | X11 carrier | Special values | What GTK 4.6 does to it at first show |
|---|---|---|---|
| Virtual desktop | `_NET_WM_DESKTOP`, CARDINAL | `0xFFFFFFFF` = all desktops | **Deletes it** in `set_initial_hints`, just before `XMapWindow` |
| Activities | `_KDE_NET_WM_ACTIVITIES`, type **STRING**, comma-separated UUIDs | empty or nil UUID = all activities | Leaves it alone |
| Monitor + position | the window's X position plus `USPosition` in `WM_NORMAL_HINTS` | — | GDK never sets `USPosition`. A flag set before `present` survives until KWin places the window; GDK's idle rewrite of the hints comes afterwards and doesn't re-place it |
| Maximized, full screen | `_NET_WM_STATE` atoms | — | GTK writes them itself from `gtk_window_maximize` / `gtk_window_fullscreen_on_monitor` called before `present` |
| Shaded, above, below | `_NET_WM_STATE_SHADED` / `_ABOVE` / `_BELOW` | — | **Overwritten**: `set_initial_hints` replaces `_NET_WM_STATE` with only the states GTK knows about. GTK 4.6 has no call to set these three |
| No titlebar and frame | `_MOTIF_WM_HINTS`, decorations flag set and decorations field `0` | — | Leaves it alone |

The activities property must be written with type `STRING`. KWin rejects any other type, so
`gdk_x11_surface_set_utf8_property` (which writes `UTF8_STRING`) cannot be used for it.

**What X11 can't express.** A window on several virtual desktops, but not all, has no X11
form: `_NET_WM_DESKTOP` is a single number, and KWin 5.27 keeps only the last desktop when
given several. A place therefore records one desktop or all of them. Several activities
are fine.

**Scope.** Wayland is out of scope: it can't be tested here, and this desktop won't be
upgraded to a Wayland-capable Plasma or a newer GTK for at least a year. macOS Spaces and
Windows virtual desktops have no supported per-window placement API, and those platforms
have no activities.

## Previously attempted

Nothing yet.

## Possible approaches

### ✅ 1. Write the window properties at first show, before KWin manages the window

On save, read each part of the place from the live window. On restore, write each part at
the point where it survives until KWin's `manage()` reads it, so the window is placed right
from its first frame with no visible jump:

- **Before `present`:** default size (from the saved default size, not the allocation);
  `gtk_window_maximize` or `gtk_window_fullscreen_on_monitor`; the position via
  `XMoveWindow` plus `USPosition`; the activities property; and the Motif no-border hint.
- **Under an X server grab held across `present`:** the virtual desktop and the
  shaded/above/below states, since GTK deletes or overwrites both just before it maps the
  window. The grab stops KWin from handling the map until those writes are in, so the order
  is deterministic. Writing straight after `present` without a grab is a race and must not
  be used (Technical details, *The server grab*).

All of this lives in a new workspace crate, `winplace`, a sibling of `richimg`. It knows
nothing about Scribobulate. Its public surface is a serde-ready `Place` value and two
calls: *capture the place of this `gtk::Window`*, and *show this `gtk::Window` in this
place*. The second call takes over `present()` for a restored window, because the order
of the writes around `present` is the whole mechanism. Size, maximized and full screen are
plain GTK and work on every platform. The rest is X11 only and does nothing elsewhere.
Like `platform/x11.rs`, the crate hand-rolls the few GDK-X11 getters and Xlib calls it
needs rather than depending on `gdk4-x11`. `platform/x11.rs` stays where it is: it answers
a popover question, not a placement one.

On macOS, full screen moves the window into its own Space, which
`platform/mac/fullscreen.rs` already manages, so restoring full screen there has to be
ratified by the `mac` seat. Restoring size, maximized and full screen on Windows has to be
ratified by the `windows` seat.

**Pros**: no third-party dependency. KWin still does the placing, so window rules, panels
and its own checks still apply. Desktop, position and states are EWMH/ICCCM, so they work
under any compliant X11 window manager. Only activities and the no-border reading are
KDE-specific.
**Cons**: X11 only. Restoring the desktop and the three X11-only states briefly grabs the X
server, a sharp tool with strict rules (Technical details, *The server grab*).

### ❌ 2. Implement XSMP so KWin's own session restore places the windows

Rejected as redundant with approach 1, which already restores the whole place, including
when the reader starts the app themselves, which XSMP doesn't cover. XSMP would also need a
libSM client written from scratch.

### ❌ 3. Move the windows after they appear (KWin scripting over D-Bus)

Rejected. The windows visibly jump, it only works on KDE, and it depends on a scripting
interface that changes between Plasma releases.

### ❌ 4. KWin window rules

Rejected. Rules are user configuration that applies to every window of the app. They
cannot express "this window goes back where it was".

## Recommendation

Approach 1. Each window in the saved session gets a place, captured when the session is
saved and applied when it is restored. On every platform the place holds size, maximized
and full screen. On X11 it also holds the X11-only parts. Elsewhere those parts are
written empty and ignored when read. The window's existing `width`/`height` move into the
place as its default size. That is a schema version bump, with a migration in
`session/migrate.rs` that carries an older session's width and height over. SCHEMA.md's
session section and TDD §15.10 are updated in the same change.

**Capturing.** The session is built in `window/lifecycle.rs` from live windows, while each
surface is still realized. Size is `gtk_window_get_default_size`, which stays the
un-maximized size while the window is maximized or full screen. Desktop, activities and
`_NET_WM_STATE` are read from the window's properties. Monitor and position are read as the
monitor's connector name plus the position relative to that monitor's origin, in GDK
logical pixels. Never store root coordinates: they change whenever the monitor layout
changes (e.g. the operator's 6400×1440 virtual screen). No titlebar and frame is read as a
KWin frame of zero width (`_NET_FRAME_EXTENTS` all zero), which is the frame KWin actually
built. KWin's own record of the choice is a window rule, not something a window can read.

**Degrading.** Restore applies each part on its own, and a part that no longer exists
falls back without affecting the rest:

| Missing | Fallback | Who decides |
|---|---|---|
| Virtual desktop | Current desktop | KWin (an out-of-range desktop is ignored, never clamped or created) |
| Activity | **The app drops it.** If none remain, the property is not written, so the window lands on the current activity | `winplace`. KWin would put it on *all* activities instead, which is wider than a new window gets |
| Monitor | Where KWin places a new window, at the saved size | `winplace` (no position written) |
| Monitor now smaller | Position clamped so the title bar stays on the monitor | `winplace` |

Only session restore uses the place. New windows, including Window ▸ New Window, still open
where KWin puts them.

**Verification.** The size, maximized and full-screen rubrics are GTK integration tests and
run on every platform's pipeline. The size test must fail on today's code first. The
X11-only parts need a real window manager, which the pipeline's Xvfb doesn't provide, so
they get an integration-test rig that runs a nested KWin (see Technical details). That rig
has one monitor, so the two-monitor checks (full screen on the saved monitor, a monitor
that's gone) are manual only, in a `tests/MANUAL-TEST.md` entry on the operator's desktop.

## Proposed TDD Rubrics

### Restored windows return to their place
- **Given** an X11 desktop where, at quit, the application's windows are spread across
  different virtual desktops, activities and monitors
- **When** the application is relaunched and restores the session
- **Then** each window opens on its own virtual desktop, in its own activities, on its own
  monitor at the same position on that monitor, and does not appear somewhere else first
  and then jump
- **And** a window that was on all desktops, or on several or all activities, is restored to
  exactly that
- **And** each window state it had (kept above or below, shaded, without titlebar and frame)
  is restored with it

### Restored windows keep their size, maximized and full screen
- **Given** windows of different sizes, some maximized and some full screen, open at quit, on
  any platform
- **When** the application is relaunched and restores the session
- **Then** each window reopens at the size it had, a maximized window reopens maximized, a full
  screen window reopens full screen on the same monitor, and leaving maximized or full
  screen brings back the size it had before
- **And** a window's size does not drift across repeated quit-and-relaunch cycles

### The place recorded is where the window was at quit
- **Given** a window whose place changed after it was opened: moved to another virtual
  desktop, activity or monitor, resized, or a window state turned on or off
- **When** the application quits and is relaunched
- **Then** the window comes back where it was at quit, not where it was first opened

### A place that no longer exists degrades one part at a time
- **Given** a saved session where a window's virtual desktop, activity or monitor no longer
  exists (the desktop was removed, the activity deleted, the monitor disconnected)
- **When** the session is restored
- **Then** that part of the place falls back to where a new window would go, every part
  that still exists is applied as usual, and the restore otherwise proceeds normally
- **And** a window whose monitor is now smaller than it was opens with its title bar on
  that monitor, never off-screen
- **And** a full screen window whose monitor is gone opens full screen on the monitor a new
  window would use

### Only session restore uses the place
- **Given** the application is running
- **When** a new window is opened by any path other than session restore
- **Then** it opens where the window manager places new windows, exactly as before

### Off X11, only size, maximized and full screen are restored
- **Given** a desktop other than X11 (macOS, Windows, Wayland), or a session saved on one and
  restored on another
- **When** the session is saved and restored
- **Then** size, maximized and full screen are restored as on X11; the window otherwise opens
  where the desktop places new windows, the X11-only parts of a saved place are ignored
  without error, and every other part of the restore (TDD §15.10) is unaffected

## Technical details preserved

Sources: GTK `gtk-4-6` at 4.6.9, KWin tag `v5.27.11`, plus an Xvfb property probe (no
window manager).

**Order inside `present` (GTK 4.6).** `gdk_x11_toplevel_present` → `gdk_x11_surface_show`
→ on first map `set_initial_hints` (`gdksurface-x11.c:1456`) → `XMapWindow` (`:1604`),
with no hook between them. `set_initial_hints` deletes `_NET_WM_DESKTOP` unless the surface
is sticky (`:1566-1570`, not settable at 4.6) and rewrites `_NET_WM_STATE` from GDK's own
state bits, deleting it when none are set (`:1541-1554`). `gdk_x11_surface_move_to_desktop`
and `gdk_wmspec_change_state` only send EWMH client messages, which act on a window KWin
already manages.

**Position.** KWin honours a position only when `USPosition` or `PPosition` is set
(`GeometryHints::hasPosition`, used in `manage()`). GDK sets `PSize` only
(`setup_toplevel_window`, `gdksurface-x11.c:1008`), and its hint rewrite runs on an idle
after `present` returns (`gdk_x11_surface_request_layout`), after KWin has already placed
the window. `x11_surface_move` multiplies by the surface scale. `gtk_window_fullscreen_on_monitor`
before map moves the window to the monitor's origin but sets no `USPosition`, so KWin
ignores that move unless the app also sets `USPosition` before `present`.

**Activities.** KWin reads the property as `STRING` (`utils/xcbutils.h:904`) and rejects a
type mismatch. It drops UUIDs that aren't running activities and treats an empty result as
all activities (`window.cpp:3545-3562`). The nil UUID means all activities.

**No titlebar and frame.** `_MOTIF_WM_HINTS` with the decorations flag set and decorations
`0` (`MotifHints::noBorder`). KWin's menu action stores the choice as a window rule
(`X11Window::setNoBorder`), which matches by class and role, so a window can't read it
back. The option only applies to server-decorated windows (`userCanSetNoBorder`).

**Nested KWin test rig.** Use a private `XDG_RUNTIME_DIR`, `XDG_CONFIG_HOME`,
`XDG_CACHE_HOME` and `XDG_DATA_HOME`, then `dbus-run-session`, then Xvfb on a free display.
Inside that bus, start `/usr/lib/x86_64-linux-gnu/libexec/kactivitymanagerd`, which is not
on `PATH`, then `kwin_x11` with `DISPLAY` set. Never pass `--replace`, which would take over
the operator's window manager if it reached the live bus. Set `GTK_A11Y=none`,
`NO_AT_BRIDGE=1` and `QT_ACCESSIBILITY=0`. The private runtime directory and these
variables are what keep the rig from deleting the live `/run/user/1000/at-spi/bus_0`
(`CLAUDE.md` § Accessibility Bus). The private config directory keeps the nested KWin away
from the operator's KWin config.

**The server grab.** KWin reads `_NET_WM_DESKTOP` and `_NET_WM_STATE` once, with
`GetProperty` from current server state, while handling the first `MapRequest`
(`X11Window::manage` → `NETWinInfo::update`, `netwm.cpp:3921-3925`). A later
`PropertyNotify` never re-applies the desktop or shade/above/below. So the writes must reach
the server after `set_initial_hints` and before KWin's read. The sequence is:

1. `XGrabServer` on GDK's **own** `Display*` (`gdk_x11_display_get_xdisplay`).
2. `gtk_window_present`.
3. Write `_NET_WM_DESKTOP` (CARDINAL). Read-modify-write `_NET_WM_STATE`, keeping
   whatever GTK wrote (e.g. maximized, full screen) and adding shaded/above/below.
4. `XUngrabServer`, then `XFlush` (neither flushes on its own).

Proved safe on GTK 4.6.9: `present` makes round-trips only to the server, which keeps
serving the grabbing connection, and never waits on another client. Its frame-clock idle
doesn't run inside `present`, and it never ungrabs on its own. Probe on Xvfb: no hang
(39 ms), and the desktop and states were present after the ungrab. Rules while the grab is
held:

- Never use a second `XOpenDisplay` connection. The grab freezes it and any round-trip
  deadlocks.
- Never iterate the main loop.
- Never call `gdk_x11_display_ungrab` or anything that reaches it, such as the XI2 pointer
  query (`gdk_x11_device_xi2_surface_at_position`). It would release the raw grab early.
- Keep the grab to exactly steps 2–3.

Without the grab, `present` may already have flushed the map (with a startup-notification
id, or on a cold EWMH cache), and even a shared flush leaves KWin free to read first. The
probe had no window manager, so the first nested-KWin integration test must prove the
desktop and states land on a real KWin before the rest is built on it.
