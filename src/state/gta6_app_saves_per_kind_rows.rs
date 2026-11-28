//! Which rows of the build the per-kind table takes: a table kind named by its constant, rows
//! spread over their sizes, and the archives holding most rows of a kind.

use std::borrow::Cow;
use std::collections::VecDeque;

use or_io::container::rpf::gta6_kinds::kind_label;

/// A kind a table row states: a 24-byte row's class byte, or a 20-byte row's type code.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Row {
    Class(u8),
    Type(u32),
}

impl Row {
    /// The label the archive gives a row of this kind, which the walk files its rows under.
    pub(super) fn label(self) -> Cow<'static, str> {
        match self {
            Row::Class(class) => kind_label(Some(class), None),
            Row::Type(code) => kind_label(None, Some(code)),
        }
    }
}

/// `want` indices of `n` rows sorted by size: the smallest, the largest, then the middle of each
/// gap in turn, so any first few of them are spread over the sizes.
pub(super) fn spread(n: usize, want: usize) -> Vec<usize> {
    let mut out = Vec::new();
    if n == 0 {
        return out;
    }
    out.push(0);
    if n > 1 {
        out.push(n - 1);
    }
    let mut gaps = VecDeque::from([(0, n - 1)]);
    while out.len() < want.min(n) {
        let Some((lo, hi)) = gaps.pop_front() else {
            break;
        };
        if hi - lo < 2 {
            continue;
        }
        let mid = lo + (hi - lo) / 2;
        out.push(mid);
        gaps.push_back((lo, mid));
        gaps.push_back((mid, hi));
    }
    out.truncate(want);
    out
}

/// The indices of at most `most` of `counts` that are not zero, the largest first; a tie keeps
/// the earlier index first.
pub(super) fn ranked(counts: &[usize], most: usize) -> Vec<usize> {
    let mut held: Vec<usize> = (0..counts.len()).filter(|&i| counts[i] > 0).collect();
    held.sort_by_key(|&i| std::cmp::Reverse(counts[i]));
    held.truncate(most);
    held
}

#[cfg(test)]
mod tests {
    use super::super::{label_done, lines_of, FILTERS, KINDS};
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn rows_spread_over_their_sizes_take_the_ends_first_then_each_middle_and_never_repeat() {
        assert_eq!(spread(10, 3), [0, 9, 4]);
        assert_eq!(spread(10, 5), [0, 9, 4, 2, 6]);
        assert_eq!(spread(1, 3), [0]);
        assert_eq!(spread(2, 1), [0]);
        assert_eq!(spread(0, 3), Vec::<usize>::new());
        assert_eq!(spread(5, 0), Vec::<usize>::new());
        // Asked for more than there are, every row is taken once.
        let mut all = spread(7, 24);
        all.sort_unstable();
        assert_eq!(all, (0..7).collect::<Vec<_>>());
    }

    #[test]
    fn the_archives_ranked_for_a_kind_hold_most_of_its_rows_first_and_none_holds_none() {
        assert_eq!(ranked(&[3, 0, 9, 9, 1], 2), [2, 3]);
        assert_eq!(ranked(&[3, 0, 9, 9, 1], 9), [2, 3, 0, 4]);
        assert_eq!(ranked(&[0, 0], 2), Vec::<usize>::new());
    }

    #[test]
    fn each_line_is_found_in_rows_labelled_by_its_constants_and_filters_also_by_their_head() {
        // A map types row opens as two lines; a frame filter row is found under its table kind and
        // under the label its head gives it.
        assert_eq!(lines_of(&Row::Type(0x4A).label()).len(), 2);
        assert_eq!(lines_of(FILTERS), [FILTERS]);
        assert_eq!(lines_of(&Row::Type(0x56).label()), [FILTERS]);
        assert_eq!(lines_of(&Row::Class(0x69).label()), ["height map"]);
        assert!(lines_of("no such label").is_empty());
        for (line, _, _, rows) in KINDS {
            assert!(!rows.is_empty(), "{line} is found in no row");
        }
        // A label of one kind is done once its lines are; one of several kinds, or read by its
        // head, never is.
        let all: HashSet<&str> = KINDS.iter().map(|l| l.0).collect();
        assert!(label_done(&Row::Type(0x4F).label(), &all));
        assert!(!label_done(&Row::Type(0x4F).label(), &HashSet::new()));
        assert!(!label_done(&Row::Class(0x0B).label(), &all));
        assert!(!label_done(&Row::Type(0x63).label(), &all));
    }
}
