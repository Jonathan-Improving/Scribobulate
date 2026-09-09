#!/usr/bin/env bash
#
# Resolves WHERE the macOS developer install puts things, and WHO it acts as.
#
# Sourced by packaging/macos/install.sh and packaging/macos/uninstall.sh. It exists
# because those two scripts must agree on every one of these answers and there is no way
# to check that they do: an uninstaller that resolves the anchor one line differently from
# the installer reports a clean removal having looked in the wrong place. One resolver,
# sourced twice, cannot disagree with itself. Same rule POLICY.md applies to the coverage
# floor and the input limits — the second copy is how the first stops matching.
#
# THE MODE IS THE INVOKING UID, NOT A FLAG, and it picks a whole set of destinations:
#
#                     app              CLI               manual pages
#   ./install.sh      ~/Applications   ~/.local/bin      ~/.local/share/man
#   sudo ./install.sh /Applications    /usr/local/bin    /usr/local/share/man
#
# NOTHING IS WRITTEN INTO $(brew --prefix) ANY MORE, and that is a deliberate reversal.
# These links used to go into Homebrew's bin/ and share/man/, justified as "the one
# directory already writable and already on PATH". That justification dies with global
# mode, and the cost was always real: files Homebrew did not install inside the prefix it
# manages are unbrewed files `brew doctor` reports, and they couple this project's install
# to a package manager the operator may relocate or remove. Homebrew is still a BUILD
# dependency — bundle.sh copies the GTK closure out of /opt/homebrew — but a build
# dependency is not an install destination.
#
# /usr/local/* IS THE STOCK PATH, MEASURED, not assumed: `/etc/paths` ships /usr/local/bin
# as its FIRST line, ahead of /usr/bin, and `/usr/libexec/path_helper -s` puts it at the
# front of PATH before any shell profile runs; `/etc/manpaths` ships /usr/local/share/man.
# That holds on Apple silicon — it is not an Intel-era leftover. An earlier revision of
# this file asserted the opposite and was wrong. Note `manpath` omits entries that do not
# EXIST, so /usr/local/share/man is absent from its output until something creates it.
#
# ~/.local/* IS A JUDGEMENT CALL and is not blessed by anything Apple ships. macOS defines
# no per-user bin directory at all, so there is no convention to appeal to; ~/.local/bin
# is chosen because packaging/linux/install.sh already uses it and one fewer difference
# between the two platforms beats inventing a third answer. Neither per-user directory is
# on the default search path, so the per-user install REPORTS that with the line to add.
# Printing a note the operator can act on is the honest option; quietly writing into
# another project's prefix to avoid printing it is not.
#
# The global mode exists because ~/Applications is NOT the "Applications" in Finder's
# sidebar — that is /Applications, and only /Applications. A per-user install therefore
# completes successfully and looks to the operator like it did nothing, which is the
# report that prompted this. Both locations are scanned by Launch Services and both give
# a real Dock tile, so the difference is visibility, not function.
#
# ONE BUNDLE STILL, AND THE GATE IS WHAT ENFORCES IT. The two modes are not two installs
# that may coexist: CFBundleIdentifier is the app's identity, so a copy in both locations
# is two registrations for one identity, and `open -b` and the PATH command can then run
# different builds with nothing warning you. So each mode names the OTHER location as
# FOREIGN and refuses to build while a bundle sits there. This replaces the older claim
# that the developer install and the .dmg route were "distinct by construction" — that was
# true only while the developer install could never target /Applications. The invariant it
# was protecting survives; the mechanism is now the gate rather than the geography.
#
# ---------------------------------------------------------------------------------
# WHY ROOT IS NOT SIMPLY LET LOOSE ON THE WHOLE RUN.
#
# `sudo ./install.sh` runs every line as root, and three of them must not be:
#
#   1. THE BUILD. `cargo build --release` as root leaves target/ owned by root, so the
#      operator's next ordinary `cargo build` fails on its own build directory, and it
#      writes root-owned files into the user's ~/.cargo. Neither is recoverable without
#      more sudo. The build therefore drops back to the invoking user (see as_user).
#   2. $HOME. It survives sudo only by macOS sudoers policy (env_keep), not by anything
#      this script can rely on: under `su -`, or any sudoers that resets it, $HOME is
#      /var/root and `$HOME/Applications` and `$HOME/.Trash` silently name root's tree
#      instead of the operator's. The gate would then scan the wrong home and pass. So
#      the home directory is read from the directory service, never from the environment.
#   3. $PATH. sudo's env_reset gives root a PATH that on a stock Mac does NOT contain
#      /opt/homebrew/bin — MEASURED here: the operator's PATH carries it, root's does not.
#      Three separate things break on that and only one of them is loud:
#        - `command -v brew` fails, so the script refuses with "brew not found" on a
#          machine where brew is plainly installed;
#        - the PATH gate scans ROOT's PATH, which is not the PATH that will resolve
#          `scribobulate` when the operator types it — the gate would inspect an
#          environment nobody uses and report the machine clean;
#        - cargo may not be found at all.
#      So the user's login PATH is recovered and adopted for the run.
#
# The recovery is `sudo -u "$RUN_USER" -i -- printenv PATH`: a LOGIN shell, so the
# operator's profile is sourced and the PATH is the real one they type at, rather than a
# guess assembled from a list of likely directories. Adopting it for root is safe because
# the only commands root then runs are ditto, ln, codesign, mkdir and rm.
#
# `brew` IS STILL LOOKED UP UNDER THAT PATH, though nothing is installed into its prefix
# any more: install.sh checks for it as a BUILD prerequisite, since bundle.sh copies the
# GTK closure out of /opt/homebrew. Only `command -v brew` and `brew --prefix` tolerate
# root — check-run-command-as-root exempts `--prefix` by name and odie's on everything
# else — so do not add a brew call that does real work to either script.

# --- Mode, acting user, and that user's real home --------------------------------
if [ "$(id -u)" -eq 0 ]; then
    MODE=global
    RUN_USER="${SUDO_USER:-}"

    # A ROOT LOGIN IS REFUSED RATHER THAN GUESSED AT. Without SUDO_USER there is no
    # invoking user to drop to for the build, no home directory to scan for the foreign
    # bundle, and no login PATH to adopt. Every one of those would have to be invented,
    # and an install built on three invented answers is worse than no install.
    if [ -z "$RUN_USER" ] || [ "$RUN_USER" = "root" ]; then
        echo "error: run this as 'sudo ./install.sh' from your own account." >&2
        echo "  This is root with no SUDO_USER — a root login or 'su -' rather than" >&2
        echo "  sudo. The global install still has to build as a normal user (root" >&2
        echo "  would leave target/ and ~/.cargo root-owned) and it has no way to" >&2
        echo "  know which user that is." >&2
        exit 1
    fi

    # Directory service, not $HOME — see point 2 above.
    USER_HOME="$(dscl . -read "/Users/$RUN_USER" NFSHomeDirectory 2>/dev/null \
        | sed -n 's/^NFSHomeDirectory: //p')"
    if [ -z "$USER_HOME" ] || [ ! -d "$USER_HOME" ]; then
        echo "error: could not resolve the home directory of '$RUN_USER'." >&2
        echo "  dscl . -read /Users/$RUN_USER NFSHomeDirectory returned nothing usable." >&2
        exit 1
    fi

    # Login PATH, not root's — see point 3 above.
    USER_PATH="$(sudo -u "$RUN_USER" -i -- /usr/bin/printenv PATH 2>/dev/null || true)"
    if [ -z "$USER_PATH" ]; then
        echo "error: could not read the login PATH of '$RUN_USER'." >&2
        echo "  Without it this script would search root's PATH, which on a stock Mac" >&2
        echo "  has no /opt/homebrew/bin: it would fail to find brew, and the PATH gate" >&2
        echo "  would inspect an environment you never type in." >&2
        exit 1
    fi
    PATH="$USER_PATH"
    export PATH
else
    MODE=user
    RUN_USER="$(id -un)"
    USER_HOME="$HOME"
fi

# --- Anchor and foreign location, chosen by mode ----------------------------------
GLOBAL_APPS="/Applications"
USER_APPS="$USER_HOME/Applications"

if [ "$MODE" = "global" ]; then
    ANCHOR_DIR="$GLOBAL_APPS"
    FOREIGN_DIR="$USER_APPS"
    MODE_LABEL="global"
    OTHER_LABEL="per-user"
    BIN_DIR="/usr/local/bin"
    MAN_DIR="/usr/local/share/man"
    # The command that clears the FOREIGN bundle: the OTHER mode's uninstall. Named as
    # the first remedy because a refusal whose only exit is a hand-typed `rm -rf` reads
    # as a wall, and because that script also clears the Launch Services registration,
    # which an `rm -rf` leaves behind.
    FOREIGN_UNINSTALL="packaging/macos/uninstall.sh"
else
    ANCHOR_DIR="$USER_APPS"
    FOREIGN_DIR="$GLOBAL_APPS"
    MODE_LABEL="per-user"
    OTHER_LABEL="global"
    BIN_DIR="$USER_HOME/.local/bin"
    MAN_DIR="$USER_HOME/.local/share/man"
    FOREIGN_UNINSTALL="sudo packaging/macos/uninstall.sh"
fi

ANCHOR="$ANCHOR_DIR/Scribobulate.app"
FOREIGN="$FOREIGN_DIR/Scribobulate.app"
LINK="$BIN_DIR/scribobulate"

# --- Where a link points, in three states ------------------------------------------
#
# TWO STATES IS NOT ENOUGH, and the missing one produces a dead end rather than a wrong
# answer. "Into the anchor" (ours, remove it) and "not into the anchor" (report it, run
# the other mode's uninstall) leaves a third case misfiled: a link into NEITHER
# Applications directory — one created before the install anchored outside target/, still
# pointing into the build tree. Reporting that as the other mode's would send the operator
# to a command that does not remove it either, which is a remedy that no-ops while reading
# as a remedy.
classify_link_target() {
    case "$1" in
        "$ANCHOR"/*)  echo ours ;;
        "$FOREIGN"/*) echo foreign ;;
        *)            echo orphan ;;
    esac
}

# --- Acting as the invoking user --------------------------------------------------
#
# In per-user mode this is a plain call; in global mode it drops root. Used for the build
# (point 1 above) and for mdfind, which is a query against a PER-USER Spotlight index and
# answers for whoever asks — run as root it is not the operator's index and its answer is
# not about the operator's machine.
# `env` WITH AN EXPLICIT ENVIRONMENT, NOT `sudo -i`. The login-shell form is the obvious
# way to get the user's PATH back, and it is used exactly once above to HARVEST that PATH
# — but it is the wrong tool for running a command with arguments, because `sudo -i`
# hands the command line to a shell, which re-quotes and re-splits it. An OUT_DIR
# containing a space would arrive at bundle.sh as two arguments. Harvest once through the
# login shell, then run everything with the environment stated outright and no shell in
# the middle.
as_user() {
    if [ "$MODE" = "global" ]; then
        sudo -u "$RUN_USER" /usr/bin/env \
            HOME="$USER_HOME" \
            PATH="$USER_PATH" \
            USER="$RUN_USER" \
            LOGNAME="$RUN_USER" \
            "$@"
    else
        "$@"
    fi
}

announce_mode() {
    echo ":: mode: $MODE_LABEL -> $ANCHOR_DIR"
    if [ "$MODE" = "global" ]; then
        echo ":: building as $RUN_USER (root builds would leave target/ root-owned)"
    fi
}
