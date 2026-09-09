#!/usr/bin/env bash
#
# Put a `scribobulate` command on PATH, backed by the built .app bundle.
#
# WHAT THIS FIXES: bundle.sh produces Scribobulate.app, but nothing after that puts a
# `scribobulate` command in a terminal — launching means `open Scribobulate.app` or
# spelling out the full path to the binary inside it. This script symlinks the bundle's
# own executable onto PATH. Where that is depends on the mode, and mode.sh owns the
# table; the short version is /usr/local/bin under sudo and ~/.local/bin without it.
#
# IT NO LONGER WRITES INTO HOMEBREW'S PREFIX. It used to, on the grounds that this project
# already requires Homebrew for GTK4 and its bin/ is therefore already writable and on
# PATH. That reasoning held only while there was no global mode, and it was buying
# convenience with someone else's directory: files Homebrew did not install inside the
# prefix it manages are unbrewed files `brew doctor` reports. Homebrew remains a BUILD
# dependency and nothing more.
#
# THE OWNERSHIP CONSEQUENCE, RE-MEASURED AFTER THE MOVE. The old destination
# (/opt/homebrew/bin) is mode 775 owned by the invoking user, so a non-sudo uninstall could
# remove even a link a sudo run had created — deleting a symlink needs write on the
# containing DIRECTORY, not on the link. /usr/local/bin is root:wheel 755, so that is no
# longer true: a global-mode link genuinely requires sudo to remove. That is not a
# regression to work around, it is why uninstall.sh mirrors the mode and why verify_gone
# re-tests every removal instead of trusting `rm`.
#
# The symlink resolves to `Scribobulate.app/Contents/MacOS/scribobulate` — inside the
# bundle, not a separate copy — so a terminal launch runs the exact executable Finder
# or the Dock would, with its Dock/Cmd-Tab identity intact (bundle.sh's whole point);
# a bare copy sitting outside `Contents/MacOS/` would lose that identity.
#
# WHAT THIS IS NOT: the redistributable installer — that is dmg.sh (pipeline step
# 10). This is the developer-convenience counterpart to the top-level `install.sh` on
# Linux: it needs cargo and the Homebrew GTK libraries already on this machine.
#
# ---------------------------------------------------------------------------------
# THE ANCHOR IS AN Applications DIRECTORY, NOT THE BUILD DIRECTORY, AND THAT IS THE POINT.
#
# WHICH Applications DIRECTORY IS THE MODE, and the mode is the invoking UID:
#
#     ./install.sh          -> per-user, anchor ~/Applications
#     sudo ./install.sh     -> global,   anchor /Applications
#
# packaging/macos/mode.sh resolves that and everything downstream of it, and uninstall.sh
# sources the same file. Read it for the anchor/foreign rules and for the three things
# root must NOT be allowed to do (build as root, trust $HOME, trust $PATH).
#
# The global mode exists for a reason worth stating plainly: ~/Applications is not the
# "Applications" in Finder's sidebar. That is /Applications, and only /Applications. A
# per-user install therefore succeeds, registers, works from Spotlight and the Dock — and
# reads to the operator as having silently done nothing, because the folder they went to
# look in is not the folder it installed to.
#
# This script used to point the PATH symlink and both manual-page symlinks straight
# at `$OUT_DIR/Scribobulate.app` — inside `target/`. Everything that legitimately
# empties a build directory then silently broke the install: `cargo clean`,
# `rm -rf target`, or moving the bundle to /Applications (a Finder drag WITHIN one
# volume is a move, not a copy).
#
# It broke SILENTLY, which is the part worth understanding, because the same trap is
# waiting for any future script that links into a directory it does not own. A
# dangling symlink is not executable, so the shell does not error on it — it SKIPS the
# entry and keeps walking PATH. MEASURED on this machine (macOS 26, Darwin 25.0.5):
# with `/opt/homebrew/bin/scribobulate` dangling, `command -v scribobulate` resolved
# to `~/.local/bin/scribobulate` — a five-day-old binary left behind by an old run of
# the LINUX installer, which nothing on this platform has ever known about. The
# operator built the current tree, ran this script, and got the stale binary with no
# diagnostic of any kind.
#
# So the bundle is COPIED out of the build directory to a stable anchor and everything
# resolves there. Launch Services scans both anchors: MEASURED by dropping an unlaunched,
# never-`lsregister`-ed bundle in ~/Applications and finding it in `lsregister -dump` and
# resolvable by `osascript -e 'id of app "…"'` within three seconds. A bundle launched
# from there reports `type="Foreground"` with its own bundle path to `lsappinfo`, i.e. a
# real Dock tile and Cmd-Tab entry. No explicit registration call is needed and none is
# made. The two anchors differ in VISIBILITY, not in function.
#
# WHY THE ANCHOR IS NEVER A BUNDLE THIS RUN DID NOT BUILD: because this script's first
# act is a release build, and it must never put anything on PATH other than the artefact
# it just produced. Adopting a bundle already sitting at the anchor would reproduce the
# original defect exactly — build the latest, type `scribobulate`, run something older,
# no diagnostic — with a different mechanism and identical silence. So the anchor is
# always overwritten with what this run built, and the run SAYS SO when it replaced
# something, because in global mode that something is plausibly a .dmg copy.
#
# THE TWO GATES BELOW EXIST BECAUSE THE ANCHOR ALONE DOES NOT GIVE ONE COPY.
# CFBundleIdentifier, not the path, is the app's identity, so a second bundle anywhere
# is a second registration for the same identity: with a copy in both /Applications and
# ~/Applications, `open -a Scribobulate` and `open -b com.extollit.scribobulate` both
# MEASURED as launching the /Applications one, while the PATH command ran ours. Two
# copies, silently divergent, decided by which route the user took. The gates run
# BEFORE the release build so a refusal costs seconds rather than minutes, and the PATH
# one runs again at the end, because the second half of what it checks is the thing this
# script just created.
#
# THE GATE, NOT THE GEOGRAPHY, IS WHAT KEEPS IT TO ONE. This used to rest on the claim
# that the developer install and the .dmg route were distinct BY CONSTRUCTION — one
# anchored at ~/Applications, the other landing in /Applications, so "remove what I
# created, report what I did not" named two different bundles. Global mode ends that:
# both routes can now target /Applications. The invariant survives unchanged; what
# enforces it is Gate 1 naming the OTHER mode's location as FOREIGN and refusing.
#
# THEY REPORT AND REFUSE; THEY NEVER DELETE — with one exception that is not one: the
# ANCHOR is replaced, because overwriting the destination is what installing means. A
# FOREIGN bundle is another route's, and removing it is an overreach a developer install
# is not entitled to make just because it noticed. The refusal names the other mode's
# uninstall.sh as the remedy, which is one command and also clears the Launch Services
# registration that a bare `rm -rf` would strand.
#
# Usage: packaging/macos/install.sh [OUTPUT_DIR]   (default: target/macos, same as bundle.sh)
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT_DIR="${1:-$REPO_ROOT/target/macos}"
BUILT="$OUT_DIR/Scribobulate.app"

[[ "$(uname)" == "Darwin" ]] || { echo "error: macOS only" >&2; exit 1; }

# MODE, ANCHOR, FOREIGN, RUN_USER, USER_HOME and as_user all come from here, and this is
# the only place any of them is decided — uninstall.sh sources the same file, so the two
# cannot resolve the anchor differently. It also fixes PATH in global mode, which must
# happen BEFORE the brew lookup below.
# shellcheck source=packaging/macos/mode.sh
. "$REPO_ROOT/packaging/macos/mode.sh"

announce_mode

# A BUILD PREREQUISITE, NOT A DESTINATION — the distinction this check used to blur.
# Nothing is installed into Homebrew's prefix any more (see mode.sh), but bundle.sh still
# copies the whole GTK closure OUT of it, so a machine without brew cannot produce the
# bundle at all. Checked here rather than left to bundle.sh because this script's next act
# is a release build and failing before it costs seconds instead of minutes.
command -v brew >/dev/null 2>&1 || {
    echo "error: 'brew' not found. This project's macOS build takes gtk4," >&2
    echo "  gtksourceview5 and adwaita-icon-theme from Homebrew (see README" >&2
    echo "  Quickstart); bundle.sh copies that closure into the .app." >&2
    echo "  Nothing is installed INTO Homebrew's prefix — the CLI goes to" >&2
    echo "  $BIN_DIR — but the build cannot run without it." >&2
    exit 1
}

# READ, NOT RESTATED. `packaging/macos/Info.plist.in` is where the bundle's
# CFBundleIdentifier is declared, and `cargo xtask lint-references` check 7 already holds
# that file against `src/icons.rs`. Deriving the value here keeps this script out of the
# set of files that can drift from the canonical ID — a literal copy here would be a new
# restatement with nothing checking it, which is the defect that check exists to prevent.
APP_ID="$(plutil -extract CFBundleIdentifier raw "$REPO_ROOT/packaging/macos/Info.plist.in")"
[ -n "$APP_ID" ] || {
    echo "error: could not read CFBundleIdentifier from packaging/macos/Info.plist.in" >&2
    exit 1
}

# --- Gate 1: exactly one Scribobulate bundle on this machine ----------------------
#
# TWO TIERS, AND THE MANDATORY ONE DOES NOT DEPEND ON AN INDEXER. The direct test of the
# two fixed locations is the gate; Spotlight is an extra that can widen it and must never
# be able to narrow it. `mdfind` was chosen over `lsregister -dump` on measurement, not
# taste: mdfind answered in 0.06s with exactly the live path, while lsregister took 2.1s
# and returned a pile of registrations for bundles that no longer exist (deleted /tmp
# staging directories, an unmounted .dmg volume) — a gate built on it would refuse to run
# over copies that are not there. Every mdfind hit is therefore existence-tested anyway.
#
# KNOW WHERE THE mdfind TIER STOPS. It is a Spotlight-INDEX query, so its coverage is
# whatever Spotlight indexes and it is blind everywhere Spotlight is excluded — which is
# not a corner case. MEASURED on this machine: /private/tmp held FIFTEEN real bundles
# (841 MB, executables present, every one carrying this identifier and registered with
# Launch Services) left by an earlier porting session, and mdfind returned exactly ONE
# path. Spotlight does not index /private/tmp; Launch Services registers it.
#
# That is a bound on the tier, not a hole in the gate, and the distinction is the reason
# the mandatory tier is a direct test rather than a query. The two fixed locations are the
# ones that decide the outcome — Launch Services ranks by location, and a /private/tmp
# bundle cannot outrank /Applications — so what mdfind misses cannot win. Do not promote
# mdfind to the mandatory tier on the strength of it usually agreeing.
found_foreign=""
note_transient=""

add_foreign() {
    case "$found_foreign" in
        *"$1"$'\n'*) ;;
        *) found_foreign="$found_foreign$1"$'\n' ;;
    esac
}

[ -d "$FOREIGN" ] && add_foreign "$FOREIGN"

if command -v mdfind >/dev/null 2>&1; then
    while IFS= read -r hit; do
        [ -n "$hit" ] || continue
        [ -d "$hit" ] || continue          # a stale index entry is not an installed copy
        case "$hit" in
            "$ANCHOR" | "$BUILT") continue ;;
            # A mounted disk image and the Trash both hold a bundle that is on the machine
            # without being installed on it; naming them as a refusal would make this gate
            # fire on an ordinary "I just opened the .dmg to look at it".
            #
            # $USER_HOME, NOT $HOME: under sudo the environment's home may be root's, and
            # this arm would then fail to recognise the operator's own Trash and report a
            # bundle they had already thrown away as an installed conflict.
            /Volumes/* | "$USER_HOME"/.Trash/*) note_transient="$note_transient$hit"$'\n' ;;
            *) add_foreign "$hit" ;;
        esac
    # AS THE INVOKING USER: mdfind queries a per-user Spotlight index and answers for
    # whoever runs it, so under sudo root's answer is not a statement about the operator's
    # machine at all.
    done < <(as_user mdfind "kMDItemCFBundleIdentifier == '$APP_ID'" 2>/dev/null || true)
fi

if [ -n "$found_foreign" ]; then
    echo "error: another Scribobulate.app is already installed on this machine." >&2
    echo >&2
    printf '%s' "$found_foreign" | while IFS= read -r p; do
        [ -n "$p" ] && echo "    $p" >&2
    done
    echo >&2
    echo "  It carries the same CFBundleIdentifier ($APP_ID) as the bundle this" >&2
    echo "  script builds, so both would register as the same application and the Dock," >&2
    echo "  'open -a Scribobulate' and the PATH command could end up running different" >&2
    echo "  copies. Nothing warns you when they diverge." >&2
    echo >&2
    echo "  Choose one and re-run:" >&2
    # THE OTHER MODE'S UNINSTALL IS THE FIRST REMEDY, not `rm -rf`. It is one command, it
    # is the exact inverse of whichever run created that bundle, and it also clears the
    # Launch Services registration — which `rm -rf` does not, leaving a stale Dock and
    # "Open With" entry pointing at a path that no longer exists.
    if printf '%s' "$found_foreign" | grep -qxF "$FOREIGN"; then
        echo "    cd '$REPO_ROOT' && $FOREIGN_UNINSTALL" >&2
        echo "        # removes $FOREIGN — the $OTHER_LABEL" >&2
        echo "        # install, i.e. this script run the other way — and unregisters it." >&2
        echo >&2
    fi
    echo "  Or remove the copy directly (leaves a Launch Services entry behind):" >&2
    printf '%s' "$found_foreign" | while IFS= read -r p; do
        [ -n "$p" ] && echo "    rm -rf '$p'" >&2
    done
    echo "    (or keep it, and do not run the developer install on this machine)" >&2
    echo >&2
    echo "  This script does not delete a bundle it did not install." >&2
    exit 1
fi

# --- Gate 2: nothing else answers to `scribobulate` on PATH -----------------------
#
# `[ -e ] || [ -L ]`, NOT `[ -e ]` ALONE, and the whole gate turns on it: the entry this
# was written to catch was a DANGLING symlink, for which `[ -e ]` is false and `[ -L ]`
# true. A gate that tested existence would have reported the machine clean on exactly the
# configuration that produced the bug.
#
# POSITION IS TAKEN FROM PATH ORDER, NOT FROM FINDING OUR OWN LINK. The obvious
# implementation — walk the hits and call everything after ours "later" — is wrong on the
# FIRST run, when our link does not exist yet: every hit would then read as earlier than a
# directory that has nothing in it, and an ordinary machine with one stale binary anywhere
# on PATH could never install. BIN_DIR's position in PATH is known without looking at the
# filesystem, so the classification uses that. A BIN_DIR that is not on PATH at all leaves
# every hit classified as earlier, which is correct: nothing we install can win from a
# directory that is never searched.
scan_path() {
    local dir p pos=before
    local IFS=:
    for dir in $PATH; do
        [ -n "$dir" ] || dir="."
        p="$dir/scribobulate"
        if [ "$dir" = "$BIN_DIR" ]; then
            pos=after
            if [ -e "$p" ] || [ -L "$p" ]; then
                printf 'ours\t%s\n' "$p"
            fi
            continue
        fi
        if [ -e "$p" ] || [ -L "$p" ]; then
            printf '%s\t%s\n' "$pos" "$p"
        fi
    done
}

# Prints nothing and returns 0 when clean; otherwise reports and returns 1 for a
# shadowing entry. Called twice — before the build so a refusal is cheap, and after the
# link is made, because until then half of what it inspects does not exist yet.
check_path() {
    local shadowing="" trailing="" pos p
    while IFS="$(printf '\t')" read -r pos p; do
        [ -n "$p" ] || continue
        case "$pos" in
            # In BIN_DIR, and ours only if it is the symlink this script creates. A regular
            # file here came from somewhere else and shadows us from our own directory.
            ours)   [ -L "$p" ] || shadowing="$shadowing$p"$'\n' ;;
            before) shadowing="$shadowing$p"$'\n' ;;
            *)      trailing="$trailing$p"$'\n' ;;
        esac
    done < <(scan_path | awk -F'\t' '!seen[$2]++')

    if [ -n "$trailing" ]; then
        echo >&2
        echo "NOTE: another 'scribobulate' is on PATH after $BIN_DIR:" >&2
        printf '%s' "$trailing" | while IFS= read -r p; do
            [ -n "$p" ] && echo "    $p -> $(readlink "$p" 2>/dev/null || echo 'regular file')" >&2
        done
        echo "  Ours wins today because it comes first, which is an accident of PATH" >&2
        echo "  order rather than a decision. Remove it when convenient." >&2
    fi

    [ -n "$shadowing" ] || return 0

    echo "error: another 'scribobulate' on PATH would win over this install." >&2
    echo >&2
    printf '%s' "$shadowing" | while IFS= read -r p; do
        [ -n "$p" ] && echo "    $p -> $(readlink "$p" 2>/dev/null || echo 'regular file')" >&2
    done
    echo >&2
    echo "  These are searched before $BIN_DIR, where this script puts its symlink, so" >&2
    echo "  typing 'scribobulate' would keep running one of them. A DANGLING symlink" >&2
    echo "  counts: the shell skips it silently rather than failing, so it does not" >&2
    echo "  protect you — it just moves the resolution further down PATH." >&2
    echo >&2
    echo "  Remove it and re-run:" >&2
    printf '%s' "$shadowing" | while IFS= read -r p; do
        [ -n "$p" ] && echo "    rm -f '$p'" >&2
    done
    echo >&2
    echo "  Two likely origins. A bundle installed in the OTHER mode leaves a link in" >&2
    echo "  that mode's bin directory — remove it with '$FOREIGN_UNINSTALL'. Otherwise" >&2
    echo "  it predates this layout: earlier revisions put the link in Homebrew's bin/," >&2
    echo "  and no current script looks there, so it has to be removed by hand." >&2
    return 1
}

check_path || exit 1

# --- Build, anchor, link ----------------------------------------------------------
echo ":: Building Scribobulate.app"
# AS THE INVOKING USER, ALWAYS — never as root. bundle.sh's first act is
# `cargo build --release`, and run as root that leaves target/ owned by root (the
# operator's next plain `cargo build` then fails on its own build directory) and writes
# root-owned files into their ~/.cargo. In per-user mode as_user is a plain call, so this
# is the same line it always was.
as_user "$REPO_ROOT/packaging/macos/bundle.sh" "$OUT_DIR"

# WHETHER SOMETHING WAS ALREADY THERE IS WORTH SAYING OUT LOUD. Overwriting the anchor is
# what installing means and this script has always done it, but in global mode the thing
# being overwritten is plausibly a copy the operator dragged from the .dmg rather than a
# previous run of this script. Replacing it silently is how they would find out later.
ANCHOR_PREEXISTED=""
[ -d "$ANCHOR" ] && ANCHOR_PREEXISTED=1

echo ":: Anchoring $ANCHOR"
[ -n "$ANCHOR_PREEXISTED" ] && echo "   (replacing the bundle already at that path)"
mkdir -p "$ANCHOR_DIR"
rm -rf "$ANCHOR"
# `ditto` rather than `cp -R`: it is the macOS-native copy and preserves extended
# attributes and the ad-hoc code signature bundle.sh applies. A bundle whose signature
# did not survive the copy is refused at launch in a way that reads as corruption.
ditto "$BUILT" "$ANCHOR"
codesign --verify --deep --strict "$ANCHOR" 2>/dev/null || {
    echo "error: the code signature did not survive the copy to $ANCHOR" >&2
    echo "  bundle.sh signs the bundle ad-hoc and macOS refuses a bundle whose" >&2
    echo "  signature does not verify, reporting it as damaged." >&2
    exit 1
}

# AND THEN THE BUILD COPY GOES, or this script leaves behind exactly the second
# registration its own gate refuses. MEASURED with a probe pair sharing one identifier,
# one in ~/Applications and one in target/macos, neither ever launched and neither
# manually registered: BOTH appear in `lsregister -dump` within seconds — a build
# directory is not exempt from Launch Services by virtue of being a build directory —
# and `open -b <id>` launches the ~/Applications one. Remove the anchor and the same
# command launches the target/macos one. So the build copy does not merely sit there: it
# is a live candidate that loses while the anchor exists and takes over the moment it
# does not, which is a stale build-directory bundle silently becoming the application.
#
# Deleting it is not the overreach the /Applications rule forbids. The distinction is
# authorship, not location: bundle.sh produced this one seconds ago in this same run,
# where a dragged copy was installed by the user through a different route. Only the
# .app is removed, never OUT_DIR itself, which may be a directory the caller named.
# dmg.sh is unaffected — it invokes bundle.sh itself rather than consuming a bundle
# somebody else left.
rm -rf "$BUILT"

TARGET="$ANCHOR/Contents/MacOS/scribobulate"
echo ":: Linking $LINK -> $TARGET"
mkdir -p "$BIN_DIR"
ln -sf "$TARGET" "$LINK"

# --- Manual pages ---------------------------------------------------------------
#
# THE DIRECTORY FOLLOWS THE MODE, exactly as the executable above does, and the two modes
# stand on different ground.
#
# GLOBAL: /usr/local/share/man is listed in /etc/manpaths, which path_helper composes into
# the default manpath before any profile runs -- MEASURED on this machine. One wrinkle
# worth knowing rather than rediscovering: `manpath` OMITS entries that do not exist, so
# that directory is absent from its output until something creates it. Creating it, which
# the mkdir below does, is what puts it on the search path.
#
# PER-USER: ~/.local/share/man is an XDG convention macOS knows nothing about, and nothing
# on this platform adds it. It is chosen for consistency with packaging/linux/install.sh
# rather than because macOS blesses it, and the run REPORTS that it is not searched, with
# the line to add. That report is the honest option; the alternative previously taken here
# -- writing into Homebrew's prefix because it happened to already be searched -- bought
# silence with a directory this project does not own.
#
# EITHER WAY THE CHECK BELOW IS THE AUTHORITY, not this comment: it asks `manpath` on the
# host actually running and reports what it finds.
#
# SYMLINKS INTO THE BUNDLE, for the same reason the executable is one: bundle.sh already
# staged the substituted, compressed pages into Contents/Resources/man, and a copy here
# would be a second original to drift. They resolve into the ANCHOR, not into the build
# directory, so `cargo clean` no longer dangles them. They still inherit the dangling-link
# failure mode when the anchored .app is deleted -- uninstall.sh tests with `[ -L ]`, which
# is true for a dangling link where `[ -e ]` is false.
for section in 1 5; do
    man_src="$ANCHOR/Contents/Resources/man/man$section/scribobulate.$section.gz"
    man_link="$MAN_DIR/man$section/scribobulate.$section.gz"
    [ -f "$man_src" ] || { echo "error: bundle.sh did not stage $man_src" >&2; exit 1; }
    echo ":: Linking $man_link -> $man_src"
    mkdir -p "$MAN_DIR/man$section"
    ln -sf "$man_src" "$man_link"
done

# REPORTED, not assumed to have worked. A link in a directory man does not search is
# indistinguishable from a successful install until someone runs `man scribobulate` and
# gets nothing -- the same failure the PATH check below exists to pre-empt.
if command -v manpath >/dev/null 2>&1; then
    case ":$(manpath 2>/dev/null):" in
        *":$MAN_DIR:"*) ;;
        *)
            echo
            echo "NOTE: $MAN_DIR is not in your 'manpath' output, so 'man scribobulate'"
            echo "may not find the pages. Add it to your shell profile:"
            echo
            echo "    export MANPATH=\"$MAN_DIR:\$MANPATH\""
            echo
            echo "Or read them directly, which needs no configuration:"
            echo "  man $ANCHOR/Contents/Resources/man/man1/scribobulate.1.gz"
            ;;
    esac
fi

# The second run of the PATH gate. The first proved nothing shadowed us; this one proves
# the link we just made is what resolves, which is a different claim and the one the
# operator actually cares about.
check_path || exit 1

if [ -n "$note_transient" ]; then
    echo
    echo "NOTE: a Scribobulate.app is also present at:"
    printf '%s' "$note_transient" | while IFS= read -r p; do
        [ -n "$p" ] && echo "    $p"
    done
    echo "  Not installed (a mounted disk image or the Trash), so it is not a conflict"
    echo "  today. It becomes one if it is copied to $FOREIGN_DIR or $ANCHOR_DIR."
fi

echo
echo "Installed ($MODE_LABEL)."
echo "  app : $ANCHOR"
echo "  cli : $LINK"
echo "  man : $MAN_DIR/man{1,5}/scribobulate.{1,5}.gz"
echo
echo "  Exactly one Scribobulate.app is installed. The build copy this run produced at"
echo "  $BUILT"
echo "  has been removed: Launch Services registers a bundle in a build directory like"
echo "  any other, so leaving it would be leaving a second candidate for the same"
echo "  identifier. Nothing above resolves into the build tree, so 'cargo clean' is safe."
case ":$PATH:" in
    *":$BIN_DIR:"*)
        echo
        echo "Open a new terminal and run: scribobulate path/to/document.md"
        ;;
    *)
        echo
        echo "NOTE: $BIN_DIR is not on your PATH, so typing 'scribobulate' will not find"
        echo "it. Add it to your shell profile (~/.zshrc on a stock macOS shell):"
        echo
        echo "    export PATH=\"$BIN_DIR:\$PATH\""
        echo
        echo "Until then, run the command by its full path: $LINK"
        ;;
esac
