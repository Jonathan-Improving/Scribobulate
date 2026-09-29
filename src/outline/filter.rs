//! The filtered outline: which headings a sidebar filter keeps, and in what role
//! (TDD 12.26).
//!
//! A heading that matches the query is kept as a **match**, with the byte ranges of its
//! title to highlight. Each match's ancestors are kept too, as **context**, so a match
//! stays under its section path and "Base" still says which "Base" it is; a context row
//! is shown dimmed and never highlighted. A match's own sub-headings are dropped unless
//! they match themselves — showing them would make any broad query long again.
//!
//! Computed from the flat heading list rather than over the view's flattened tree model,
//! which only holds rows under EXPANDED nodes and so could not see a match inside a
//! collapsed section. Kept nodes carry their original `doc_index`, so every index-keyed
//! reader — the scroll-spy's ancestor walk, a row's durable path, the preview's heading
//! sites — reads a filtered row exactly as it reads an unfiltered one.

use super::tree::{ancestor_chain, build_tree, HeadingNode};
use super::Heading;
use crate::sidebarfilter::Query;
use std::collections::BTreeMap;
use std::ops::Range;

/// Why a filtered outline shows a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RowMark {
    /// The heading matches; these byte ranges of its title are highlighted.
    Match(Vec<Range<usize>>),
    /// The heading does not match; it is shown dimmed as the path to a match below it.
    Context,
}

/// The outline a filter produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FilteredOutline {
    /// The kept headings as a forest — every match plus its ancestors, nothing else.
    pub(crate) roots: Vec<HeadingNode>,
    /// Every kept heading's role, keyed by `doc_index`.
    pub(crate) marks: BTreeMap<usize, RowMark>,
    /// How many headings match.
    pub(crate) matched: usize,
    /// How many headings the document has.
    pub(crate) total: usize,
}

/// Filter `headings` by `query`.
pub(crate) fn filter_outline(headings: &[Heading], query: &Query) -> FilteredOutline {
    let levels: Vec<u8> = headings.iter().map(|h| h.level).collect();
    let mut marks: BTreeMap<usize, RowMark> = BTreeMap::new();
    let mut matched = 0;
    for (doc_index, heading) in headings.iter().enumerate() {
        let Some(ranges) = query.find(&heading.text) else {
            continue;
        };
        matched += 1;
        // The chain is root → this heading inclusive; everything before the last is an
        // ancestor. Ancestors precede their descendants in document order, so an ancestor
        // that matches in its own right is already marked Match by the time its
        // descendant is visited, and `or_insert` leaves that mark alone.
        let chain = ancestor_chain(&levels, doc_index);
        for &ancestor in chain.iter().take(chain.len().saturating_sub(1)) {
            marks.entry(ancestor).or_insert(RowMark::Context);
        }
        marks.insert(doc_index, RowMark::Match(ranges));
    }
    let roots = prune(build_tree(headings), &marks);
    FilteredOutline {
        roots,
        marks,
        matched,
        total: headings.len(),
    }
}

/// Keep only the nodes named in `marks`. The kept set is closed under ancestry (every
/// match brings its whole chain), so a dropped node never has a kept descendant and one
/// recursive filter is the whole pruning.
fn prune(nodes: Vec<HeadingNode>, marks: &BTreeMap<usize, RowMark>) -> Vec<HeadingNode> {
    nodes
        .into_iter()
        .filter(|n| marks.contains_key(&n.doc_index))
        .map(|mut n| {
            n.children = prune(std::mem::take(&mut n.children), marks);
            n
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One highlight span, spelled so clippy does not read it as a mistaken range-vec.
    fn one(r: std::ops::Range<usize>) -> Vec<std::ops::Range<usize>> {
        vec![r]
    }
    use crate::span::OriginalByteOffset;

    fn h(level: u8, text: &str) -> Heading {
        Heading {
            level,
            text: text.into(),
            src_offset: OriginalByteOffset::new(0),
        }
    }

    fn q(text: &str) -> Query {
        Query::parse(text).expect("a query")
    }

    /// `# Guide` > `## Install` > `### Base setup`, `### Extras`; `## Usage` > `### Base
    /// commands`; `# Appendix`.
    fn doc() -> Vec<Heading> {
        vec![
            h(1, "Guide"),
            h(2, "Install"),
            h(3, "Base setup"),
            h(3, "Extras"),
            h(2, "Usage"),
            h(3, "Base commands"),
            h(1, "Appendix"),
        ]
    }

    /// A forest's shape as `doc_index`es, three levels deep: (root, [(child, [grandchild])]).
    type Shape = Vec<(usize, Vec<(usize, Vec<usize>)>)>;

    fn shape(nodes: &[HeadingNode]) -> Shape {
        nodes
            .iter()
            .map(|n| {
                (
                    n.doc_index,
                    n.children
                        .iter()
                        .map(|c| {
                            (
                                c.doc_index,
                                c.children.iter().map(|g| g.doc_index).collect(),
                            )
                        })
                        .collect(),
                )
            })
            .collect()
    }

    #[test]
    fn matches_keep_their_ancestors_as_context_and_nothing_else() {
        let f = filter_outline(&doc(), &q("base"));
        assert_eq!(f.matched, 2);
        assert_eq!(f.total, 7);
        // Guide > {Install > Base setup, Usage > Base commands}; Extras and Appendix gone.
        assert_eq!(shape(&f.roots), vec![(0, vec![(1, vec![2]), (4, vec![5])])]);
        assert_eq!(f.marks.get(&0), Some(&RowMark::Context));
        assert_eq!(f.marks.get(&1), Some(&RowMark::Context));
        assert_eq!(f.marks.get(&2), Some(&RowMark::Match(one(0..4))));
        assert!(!f.marks.contains_key(&3));
        assert!(!f.marks.contains_key(&6));
    }

    #[test]
    fn a_matching_headings_own_sub_headings_are_hidden_unless_they_match() {
        let f = filter_outline(&doc(), &q("install"));
        assert_eq!(shape(&f.roots), vec![(0, vec![(1, vec![])])]);
    }

    #[test]
    fn a_matching_ancestor_is_a_match_not_context() {
        let f = filter_outline(&doc(), &q("i"));
        assert!(matches!(f.marks.get(&0), Some(RowMark::Match(_))), "Guide");
        assert!(
            matches!(f.marks.get(&1), Some(RowMark::Match(_))),
            "Install"
        );
        assert!(
            matches!(f.marks.get(&6), Some(RowMark::Match(_))),
            "Appendix"
        );
    }

    #[test]
    fn no_match_is_an_empty_outline_that_still_knows_its_total() {
        let f = filter_outline(&doc(), &q("zebra"));
        assert!(f.roots.is_empty());
        assert_eq!((f.matched, f.total), (0, 7));
    }

    #[test]
    fn a_match_under_a_level_skip_keeps_the_skipped_parent() {
        let flat = vec![h(1, "Top"), h(3, "Deep base")];
        let f = filter_outline(&flat, &q("base"));
        assert_eq!(shape(&f.roots), vec![(0, vec![(1, vec![])])]);
        assert_eq!(f.marks.get(&0), Some(&RowMark::Context));
    }
}
