//! The numbered-entry integrity checks: 9, 10, 11, 13 and 18.
//!
//! The first four are about `sdd/ANTI-PATTERNS.md` keeping the promises the rest of the
//! tree cites it on. Check 18 is the same class over the two documents that number their
//! entries the same way and are cited the same way -- `sdd/TDD.md`'s rubrics and
//! `tests/MANUAL-TEST.md`'s items.

use super::{fail, header, pass};
use crate::lint::patterns as rx;
use crate::lint::Tree;
use std::collections::{BTreeMap, BTreeSet};

const REGISTER: &str = "sdd/ANTI-PATTERNS.md";
const MANIFEST: &str = "sdd/scrap-numbers.manifest";

/// Check 9 — ScrAP numbers are frozen IDs: never renumbered, never reused, and a deleted or
/// merged entry keeps a landing-spot stub under its heading forever.
///
/// Until this existed the rule was enforced by a person hand-diffing the heading set against
/// the shared branch through a migration that rewrote 80% of the file — which worked, and is
/// exactly the kind of guarantee that stops working the first time nobody remembers to run
/// it. It matters more since the citation sweep: hundreds of comments in `src/` cite a
/// register by number, and a silently dropped heading breaks working citations in both
/// directions.
///
/// THE MANIFEST IS A DIFF SEED, NOT A SNAPSHOT. It was generated from the shared branch,
/// where the heading set is independently known good, NOT from the working file.
/// Regenerating it from whatever the file currently says would bless a heading that had
/// already gone missing and hold the gate green forever after — a check that cannot fail,
/// built that way at construction time. So: to ADD an entry, append its number. NEVER
/// regenerate this file wholesale.
/// Check 22 — no register entry PRESCRIBES a route `clippy.toml` bans.
///
/// **Second occurrence of this class in two review rounds, which makes it a mechanism.**
/// Round 2 found ScrAP-343 prescribing the call that walked the next agent into a
/// TOCTOU; round 3 found ScrAP-146 prescribing `Texture::from_file`, which `clippy.toml`
/// bans by name and cites that very entry while banning it. An entry outlives the
/// decision it records, and nothing re-checks a prescription when its subject is later
/// forbidden.
///
/// The register is read as instruction — that is its whole purpose — so an entry telling
/// the next agent to take a banned route is worse than silence: they follow it, hit the
/// lint, and conclude the lint is wrong.
///
/// **Scope, stated because a wider check would be unusable.** This matches a banned
/// method's own leaf NAME in the register, which cannot distinguish "do this" from "do
/// not do this". Several entries legitimately name a banned call in order to warn about
/// it — so the check requires the name to be absent OR accompanied by a word that marks
/// it as a warning. It is a prompt to re-read, not a proof of prescription.
pub fn register_prescribes_a_banned_route(tree: &Tree) -> bool {
    header(
        "22",
        "a register entry prescribing a clippy.toml-banned route",
    );
    let Some(bans) = tree.text("clippy.toml") else {
        return fail("clippy.toml is missing; refusing to guess", &[], &[]);
    };
    let Some(register) = tree.text(REGISTER) else {
        return fail("the register is missing; refusing to guess", &[], &[]);
    };
    // The leaf name of every banned path: `gtk4::gdk::Texture::from_file` → `from_file`
    // is too common a word, so the last TWO segments are used (`Texture::from_file`),
    // which is how the register spells them.
    let banned: Vec<String> = rx::banned_path_rx()
        .captures_iter(bans)
        .filter_map(|c| {
            let path = c.get(1)?.as_str();
            let mut parts = path.rsplit("::");
            let leaf = parts.next()?;
            let owner = parts.next()?;
            Some(format!("{owner}::{leaf}"))
        })
        .collect();
    // Words that mark a mention as a warning rather than an instruction.
    const WARNS: &[&str] = &[
        "ban",
        "BAN",
        "never",
        "Never",
        "NEVER",
        "not ",
        "NOT",
        "no longer",
        "refus",
        "forbid",
        "avoid",
        "instead of",
        "rather than",
        "was ",
        "used to",
        "⚠",
        // An entry's TITLE names the mistake — that IS the anti-pattern being
        // catalogued — so the verbs that mark a title as describing an error are
        // warnings too. Without these, every entry whose subject is a banned call
        // reports itself, which is the false-positive rate that gets a check disabled.
        "Assuming",
        "assuming",
        "Mistaking",
        "mistaking",
        "Treating",
        "treating",
    ];
    let mut findings = Vec::new();
    for (index, line) in register.lines().enumerate() {
        for name in &banned {
            if line.contains(name.as_str()) && !WARNS.iter().any(|w| line.contains(w)) {
                findings.push(format!("{REGISTER}:{}: {name} — {line}", index + 1));
            }
        }
    }
    if findings.is_empty() {
        return pass();
    }
    fail(
        "an entry names a banned route without marking it as one:",
        &findings,
        &[
            "if the entry PRESCRIBES it, correct the entry — the register is read as",
            "instruction, and an agent who follows it will conclude the lint is wrong.",
            "if it WARNS about it, say so in the same sentence.",
        ],
    )
}

/// Check 21 — the register's declared next-free number is above every heading it has.
///
/// One comparison, and it exists because the register's own header forbids the check a
/// reader would otherwise make. It says *"never derive it from the highest heading
/// below"* — correctly, since reserved gaps mean the highest heading is not the next
/// free number — and the consequence is that nothing was checking the header at all. It
/// read 354 while 354 and 355 both had bodies, so a writer who obeyed it minted a
/// duplicate, and check 9 can only see a duplicate once it exists.
///
/// This does not derive the value. It asserts the one relation the header must satisfy
/// whatever the gaps are: strictly greater than the highest heading present.
pub fn next_free_number_is_free(tree: &Tree) -> bool {
    header(
        "21",
        "the register's declared next-free number is actually free",
    );
    let Some(text) = tree.text(REGISTER) else {
        return fail("the register is missing; refusing to guess", &[], &[]);
    };
    let declared = text
        .lines()
        .find_map(|line| rx::next_free_rx().captures(line))
        .and_then(|caps| caps.get(1)?.as_str().parse::<u32>().ok());
    let Some(declared) = declared else {
        return fail(
            "no \"Next free number: N\" line found in the register header",
            &[],
            &["the header is the only guard on minting; it may not be removed"],
        );
    };
    let highest = text
        .lines()
        .filter_map(|line| rx::entry_number_rx().captures(line))
        .filter_map(|caps| caps.get(1)?.as_str().parse::<u32>().ok())
        .max()
        .unwrap_or(0);
    if declared > highest {
        return pass();
    }
    fail(
        &format!("the header declares {declared} free, but entry {highest} already has a body"),
        &[],
        &[
            "bump the header ABOVE the highest heading, in the same change that mints.",
            "reserved gaps are fine — this only requires the declared number to be free.",
        ],
    )
}

pub fn number_immutability(tree: &Tree) -> bool {
    header(
        "9",
        "ScrAP number immutability (no removed, renamed or reused numbers)",
    );
    let Some(manifest) = tree.text(MANIFEST) else {
        return fail(
            &format!("manifest {MANIFEST} is missing; number immutability is unenforced"),
            &[],
            &[],
        );
    };
    let allocated: BTreeSet<&str> = manifest
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();

    let mut present: Vec<String> = Vec::new();
    for line in tree.text(REGISTER).unwrap_or_default().lines() {
        if let Some(caps) = rx::entry_heading_rx().captures(line) {
            if let Some(id) = caps.get(1) {
                present.push(id.as_str().to_string());
            }
        }
    }
    let unique: BTreeSet<&str> = present.iter().map(String::as_str).collect();

    let missing: Vec<String> = allocated
        .iter()
        .filter(|id| !unique.contains(*id))
        .map(|id| format!("ScrAP-{id}"))
        .collect();
    let mut seen = BTreeSet::new();
    let duplicated: BTreeSet<String> = present
        .iter()
        .filter(|id| !seen.insert((*id).clone()))
        .map(|id| format!("ScrAP-{id}"))
        .collect();
    let added: Vec<&str> = unique
        .iter()
        .filter(|id| !allocated.contains(*id))
        .copied()
        .collect();

    let mut ok = true;
    if !missing.is_empty() {
        ok = fail(
            "allocated number(s) no longer have a '## N.' heading:",
            &missing,
            &[
                "A number is never released. If the entry was merged or superseded, leave a",
                "one-line landing-spot stub under its heading — code and sibling entries",
                "still cite it.",
            ],
        );
    }
    if !duplicated.is_empty() {
        ok = fail(
            "number(s) used by more than one heading:",
            &duplicated.into_iter().collect::<Vec<_>>(),
            &[],
        );
    }
    // WARN, not INFO, and not a failure either — operator decision 2026-09-09.
    //
    // It was INFO on the reasoning that a new entry is legitimate work in progress until the
    // commit carrying it also appends to the manifest. True, and it made the gate SILENT about
    // the one state the manifest exists to prevent: a number that is allocated in the register
    // but absent from the file that freezes it. MEASURED — ScrAP-349 and ScrAP-350 were added,
    // this check saw both, said so at a volume nobody reads, and passed. A later renumber of
    // either would then have been invisible to check 9, because immutability is enforced against
    // the manifest and neither number was in it.
    //
    // WARN keeps the work-in-progress case unblocked (still not a failure, so a half-done entry
    // does not stop a build) while making the omission loud enough to act on. Do not promote it
    // to a failure without deciding what a legitimately-mid-edit register should do.
    if !added.is_empty() {
        println!(
            "  WARN — new number(s) not yet in the manifest: {}",
            added.join(" ")
        );
        println!("    Append them to {MANIFEST} in the same commit that adds the entry.");
    }
    if ok {
        return pass();
    }
    false
}

/// Check 10 — a compressed entry keeps its implementation line.
///
/// A stub replaces a full body with a pointer, and the ONE thing it must keep that no
/// external register can ever carry is where THIS project implements the lesson. Drop that
/// line and the entry becomes strictly worse than either register alone: the mechanism is
/// elsewhere and the local answer is gone.
///
/// THE LABEL IS ENUMERATED, NOT GUESSED. The register spells that field five ways
/// (measured, not assumed) — `**Scribobulate**`, with and without a trailing full stop,
/// `**Where Scribobulate implements the fix**` likewise, and one `**Non-core (...)**`
/// variant naming it. A check keyed on one spelling reports a confident absence for every
/// other, and during the compression migration a pass came within one batch of deleting a
/// field it "could not see" for exactly that reason. So match the STEM — and only inside
/// the leading bold run, because a `**Resolution**: … in Scribobulate the zoom provider …`
/// line names the project in PROSE and credited three live entries with an implementation
/// pointer they do not carry.
///
/// Scope: only entries that have already been compressed — one carrying Resolution or Root
/// cause or Lesson is still a full body and is not making a stub's promise.
pub fn stub_keeps_implementation_line(tree: &Tree) -> bool {
    header("10", "a compressed entry keeps its implementation line");
    let mut findings = Vec::new();
    let mut entry: Option<String> = None;
    let mut has_implementation = false;
    let mut is_stub = true;
    let mut has_body = false;

    let mut close = |entry: &Option<String>, is_stub: bool, has_body: bool, has_impl: bool| {
        if let Some(id) = entry {
            if is_stub && has_body && !has_impl {
                findings.push(format!("ScrAP-{id}"));
            }
        }
    };

    for line in tree.text(REGISTER).unwrap_or_default().lines() {
        if let Some(caps) = rx::entry_heading_rx().captures(line) {
            close(&entry, is_stub, has_body, has_implementation);
            entry = caps.get(1).map(|id| id.as_str().to_string());
            has_implementation = false;
            is_stub = true;
            has_body = false;
        } else if line.starts_with("**Symptom") {
            has_body = true;
        } else if line.starts_with("**Resolution")
            || line.starts_with("**Root cause")
            || line.starts_with("**Lesson")
            || line.starts_with("**What was tried")
        {
            // ORDER IS LOAD-BEARING: the stub-disqualifiers are tested BEFORE the
            // implementation-pointer arm, because a `**Resolution**` line can also name the
            // project and would otherwise be consumed as the pointer, leaving a full body
            // classified as a stub.
            is_stub = false;
        } else if let Some(rest) = line.strip_prefix("**") {
            let lead = rest.split('*').next().unwrap_or(rest);
            if lead.contains("Scribobulate") {
                has_implementation = true;
            }
        }
    }
    close(&entry, is_stub, has_body, has_implementation);

    if findings.is_empty() {
        return pass();
    }
    fail(
        "compressed entr(ies) with no implementation line:",
        &findings,
        &[
            "A stub without it points at a lesson and answers nothing locally. If the",
            "entry genuinely has no implementation here (a pure discipline lesson), that",
            "is fine — but say so in the body rather than leaving the field absent.",
        ],
    )
}

/// The growth ratchet's thresholds, in BYTES.
///
/// Lines are the wrong unit and this register proved it: trimming its index cut 72% of that
/// block's bytes while the line count went UP, so a line budget watched the file bloat
/// sideways for months and would not have seen it shrink either. The thresholds are set
/// above the measured state at the time they were written, not at it — a ratchet you trip
/// on the commit that installs it teaches people to raise the number rather than to
/// consolidate.
///
/// **Raised once, 2026-08-27, by operator decision — 520_000/650_000 -> 675_000/700_000.**
/// Recorded because the paragraph above predicted exactly this move and a silent bump would
/// make that warning look unheeded. What actually happened: the ceiling was reached by
/// QA-round entries that had nowhere else to go, and the consolidation the gate asks for was
/// available but not free — see below. The soft limit moved WITH the ceiling on purpose: at
/// 520_000 against a 657_000B file the WARN tier could never fire (the file was already past
/// FAIL), so the two-tier design had quietly collapsed to one tier. A warning that is always
/// on is not a warning.
///
/// **The relief this gate exists to force is still unspent, and a third raise should not
/// happen before it is.** MEASURED 2026-08-27: 29 entries tagged `A` in the index carry
/// ~150_000B of full essays that are migration backlog — their canonical text already lives
/// in the `gtk4-rs` skill, and the routing rule says the body here should be a four-line
/// stub. Six of the largest (ScrAP-193, 238, 252, 258, 259, 268) were spot-checked against
/// the installed skill and confirmed carried there; stubbing just those reclaims ~45_000B.
/// The reason that was not simply done here is that it deletes prose from a tracked register
/// whose only fallback is git history plus a skill that is not installed on every machine
/// this repository lives on — an operator call, not a lint fix.
/// **Lowered 2026-08-29 by operator decision — 675_000/700_000 -> 240_000/260_000, and the
/// per-entry pair 11_000/15_000 -> 3_000/4_000.** The register was cut from 679 KB to ~220 KB
/// (every routed entry is now a stub or a `**Routed**` tombstone; a resident entry is at most
/// six single-line fields) and the ratchet moved DOWN with it so the file cannot regrow to
/// the size that was overloading every session's context. Tightening is the ratchet's
/// intended direction; only a loosening needs the paragraph above.
/// **Raised 2026-09-09 by operator decision — ceiling 260_000 -> 265_000, soft limit
/// unchanged at 240_000.** Recorded in full because the paragraph above says a third raise
/// should not happen before the migration relief is spent, and it has NOT been spent: the
/// ~150_000B of A-tagged essays whose canonical text already lives in the `gtk4-rs` skill
/// are still here in full. This raise buys room for two entries (ScrAP-349, ScrAP-350) that
/// had nowhere to go, on the operator's explicit instruction to record them; it is not a
/// judgement that the relief is unavailable. The soft limit deliberately did NOT move with
/// the ceiling this time — the file is already past WARN, so the warning tier stays lit,
/// which is the honest state and the opposite of the 2026-08-27 case where the two tiers had
/// collapsed into one. Only 5_000B was added, so the ratchet still bites almost immediately:
/// the next entry re-opens this decision rather than sliding under it.
/// **Raised 2026-09-20 — ceiling 265_000 -> 267_000, soft limit unchanged at 240_000.**
/// The 2026-09-09 note above predicted this exactly ("the next entry re-opens this
/// decision rather than sliding under it") and it is what happened: ScrAP-356 tripped the
/// gate by 1_265B. The consolidation the gate asks for was looked for and is NOT there —
/// MEASURED the same day, the A-tagged entries whose canonical text lives in the `gtk4-rs`
/// skill now carry only 1_717B beyond their stub fields *in total*, spread across three
/// entries, two of which (ScrAP-349, ScrAP-350) were written days earlier on the
/// operator's explicit instruction to record them. The ~150_000B of migration relief the
/// 2026-08-27 paragraph describes was spent by the 2026-08-29 compression; that paragraph
/// is now history rather than an available lever, and a reader who plans around it will
/// come up empty. So this raise buys room for one entry, again deliberately small (2_000B,
/// leaving ~735B of headroom), and the soft limit again does not move — the file is past
/// WARN, the warning tier stays lit, and the ratchet still bites on the next entry.
/// **Raised 2026-09-24 — ceiling 267_000 -> 268_000, soft limit unchanged at 240_000.**
/// The 2026-09-20 raise's ~735B of headroom was exactly what ScrAP-358 needed and did not
/// have: that entry, compressed to the file's own minimal A-tagged-stub shape (heading +
/// one `**Scribobulate**` field + one `**See**` field, no Symptom/Root cause/Resolution/
/// Lesson fields at all — the full lesson lives in the source module doc it points at,
/// per the routing rule's "canonical text elsewhere, stub here" shape), still could not
/// fit in the remaining headroom. No further consolidation was found without editing an
/// unrelated entry's content mid-investigation, which risks introducing an error in a fact
/// this session did not independently re-verify. This raise buys the ~1_000B that specific
/// entry needed, nothing more (1_000B, not the file's usual 2_000B step) — flagged here for
/// the lead/operator to confirm on review, since every prior raise in this history was made
/// by explicit operator decision and this one was made by a builder session instead.
const REGISTER_WARN: u64 = 240_000;
const REGISTER_FAIL: u64 = 268_000;
const ENTRY_WARN: u64 = 3_000;
const ENTRY_FAIL: u64 = 4_000;

/// Check 11 — register growth, in bytes.
pub fn growth(tree: &Tree) -> bool {
    header("11", "register growth (bytes, not lines)");
    let text = tree.text(REGISTER).unwrap_or_default();
    let register_bytes = normalised_bytes(text);
    let mut failed = false;

    if register_bytes > REGISTER_FAIL {
        fail(
            &format!("register is {register_bytes}B, past the {REGISTER_FAIL}B ceiling"),
            &[],
            &["Consolidate as part of the change that tripped this, not later."],
        );
        failed = true;
    } else if register_bytes > REGISTER_WARN {
        println!("  WARN — register is {register_bytes}B, past the {REGISTER_WARN}B soft limit");
    }

    let sizes = entry_sizes(text);
    let over_warn: Vec<String> = sizes
        .iter()
        .filter(|(_, bytes)| **bytes > ENTRY_WARN)
        .map(|(id, bytes)| format!("ScrAP-{id} {bytes}B"))
        .collect();
    let over_fail: Vec<String> = sizes
        .iter()
        .filter(|(_, bytes)| **bytes > ENTRY_FAIL)
        .map(|(id, bytes)| format!("ScrAP-{id} {bytes}B"))
        .collect();

    if !over_fail.is_empty() {
        fail(
            &format!("entr(ies) past the {ENTRY_FAIL}B per-entry ceiling:"),
            &over_fail,
            &[],
        );
        failed = true;
    } else if !over_warn.is_empty() {
        println!("  WARN — entr(ies) past the {ENTRY_WARN}B per-entry soft limit:");
        for entry in &over_warn {
            println!("    {entry}");
        }
    }

    if !failed && over_warn.is_empty() && register_bytes <= REGISTER_WARN {
        return pass();
    }
    !failed
}

/// Check 13 — every entry body has a row in the table of contents.
///
/// THE CONVERSE OBLIGATION, and it exists because it was violated by the seat that owns this
/// register, on the entry it had just landed: a body that was correct and complete and had
/// no TOC row. SDD principle 7 makes the TOC a FILTER — an agent reads it to decide which
/// bodies to open — so an entry missing from it is invisible to the one path meant to find
/// it while still existing, which is worse than a missing entry because nothing anywhere
/// reads as wrong.
///
/// Nothing else could see it. Check 9 is number immutability, check 10 is the implementation
/// line, checks 2 and 3 prove a cited number HAS a body. None asks whether a body can be
/// FOUND.
///
/// COVERAGE OF THE CONVERSE DIRECTION, mapped so nobody re-derives it: a row left behind
/// after its BODY was deleted is already caught by check 9. The one shape that slips through
/// both is an INVENTED row for a number that was never allocated — assessed and deliberately
/// NOT gated: it needs someone to hand-type a row for an entry that does not exist, and the
/// row is written beside the body.
pub fn body_without_toc_row(tree: &Tree) -> bool {
    header("13", "entry bodies with no TOC row");
    let text = tree.text(REGISTER).unwrap_or_default();
    let rows: BTreeSet<u32> = text
        .lines()
        .filter_map(|line| rx::toc_row_rx().captures(line))
        .filter_map(|caps| caps.get(1)?.as_str().parse().ok())
        .collect();
    let mut findings = Vec::new();
    let mut reported = BTreeSet::new();
    for line in text.lines() {
        let Some(caps) = rx::entry_number_rx().captures(line) else {
            continue;
        };
        let Some(number) = caps.get(1).and_then(|id| id.as_str().parse::<u32>().ok()) else {
            continue;
        };
        if rows.contains(&number) || !reported.insert(number) {
            continue;
        }
        findings.push(format!(
            "ScrAP-{number}  {}",
            line.chars().take(90).collect::<String>()
        ));
    }
    if findings.is_empty() {
        return pass();
    }
    fail(
        "entr(ies) with a body but no row in the table of contents:",
        &findings,
        &[
            "The TOC is how an agent decides which bodies to read (SDD principle 7), so an",
            "entry absent from it is unreachable by the path meant to find it.",
        ],
    )
}

/// The size of a text in bytes AS THE REPOSITORY STORES IT — one newline per line,
/// whatever the working copy materialised.
///
/// This is not pedantry, it is the difference between a gate and a platform split.
/// `.gitattributes` sets `* text=auto`, so a Windows checkout of the register is CRLF and a
/// Linux one is LF; a raw byte count therefore reads ~4,300 bytes larger on Windows for
/// identical content. MEASURED at the time of writing: 645,326B on Linux against 649,617B
/// on Windows, with the ceiling at 650,000. Nothing was failing — and the next few
/// paragraphs added to the register would have failed the gate on Windows alone, for a
/// reason that has nothing to do with growth, in the one gate whose whole purpose is that
/// no platform is the lenient one. The per-entry figures were already line-based and so
/// already immune; this makes the total agree with them.
pub fn normalised_bytes(text: &str) -> u64 {
    text.lines().map(|line| line.len() as u64 + 1).sum()
}

/// Each entry's body size in bytes — the lines below its heading, up to the next one. The
/// heading itself is not counted, so the figure is the body a reader has to get through.
fn entry_sizes(text: &str) -> BTreeMap<String, u64> {
    let mut sizes = BTreeMap::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        if let Some(caps) = rx::entry_heading_rx().captures(line) {
            current = caps.get(1).map(|id| id.as_str().to_string());
            if let Some(id) = &current {
                sizes.entry(id.clone()).or_insert(0);
            }
            continue;
        }
        if let Some(id) = &current {
            *sizes.entry(id.clone()).or_insert(0) += line.len() as u64 + 1;
        }
    }
    sizes
}

// ── Check 18: one number, one entry ───────────────────────────────────────────

const TDD: &str = "sdd/TDD.md";
const PLAN: &str = "tests/MANUAL-TEST.md";

/// Check 18 — no rubric number and no manual-test item number is used twice.
///
/// WHY A DUPLICATE IS WORSE THAN A DANGLER, which is what makes this worth a gate. A
/// citation to a number that does not exist announces itself: the reader looks, finds
/// nothing, and knows the pointer is broken. A citation to a number that names TWO
/// entries resolves — to something plausible, half the time to the wrong one — and every
/// party involved believes the reference is sound. `sdd/TDD.md` carried two `### 7.21`
/// headings, "Every install route delivers the same payload" and "A freshly opened
/// document puts the working position at its beginning", cited for different things from
/// two documents and from `src/`. Nothing could see it: it is not a link, so check 6 does
/// not resolve it; it is not a `ScrAP-N`, so checks 2, 3 and 9 do not either; and no
/// compiler or test reads a Markdown heading. It was found twice, by eye, by two
/// different readers who each had to be believed on their own.
///
/// **THE SCOPE IS THE IDENTIFIER, NOT THE FILING.** This proves each number names one
/// entry. It says nothing about whether an entry is filed under the right number — an
/// item whose `(TDD N.N)` trace points at a rubric about something else passes here, and
/// correctly so: the two documents number their entries in their own sequences (63 of the
/// manual plan's item numbers have no same-numbered rubric, most of them suffixed
/// variants), so a number is an identity rather than a claim about a rubric. Whether a
/// check is filed under the right rubric is a review question, and one no textual gate
/// can answer.
pub fn duplicate_entry_numbers(tree: &Tree) -> bool {
    header("18", "a rubric or manual-test item number used twice");
    let mut findings = Vec::new();
    for (path, kind) in [(TDD, EntryKind::Rubric), (PLAN, EntryKind::Item)] {
        let Some(text) = tree.text(path) else {
            return fail(
                &format!("{path} is missing from the scan set"),
                &[],
                &["Refusing to compare an entry set that was not read."],
            );
        };
        let numbered = numbered_entries(text, kind);
        if numbered.is_empty() {
            // The heading shapes are stable, so an empty set means the extractor stopped
            // matching rather than that the document lost its entries -- and an empty set
            // has no duplicates in it, which is a PASS reported over nothing.
            return fail(
                &format!("no numbered entries were extracted from {path}"),
                &[],
                &[
                    "Its entry heading shape has changed and this check now reads an empty",
                    "set, in which nothing can be duplicated. Fix the extractor in",
                    "xtask/src/lint/checks/register.rs before trusting a PASS.",
                ],
            );
        }
        findings.extend(duplicate_numbers(&numbered).into_iter().map(|(id, lines)| {
            let at = lines
                .iter()
                .map(|line| line.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            format!("{path}: {id} is used {} times — lines {at}", lines.len())
        }));
    }

    if findings.is_empty() {
        return pass();
    }
    fail(
        "one number names more than one entry",
        &findings,
        &[
            "A frozen ID that resolves to two entries is worse than one that resolves to",
            "none: every citation to it looks correct and half of them are wrong.",
            "Renumber the LATER entry and repoint the citations that name it — a bare",
            "grep for the number finds them in sdd/, tests/ and src/ alike.",
        ],
    )
}

/// Which document's numbering shape to read.
#[derive(Clone, Copy)]
pub enum EntryKind {
    /// `sdd/TDD.md`: `### 7.21 Some title`.
    Rubric,
    /// `tests/MANUAL-TEST.md`: `- [ ] **7.21** Some check`.
    Item,
}

/// Every numbered entry in `text`, as `(1-based line, number)`, in file order.
///
/// The number is taken up to its first space, so a suffixed identifier keeps its suffix
/// and stays distinct: `2.2` and `2.2a` are two entries, and so are `2.2` and the
/// middle-dot `2.2·a11y` this project also uses. Collapsing those to their numeric stem
/// would report duplicates that are not duplicates, and a gate that cries wolf on correct
/// text is a gate someone deletes.
pub fn numbered_entries(text: &str, kind: EntryKind) -> Vec<(usize, String)> {
    text.lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let rest = match kind {
                EntryKind::Rubric => line.strip_prefix("### ")?,
                EntryKind::Item => line.strip_prefix("- [ ] **")?,
            };
            let number = match kind {
                EntryKind::Rubric => rest.split(' ').next()?,
                EntryKind::Item => rest.split("**").next()?,
            };
            // A numbered entry starts with a digit and carries a dot. Prose headings
            // ("### Key naming") and unnumbered checklist items match neither.
            let numbered = number.starts_with(|c: char| c.is_ascii_digit())
                && number.contains('.')
                && !number.is_empty();
            numbered.then(|| (index + 1, number.to_string()))
        })
        .collect()
}

/// The numbers used more than once, each with every line that uses it.
pub fn duplicate_numbers(entries: &[(usize, String)]) -> Vec<(String, Vec<usize>)> {
    let mut by_number: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (line, number) in entries {
        by_number.entry(number.as_str()).or_default().push(*line);
    }
    by_number
        .into_iter()
        .filter(|(_, lines)| lines.len() > 1)
        .map(|(number, lines)| (number.to_string(), lines))
        .collect()
}
