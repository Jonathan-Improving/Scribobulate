//! The outline sidebar *widget* — a collapsible heading tree.
//!
//! Builds a virtualized `GtkListView` over a `GtkTreeListModel` whose rows are
//! `GtkTreeExpander`s (disclosure chevrons + depth indentation for free), driven
//! by a small `HeadingObject` GObject wrapping each `outline::HeadingNode`.
//! Activating a row calls back with two things, and the pairing is deliberate: the
//! heading's **path** — its durable identity (`outline::expansion::HeadingPath`), which
//! `window/outline_nav.rs` re-finds in the document as it stands at the click — and the
//! row's own `doc_index`, which names its place in THIS build and is only ever applied
//! to the render of the same generation. The row used to hand over a source byte offset
//! baked in at build time, which is stale for the length of the live-edit debounce: the
//! reader types a paragraph, clicks a heading within 300 ms, and lands where that
//! heading used to be.
//!
//! This module is GTK-construction code (custom GObject + signal factory), not
//! headlessly testable, so it sits outside the unit-test coverage gate alongside
//! `codeview.rs` — the pure model and tree-folding it consumes live in
//! `outline/`, where they are unit-tested.

use crate::outline::expansion::HeadingPath;
use crate::outline::filter::RowMark;
use crate::outline::HeadingNode;
use gtk::prelude::*;
use gtk::{gio, glib, pango};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

mod imp {
    use super::*;
    use glib::subclass::prelude::*;
    use std::cell::{Cell, RefCell};

    pub(crate) struct HeadingObject {
        pub(crate) level: Cell<u8>,
        /// This row's place in the build that produced it — a transient handle, valid
        /// for as long as this widget tree is, which is what the tree model and the
        /// scroll-spy index by. Never carried out of the build: see `path`.
        pub(crate) doc_index: Cell<usize>,
        /// The heading's DURABLE identity — the title path from a root heading down to
        /// it. What an activation carries, because it is the one name that still means
        /// the same heading after the document has been edited (`outline::expansion`).
        pub(crate) path: RefCell<HeadingPath>,
        pub(crate) title: RefCell<String>,
        /// The title as Pango markup with a sidebar filter's matches highlighted, or
        /// `None` when the row shows its title plainly (no filter, or a context row).
        pub(crate) markup: RefCell<Option<String>>,
        /// Whether a sidebar filter keeps this row only as the path to a match below it
        /// — shown dimmed and never highlighted (TDD 12.26).
        pub(crate) context: Cell<bool>,
        /// Child headings nested under this one (empty for a leaf).
        pub(crate) children: gio::ListStore,
    }

    impl Default for HeadingObject {
        fn default() -> Self {
            Self {
                level: Cell::new(1),
                doc_index: Cell::new(0),
                path: RefCell::new(HeadingPath::new()),
                title: RefCell::new(String::new()),
                markup: RefCell::new(None),
                context: Cell::new(false),
                children: gio::ListStore::new::<super::HeadingObject>(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for HeadingObject {
        const NAME: &'static str = "ScribHeadingObject";
        type Type = super::HeadingObject;
        type ParentType = glib::Object;
    }

    impl ObjectImpl for HeadingObject {}
}

glib::wrapper! {
    pub(crate) struct HeadingObject(ObjectSubclass<imp::HeadingObject>);
}

impl HeadingObject {
    /// Recursively wrap a `HeadingNode` (and its subtree) as GObjects.
    ///
    /// `paths` is this build's `doc_index` → path map, handed in rather than re-derived
    /// here: the rule for naming a heading (including how same-titled siblings are told
    /// apart) belongs to `outline::expansion` alone, and a second implementation of it
    /// would name the same heading two ways.
    ///
    /// `marks` is a sidebar filter's verdict per `doc_index` (`outline::filter`), or
    /// `None` for the unfiltered outline.
    fn new(
        node: &HeadingNode,
        paths: &[HeadingPath],
        marks: Option<&BTreeMap<usize, RowMark>>,
    ) -> Self {
        use glib::subclass::prelude::*;
        let obj: Self = glib::Object::new();
        let imp = obj.imp();
        imp.level.set(node.level);
        imp.doc_index.set(node.doc_index);
        // Every node has a path: `paths` is built from the same tree. A row without one
        // would carry an empty identity that matches nothing.
        debug_assert!(
            paths.get(node.doc_index).is_some(),
            "outline node {} has no identity path",
            node.doc_index
        );
        if let Some(path) = paths.get(node.doc_index) {
            *imp.path.borrow_mut() = path.clone();
        }
        *imp.title.borrow_mut() = node.text.clone();
        match marks.and_then(|m| m.get(&node.doc_index)) {
            Some(RowMark::Match(ranges)) => {
                *imp.markup.borrow_mut() =
                    Some(crate::sidebarfilter::highlight_markup(&node.text, ranges));
            }
            Some(RowMark::Context) => imp.context.set(true),
            None => {}
        }
        for child in &node.children {
            imp.children
                .append(&HeadingObject::new(child, paths, marks));
        }
        obj
    }

    fn title(&self) -> String {
        use glib::subclass::prelude::*;
        self.imp().title.borrow().clone()
    }
    fn level(&self) -> u8 {
        use glib::subclass::prelude::*;
        self.imp().level.get()
    }
    pub(crate) fn doc_index(&self) -> usize {
        use glib::subclass::prelude::*;
        self.imp().doc_index.get()
    }
    /// The heading's durable identity — see the field.
    pub(crate) fn path(&self) -> HeadingPath {
        use glib::subclass::prelude::*;
        self.imp().path.borrow().clone()
    }
    /// The title as highlighted markup, when a sidebar filter matched it.
    pub(crate) fn markup(&self) -> Option<String> {
        use glib::subclass::prelude::*;
        self.imp().markup.borrow().clone()
    }
    /// Whether a sidebar filter shows this row only as context (see the field).
    pub(crate) fn is_context(&self) -> bool {
        use glib::subclass::prelude::*;
        self.imp().context.get()
    }
    fn children(&self) -> gio::ListStore {
        use glib::subclass::prelude::*;
        self.imp().children.clone()
    }
}

/// CSS class applied to each row label so headings read hierarchically (H1
/// boldest, deeper levels lighter/smaller). Defined in `preview::css`.
///
/// h6-and-deeper fold onto `outline-h5`, the deepest tier — matching the preview's
/// h6→h5 fold, so no surface distinguishes h6 from h5 (TDD 2.1a).
fn level_class(level: u8) -> &'static str {
    const CLASSES: [&str; crate::theme::HEADING_LEVELS] = [
        "outline-h1",
        "outline-h2",
        "outline-h3",
        "outline-h4",
        "outline-h5",
    ];
    CLASSES[crate::theme::heading_slot(level)]
}

/// Expand every heading row of a `GtkTreeListModel` that was built
/// `autoexpand=false` (GTK4Rs/AP-111 — required so a later true-recursive Collapse all is
/// possible). The single, subtle forward walk shared by the build-time full-open
/// (`build_outline_content`) and the runtime "Expand all" button
/// (`window::outline_nav::outline_expand_all`) — kept in one place so the two
/// can't drift (QA M-3), since the walk is load-bearing and must stay identical.
///
/// A `GtkTreeListRow::set_expanded(true)` that actually expands inserts that row's
/// DIRECT children contiguously right after it, synchronously
/// (`gtk_tree_list_row_set_expanded` → `expand_node` → `items_changed(pos+1, 0, n)`,
/// source-confirmed `gtktreelistmodel.c:1216`). Because the model is
/// `autoexpand=false`, each step opens exactly one level; re-reading `n_items()`
/// each step and advancing the index one at a time therefore descends into each
/// freshly-revealed child in turn, reaching full depth one level per step.
/// Already-expanded rows are a cheap no-op. See GTK4Rs/AP-111.
pub(crate) fn expand_all_rows(model: &gtk::TreeListModel) {
    expand_rows_except(model, &BTreeSet::new());
}

/// The same forward walk, leaving the headings in `keep_collapsed` shut.
///
/// **Descending past a collapsed row still works**, which is the property that makes one
/// walk serve both cases: a row left collapsed simply never inserts its children, so the
/// walk continues with whatever the model does hold, and everything below that node keeps
/// the default. That is also why the remembered state cannot describe a node underneath a
/// collapsed one — GTK frees that subtree (GTK4Rs/AP-111), so there is nothing there to be in
/// any state. Re-opening the parent reveals its children collapsed, which is the behaviour
/// TDD 12.17 already pins.
///
/// `keep_collapsed` holds `doc_index`es, resolved from durable title paths by
/// `outline::expansion` before the build — see that module for why an index is only ever a
/// within-one-build handle.
pub(crate) fn expand_rows_except(model: &gtk::TreeListModel, keep_collapsed: &BTreeSet<usize>) {
    let mut i = 0;
    while i < model.n_items() {
        if let Some(row) = model.item(i).and_downcast::<gtk::TreeListRow>() {
            let collapse = row
                .item()
                .and_downcast::<HeadingObject>()
                .is_some_and(|h| keep_collapsed.contains(&h.doc_index()));
            if !collapse {
                row.set_expanded(true);
            }
        }
        i += 1;
    }
}

/// Every currently-materialised row's `(doc_index, expanded)`, in model order.
///
/// The read side of expansion persistence: taken while the reader's outline is still on
/// screen, because after a rebuild there is nothing left to ask (GTK4Rs/AP-111). Rows under a
/// collapsed node are absent by construction — GTK freed them — and their absence is
/// correct, not a gap: nothing under a closed node has a state to report.
///
/// Only rows that CAN be expanded are reported. A leaf heading is neither open nor shut, and
/// reporting it as "not expanded" would record every leaf as collapsed.
pub(crate) fn row_expansion_states(model: &gtk::TreeListModel) -> Vec<(usize, bool)> {
    let mut out = Vec::new();
    for i in 0..model.n_items() {
        let Some(row) = model.item(i).and_downcast::<gtk::TreeListRow>() else {
            continue;
        };
        if !row.is_expandable() {
            continue;
        }
        if let Some(heading) = row.item().and_downcast::<HeadingObject>() {
            out.push((heading.doc_index(), row.is_expanded()));
        }
    }
    out
}

/// Build the outline content widget for `roots`.
///
/// Returns a `GtkListView` (collapsible heading tree) when there are headings,
/// or a muted "No headings" placeholder otherwise — never an error. `on_activate`
/// receives the activated heading's `(doc_index, src_offset)`.
///
/// `initial_selected`, when `Some(doc_index)`, pre-selects the row for that heading
/// *before* the selection-change navigation handler is connected — so restoring the
/// selection across a rebuild does not re-fire a scroll.
///
/// `keep_collapsed` names the headings to leave shut, so a rebuild restores what the reader
/// had folded rather than springing the whole tree open (TDD 12.24).
///
/// `marks` is a sidebar filter's verdict per heading (`outline::filter`) when `roots` is
/// a filtered outline — its matches are highlighted and its context rows dimmed — and
/// `empty_label` is what an empty outline says: "No headings", or "No matching headings"
/// under a filter that matched nothing (TDD 12.26).
pub(crate) fn build_outline_content(
    roots: &[HeadingNode],
    paths: &[HeadingPath],
    on_activate: Rc<dyn Fn(HeadingPath, usize)>,
    initial_selected: Option<usize>,
    keep_collapsed: &BTreeSet<usize>,
    marks: Option<&BTreeMap<usize, RowMark>>,
    empty_label: &str,
) -> gtk::Widget {
    if roots.is_empty() {
        return empty_placeholder(empty_label);
    }
    let (tree_model, selection) = heading_tree(roots, paths, keep_collapsed, marks);

    // Re-select the previously activated heading BEFORE wiring the navigation
    // handler below, so restoring the selection across a rebuild doesn't trigger a
    // spurious scroll. The build-time expand-all pass in `heading_tree` materialised a row for
    // every heading in the flattened TreeListModel, so a linear scan finds it by
    // doc_index. (This holds only at build time — after a user Collapse all,
    // collapsed headings have no row; the scroll-spy handles that miss gracefully.)
    if let Some(target) = initial_selected {
        let n = tree_model.n_items();
        for i in 0..n {
            let is_match = tree_model
                .item(i)
                .and_downcast::<gtk::TreeListRow>()
                .and_then(|row| row.item())
                .and_downcast::<HeadingObject>()
                .is_some_and(|h| h.doc_index() == target);
            if is_match {
                selection.set_selected(i);
                break;
            }
        }
    }

    // Navigate on selection change rather than on `activate`: a single click (and
    // arrow-key movement) changes the SingleSelection, so this gives single-click
    // navigation AND keyboard navigation for free — `activate` would require a
    // double-click / Enter. selected_item() is the GtkTreeListRow (passthrough=false);
    // the heading is one hop in via row.item(). Fires with None when a rebuild
    // clears the selection — guarded.
    selection.connect_selected_item_notify(move |sel| {
        let Some(item) = sel.selected_item() else {
            return;
        };
        let Some(row) = item.downcast_ref::<gtk::TreeListRow>() else {
            return;
        };
        if let Some(heading) = row.item().and_downcast::<HeadingObject>() {
            on_activate(heading.path(), heading.doc_index());
        }
    });

    heading_list_view(selection).upcast()
}

/// The muted placeholder an empty outline shows in place of the tree.
fn empty_placeholder(empty_label: &str) -> gtk::Widget {
    let placeholder = gtk::Label::builder()
        .label(empty_label)
        .xalign(0.0)
        .margin_start(12)
        .margin_top(12)
        .valign(gtk::Align::Start)
        .build();
    placeholder.add_css_class("dim-label");
    placeholder.upcast()
}

/// The heading tree model and its selection, opened to the default depth — the half of
/// the outline both the sidebar and Insert Link's Section picker build identically.
fn heading_tree(
    roots: &[HeadingNode],
    paths: &[HeadingPath],
    keep_collapsed: &BTreeSet<usize>,
    marks: Option<&BTreeMap<usize, RowMark>>,
) -> (gtk::TreeListModel, gtk::SingleSelection) {
    let root_store = gio::ListStore::new::<HeadingObject>();
    for r in roots {
        root_store.append(&HeadingObject::new(r, paths, marks));
    }

    // passthrough=false ⇒ rows are GtkTreeListRow (required to drive a
    // TreeExpander; get the real data via row.item()).
    //
    // autoexpand=FALSE — deliberately, so that JetBrains-style *true recursive*
    // Collapse all is possible (TDD 12.17, GTK4Rs/AP-111). Under autoexpand=true a
    // programmatic `set_expanded(true)` on any row recursively re-opens its WHOLE
    // subtree synchronously (init_node → expand_node per child → recursion), so
    // "Collapse all, then expand one root to see its children collapsed" is
    // impossible — expanding a root would spring the entire subtree open again.
    // With autoexpand=false, collapsing a node DESTROYS its descendant subtree
    // (collapse_node → clear_node_children frees node->model + the child rbtree,
    // source-confirmed `gtktreelistmodel.c`), and re-expanding recreates only its
    // direct children, collapsed. The cost is that the outline no longer opens
    // fully on build for free — so we run an explicit expand-all pass just below.
    let tree_model = gtk::TreeListModel::new(root_store, false, false, |item| {
        let heading = item.downcast_ref::<HeadingObject>()?;
        let children = heading.children();
        if children.n_items() > 0 {
            Some(children.upcast())
        } else {
            None
        }
    });

    // Default outline is fully open (TDD 12.17). autoexpand=false builds only the
    // root rows, so open every level explicitly here — the same forward walk the
    // runtime "Expand all" button uses, shared as `expand_rows_except` (QA M-3).
    //
    // `keep_collapsed` re-applies what the reader had shut before this rebuild (TDD 12.24).
    // Empty on a first build and for any heading nobody has touched, so the default is
    // still "fully open" and this is the same call it always was.
    expand_rows_except(&tree_model, keep_collapsed);

    // `autoselect` BEFORE `model`, and the order is load-bearing: GObject applies
    // construct properties in the order given, and setting the model while `autoselect`
    // is still its default TRUE selects row 0 — the first heading shown as already chosen
    // before anyone chose it (GTK4Rs/AP-267). The sidebar's scroll-spy overwrites that
    // straight away; the Section picker has no spy, so there it would stay.
    let selection = gtk::SingleSelection::builder()
        .autoselect(false)
        .can_unselect(true)
        .model(&tree_model)
        .build();
    (tree_model, selection)
}

/// The list view over `selection`: one expander row per heading, styled by level and by
/// a sidebar filter's verdict.
fn heading_list_view(selection: gtk::SingleSelection) -> gtk::ListView {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let expander = gtk::TreeExpander::new();
        let label = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(pango::EllipsizeMode::End)
            .build();
        expander.set_child(Some(&label));
        item.set_child(Some(&expander));
    });
    factory.connect_bind(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        // GOTCHA: with passthrough=false, item.item() is a GtkTreeListRow, not
        // the HeadingObject — the data is one hop further in (row.item()).
        let Some(row) = item.item().and_downcast::<gtk::TreeListRow>() else {
            return;
        };
        let Some(expander) = item.child().and_downcast::<gtk::TreeExpander>() else {
            return;
        };
        let Some(label) = expander.child().and_downcast::<gtk::Label>() else {
            return;
        };
        // set_list_row gives the disclosure triangle + depth indentation for free.
        expander.set_list_row(Some(&row));
        if let Some(heading) = row.item().and_downcast::<HeadingObject>() {
            // `set_text` and `set_markup` each set `use-markup` themselves, so a recycled
            // label never carries the previous row's mode over.
            match heading.markup() {
                Some(markup) => label.set_markup(&markup),
                None => label.set_text(&heading.title()),
            }
            let level = level_class(heading.level());
            if heading.is_context() {
                label.set_css_classes(&[level, "dim-label"]);
            } else {
                label.set_css_classes(&[level]);
            }
        }
    });
    let list_view = gtk::ListView::new(Some(selection), Some(factory));
    list_view.add_css_class("outline");
    list_view
}

/// Build a heading PICKER over `roots` — the outline sidebar's tree, rows and filter
/// styling, for choosing a heading rather than navigating to it (Insert Link's Section
/// button). `on_pick` receives the chosen heading's `doc_index` in this build.
///
/// Picks on ACTIVATION, not on selection change as the sidebar navigates: a selection
/// change also fires on every arrow key, and a picker that commits on the first arrow
/// press closes before the reader reaches the heading they wanted. Single-click
/// activation keeps the sidebar's one-click feel for the mouse; Enter picks from the
/// keyboard.
pub(crate) fn build_heading_picker(
    roots: &[HeadingNode],
    paths: &[HeadingPath],
    marks: Option<&BTreeMap<usize, RowMark>>,
    empty_label: &str,
    on_pick: Rc<dyn Fn(usize)>,
) -> gtk::Widget {
    if roots.is_empty() {
        return empty_placeholder(empty_label);
    }
    let (tree_model, selection) = heading_tree(roots, paths, &BTreeSet::new(), marks);
    let list_view = heading_list_view(selection);
    list_view.set_single_click_activate(true);
    list_view.connect_activate(move |_, position| {
        let heading = tree_model
            .item(position)
            .and_downcast::<gtk::TreeListRow>()
            .and_then(|row| row.item())
            .and_downcast::<HeadingObject>();
        if let Some(heading) = heading {
            on_pick(heading.doc_index());
        }
    });
    list_view.upcast()
}

#[cfg(all(test, feature = "gtk-integration-tests"))]
mod gtk_tests {
    use super::*;
    use crate::outline::{build_tree, extract_headings};

    fn selection_of(content: &gtk::Widget) -> gtk::SingleSelection {
        content
            .downcast_ref::<gtk::ListView>()
            .and_then(|v| v.model())
            .and_then(|m| m.downcast::<gtk::SingleSelection>().ok())
            .expect("the outline is a single-selection list view")
    }

    fn outline_of(md: &str) -> (Vec<HeadingNode>, Vec<HeadingPath>) {
        let roots = build_tree(&extract_headings(md));
        let paths = crate::outline::expansion::paths_in_document_order(&roots);
        (roots, paths)
    }

    /// Neither the sidebar nor the Section picker opens with a row highlighted.
    ///
    /// `GtkSingleSelection` selects item 0 when a model is set while `autoselect` is still
    /// TRUE (GTK4Rs/AP-267). The sidebar hid that — its scroll-spy selects the section in
    /// view straight after every build — but the picker has no spy, so it opened with the
    /// first heading looking already chosen. A selection must only ever be one somebody
    /// asked for. Mutation check (run): put `.model(…)` back before `.autoselect(false)`
    /// in `heading_tree` and this fails.
    #[gtktest::test]
    fn a_freshly_built_outline_selects_nothing() {
        let (roots, paths) = outline_of("# One\n\n## Two\n\n# Three\n");
        let sidebar = build_outline_content(
            &roots,
            &paths,
            Rc::new(|_, _| {}),
            None,
            &BTreeSet::new(),
            None,
            "No headings",
        );
        let picker = build_heading_picker(&roots, &paths, None, "No headings", Rc::new(|_| {}));
        for (name, content) in [("sidebar", sidebar), ("picker", picker)] {
            assert_eq!(
                selection_of(&content).selected(),
                gtk::INVALID_LIST_POSITION,
                "the {name} must open with nothing selected"
            );
        }
    }

    /// …and a restored selection still lands on the heading it names — the companion that
    /// stops "nothing selected" being satisfied by a build that ignores restores.
    #[gtktest::test]
    fn a_restored_outline_selection_lands_on_its_heading() {
        let (roots, paths) = outline_of("# One\n\n## Two\n\n# Three\n");
        let sidebar = build_outline_content(
            &roots,
            &paths,
            Rc::new(|_, _| {}),
            Some(2),
            &BTreeSet::new(),
            None,
            "No headings",
        );
        let selected = selection_of(&sidebar)
            .selected_item()
            .and_downcast::<gtk::TreeListRow>()
            .and_then(|row| row.item())
            .and_downcast::<HeadingObject>()
            .map(|h| h.doc_index());
        assert_eq!(selected, Some(2));
    }

    /// Activating a picker row hands back that row's heading — here the nested one, so a
    /// position-for-index mix-up (row 1 is doc_index 1 only while everything is open and
    /// in order) would still be caught by the filtered case below.
    #[gtktest::test]
    fn activating_a_picker_row_picks_its_heading() {
        let headings = extract_headings("# One\n\n## Two\n\n# Three\n");
        let paths = crate::outline::expansion::paths_in_document_order(&build_tree(&headings));
        let query = crate::sidebarfilter::Query::parse("three").expect("a query");
        let filtered = crate::outline::filter::filter_outline(&headings, &query);
        let picked = Rc::new(std::cell::Cell::new(None));
        let sink = picked.clone();
        let picker = build_heading_picker(
            &filtered.roots,
            &paths,
            Some(&filtered.marks),
            "No matching headings",
            Rc::new(move |i| sink.set(Some(i))),
        );
        let view = picker.downcast_ref::<gtk::ListView>().expect("a list view");
        view.emit_by_name::<()>("activate", &[&0u32]);
        assert_eq!(
            picked.get(),
            Some(2),
            "the only row shown is heading 2, 'Three'"
        );
    }
}
