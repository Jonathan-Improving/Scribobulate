//! The find bar's **match options** — what counts as a match, independent of which
//! pane is being searched.
//!
//! These three live in one struct rather than as three loose booleans because they are
//! consumed by two entirely separate search engines: the editor's
//! `GtkSourceSearchSettings`, which implements them natively, and the preview's own
//! [`super::matcher::Matcher`], which implements them by hand over three different
//! texts (the preview buffer, table-cell labels, and the source of collapsed
//! disclosures). One struct means one thing to pass, one thing to key a cache on, and
//! one thing to persist — three booleans threaded separately through those paths is
//! how two panes come to disagree about what the reader asked for.
//!
//! **"Search in selection" is deliberately NOT here.** It is an option in the same
//! row of the bar, but it is not a property of *matching*: it is a bound on where
//! matching is applied, it is held as a live reference into a buffer, and it does not
//! survive a restart the way these do (the buffer it indexes does not exist yet at
//! restore time). Putting it in this struct would persist a toggle whose bound cannot
//! be persisted, which restores the reader into a scope that is not there.

/// How the query text is interpreted. `Default` is the behaviour the find bar had
/// before any of these existed — a case-insensitive literal substring — so a tab
/// restored from a session file that predates the fields reads as the old behaviour
/// rather than as something the reader never chose.
#[derive(
    Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize, Hash,
)]
#[serde(default)]
pub(crate) struct FindOptions {
    /// `Note` does not match `note`.
    pub case_sensitive: bool,
    /// `note` does not match `notebook` — the match must be bounded by non-word
    /// characters (or the ends of the text) on both sides.
    pub whole_word: bool,
    /// The query is a regular expression rather than a literal.
    pub regex: bool,
}

impl FindOptions {
    /// The name of the `win.` action carrying each option, and the field it drives.
    ///
    /// One table rather than three parallel `match` arms: the find-bar toggle, the Edit
    /// menu item and the session field all have to agree on which action name means
    /// which boolean, and an action name is a string that no compiler checks. Every
    /// surface reads this, so a fourth surface cannot invent a fourth spelling.
    pub(crate) const ACTIONS: [(&'static str, Accessor); 3] = [
        (
            "find-match-case",
            Accessor {
                read: |o| o.case_sensitive,
                write: |o, v| o.case_sensitive = v,
            },
        ),
        (
            "find-whole-word",
            Accessor {
                read: |o| o.whole_word,
                write: |o, v| o.whole_word = v,
            },
        ),
        (
            "find-regex",
            Accessor {
                read: |o| o.regex,
                write: |o, v| o.regex = v,
            },
        ),
    ];
}

/// Read and write one option field, so [`FindOptions::ACTIONS`] can name a field
/// without the call sites knowing which one.
#[derive(Clone, Copy)]
pub(crate) struct Accessor {
    read: fn(&FindOptions) -> bool,
    write: fn(&mut FindOptions, bool),
}

impl Accessor {
    pub(crate) fn get(self, opts: &FindOptions) -> bool {
        (self.read)(opts)
    }
    pub(crate) fn set(self, opts: &mut FindOptions, value: bool) {
        (self.write)(opts, value);
    }
}

#[cfg(test)]
mod tests {
    use super::FindOptions;

    /// The default has to be the pre-existing behaviour, because it is what every tab
    /// restored from a session file written before these fields existed will read as.
    #[test]
    fn the_default_is_a_case_insensitive_literal() {
        let d = FindOptions::default();
        assert!(!d.case_sensitive);
        assert!(!d.whole_word);
        assert!(!d.regex);
    }

    /// Each accessor must reach its own field and no other — a copy-paste in the table
    /// would otherwise wire two toggles to one boolean, which reads as "the button does
    /// nothing" rather than as a mistake.
    #[test]
    fn each_action_accessor_reaches_exactly_its_own_field() {
        for (name, accessor) in FindOptions::ACTIONS {
            let mut opts = FindOptions::default();
            accessor.set(&mut opts, true);
            assert!(
                accessor.get(&opts),
                "{name} did not read back what it wrote"
            );
            let set_count = FindOptions::ACTIONS
                .iter()
                .filter(|(_, other)| other.get(&opts))
                .count();
            assert_eq!(set_count, 1, "{name} also moved another option's field");
        }
    }

    /// The names are what the `win.` actions, the menu model and the toggle buttons all
    /// spell; a duplicate would silently make two toggles one control.
    #[test]
    fn action_names_are_distinct() {
        let mut names: Vec<&str> = FindOptions::ACTIONS.iter().map(|(n, _)| *n).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before, "two options share an action name");
    }
}
