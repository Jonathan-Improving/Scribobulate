//! Check 25's corpus: a citation of a numbered POLICY step, which POLICY no longer has.
//!
//! The cited document's name is spliced in at run time (`{P}`) so this file's own text
//! carries no citation for the check to find; every case still goes through the predicate
//! the check calls.

use crate::lint::patterns::policy_step_citations;

const P: &str = "POLICY";

/// Each was a real dangling citation in the tree (R6-SPEC-01), and each must flag.
#[test]
fn a_numbered_policy_step_is_flagged() {
    let cases = [
        format!("# Scoped coverage gate — {P}.md § \"Build pipeline\" step 5 states the rule"),
        format!("# header and {P} step 5. Sub-point movement is noise, not news."),
        format!("    /// What that cost was a coverage run — {P}'s build-pipeline step 5 —"),
        // Lower case, as `pipeline.steps` once wrote it in a reason every Linux run prints.
        format!("na.linux  integration  permanent  … ({} build pipeline step 5).", P.to_lowercase()),
        // Wrapped across a comment line: the name ends one line, the step opens the next.
        format!("//! asked directly, which is the extraction {P}\n//! § Build pipeline step 5 describes."),
        format!("# the cross-reference gate ({P} § Build\n# pipeline step 8)."),
    ];
    for case in &cases {
        assert_eq!(
            policy_step_citations(case),
            [1],
            "MISS (should flag): {case}"
        );
    }
    // The line reported is the one the citation STARTS on.
    assert_eq!(
        policy_step_citations(&format!("first\nsee {P}\n// step 2 here")),
        [2]
    );
}

#[test]
fn a_section_citation_or_a_contract_step_is_not_flagged() {
    let cases = [
        format!("# Scoped coverage gate — {P}.md § \"Build pipeline\" (coverage ratchet)."),
        format!("//! the extraction {P} § Build pipeline describes."),
        // The contract's steps still exist, and are cited without POLICY.
        "# Build-pipeline step 5b (Linux): the per-render memory-growth class.".to_string(),
        // A POLICY mention and a contract step in the next sentence are not a citation.
        format!("# {P} § Build pipeline owns the rule. The contract's step 5c runs it."),
        // A file named for a policy is not the document.
        "//! see src/imagecache/policy.rs; step 2 decodes.".to_string(),
    ];
    for case in &cases {
        let found = policy_step_citations(case);
        assert!(found.is_empty(), "FALSE POSITIVE {found:?}: {case}");
    }
}
