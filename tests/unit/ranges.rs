use std::ops::Bound::{Excluded, Included, Unbounded};

use crate::{Collator, Overlap, OverlapsRange, OverlapsValue};

#[test]
fn inclusive_left_ranges_share_bound_semantics() {
    let c = Collator::default();
    assert_eq!(
        (1..=3).overlaps(&(Included(1), Included(3)), &c),
        Overlap::Equal
    );
    assert_eq!((1..=3).overlaps(&(1..3), &c), Overlap::Wide);
    assert_eq!((..=3).overlaps(&(3..=3), &c), Overlap::Wide);
    assert!(OverlapsRange::contains(&(1..=3), &(3..=3), &c));
    assert!(OverlapsRange::contains(&(..=3), &(3..3), &c));
    assert_eq!((1..=1).overlaps(&(1..=1), &c), Overlap::Equal);
    assert_eq!(
        std::ops::RangeInclusive::new(3, 1).overlaps(&(..=3), &c),
        Overlap::Empty
    );
    assert_eq!((..=3).overlaps(&(3..), &c), Overlap::WideLess);
    assert_eq!((1..=3).overlaps(&(..3), &c), Overlap::WideGreater);
    assert_eq!((3..4).overlaps(&(..=3), &c), Overlap::WideGreater);
}

#[test]
fn empty_ranges_have_containment_without_intersection() {
    let collator = Collator::default();
    let empty = [
        (Included(1), Excluded(1)),
        (Excluded(1), Included(1)),
        (Excluded(1), Excluded(1)),
        (Included(2), Included(0)),
        (Included(2), Excluded(0)),
        (Excluded(2), Included(0)),
        (Excluded(2), Excluded(0)),
    ];
    let nonempty = [
        (Unbounded, Unbounded),
        (Included(0), Included(2)),
        (Included(1), Included(1)),
        (Unbounded, Excluded(0)),
        (Excluded(2), Unbounded),
    ];
    for outer in empty {
        for inner in empty.into_iter().chain(nonempty) {
            assert_eq!(outer.overlaps(&inner, &collator), Overlap::Empty);
            assert_eq!(inner.overlaps(&outer, &collator), Overlap::Empty);
            assert!(!outer.contains_partial(&inner, &collator));
            assert!(!inner.contains_partial(&outer, &collator));
            assert!(OverlapsRange::contains(&inner, &outer, &collator));
            assert_eq!(
                OverlapsRange::contains(&outer, &inner, &collator),
                empty.contains(&inner)
            );
            assert_eq!(
                (&outer.0, &outer.1).overlaps(&(&inner.0, &inner.1), &collator),
                Overlap::Empty
            );
            assert_eq!(
                OverlapsRange::contains(&(&outer.0, &outer.1), &(&inner.0, &inner.1), &collator),
                empty.contains(&inner)
            );
        }
        for value in -1..=3 {
            assert_eq!(outer.overlaps_value(&value, &collator), Overlap::Empty);
            assert!(!outer.contains_value(&value, &collator));
        }
    }
    assert_eq!((1..1).overlaps(&(0..2), &collator), Overlap::Empty);
    let reversed = std::ops::Range { start: 2, end: 1 };
    assert!(OverlapsRange::contains(&(1..1), &reversed, &collator));
    assert_eq!((1..=1).overlaps_value(&1, &collator), Overlap::Equal);
    assert!((1..=1).contains_value(&1, &collator));
    for overlap in [
        Overlap::Empty,
        Overlap::Less,
        Overlap::Greater,
        Overlap::Equal,
        Overlap::Narrow,
        Overlap::Wide,
        Overlap::WideLess,
        Overlap::WideGreater,
    ] {
        assert_eq!(overlap.then(Overlap::Empty), overlap);
        assert_eq!(Overlap::Empty.then(overlap), overlap);
    }
}

#[test]
fn containment_matches_bound_inclusion() {
    let collator = Collator::default();
    let cases = [
        (
            (Included(0), Included(5)),
            (Included(1), Included(5)),
            Overlap::Wide,
        ),
        (
            (Included(0), Included(5)),
            (Included(0), Included(4)),
            Overlap::Wide,
        ),
        (
            (Included(0), Excluded(5)),
            (Included(0), Excluded(4)),
            Overlap::Wide,
        ),
        (
            (Excluded(0), Included(5)),
            (Excluded(0), Included(4)),
            Overlap::Wide,
        ),
        (
            (Included(0), Included(5)),
            (Excluded(0), Included(5)),
            Overlap::Wide,
        ),
        (
            (Included(0), Included(5)),
            (Included(0), Excluded(5)),
            Overlap::Wide,
        ),
        (
            (Unbounded, Included(5)),
            (Included(0), Included(5)),
            Overlap::Wide,
        ),
        (
            (Included(0), Unbounded),
            (Included(0), Included(5)),
            Overlap::Wide,
        ),
        (
            (Unbounded, Unbounded),
            (Unbounded, Included(5)),
            Overlap::Wide,
        ),
        (
            (Included(0), Included(5)),
            (Included(0), Included(5)),
            Overlap::Equal,
        ),
        (
            (Included(0), Included(5)),
            (Included(1), Included(4)),
            Overlap::Wide,
        ),
        (
            (Included(0), Included(5)),
            (Included(-1), Included(6)),
            Overlap::Narrow,
        ),
        (
            (Excluded(0), Included(5)),
            (Included(0), Included(4)),
            Overlap::WideGreater,
        ),
        (
            (Included(0), Excluded(5)),
            (Included(1), Included(5)),
            Overlap::WideLess,
        ),
        (
            (Included(0), Included(2)),
            (Included(1), Included(3)),
            Overlap::WideLess,
        ),
        (
            (Included(1), Included(3)),
            (Included(0), Included(2)),
            Overlap::WideGreater,
        ),
        (
            (Included(0), Included(1)),
            (Included(1), Included(2)),
            Overlap::WideLess,
        ),
        (
            (Included(0), Excluded(1)),
            (Included(1), Included(2)),
            Overlap::Less,
        ),
        (
            (Included(1), Included(2)),
            (Included(0), Excluded(1)),
            Overlap::Greater,
        ),
    ];
    for (outer, inner, expected) in cases {
        assert_eq!(outer.overlaps(&inner, &collator), expected);
        assert_eq!(
            (&outer.0, &outer.1).overlaps(&(&inner.0, &inner.1), &collator),
            expected
        );
        assert_eq!(
            OverlapsRange::contains(&outer, &inner, &collator),
            matches!(expected, Overlap::Wide | Overlap::Equal)
        );
        assert_eq!(
            outer.contains_partial(&inner, &collator),
            !matches!(expected, Overlap::Less | Overlap::Greater)
        );
    }
    assert!(OverlapsRange::contains(&(0..5), &(1..5), &collator));
    assert!(OverlapsRange::contains(&(0..5), &(0..4), &collator));
    assert!(OverlapsRange::contains(
        &(Unbounded, Included(5)),
        &(0..=5),
        &collator
    ));
    assert!(!OverlapsRange::contains(&(0..5), &(1..6), &collator));
}

#[test]
fn endpoint_relations_match_interval_membership() {
    let bounds: Vec<_> = std::iter::once(Unbounded)
        .chain((-2..=2).flat_map(|n| [Included(n), Excluded(n)]))
        .collect();
    let ranges: Vec<_> = bounds
        .iter()
        .flat_map(|lo| bounds.iter().map(move |hi| (*lo, *hi)))
        .collect();
    let collator = Collator::default();
    let includes = |bounds: &(std::ops::Bound<i32>, std::ops::Bound<i32>), doubled: i32| {
        let lower = match bounds.0 {
            Unbounded => true,
            Included(n) => doubled >= 2 * n,
            Excluded(n) => doubled > 2 * n,
        };
        let upper = match bounds.1 {
            Unbounded => true,
            Included(n) => doubled <= 2 * n,
            Excluded(n) => doubled < 2 * n,
        };
        lower && upper
    };
    for left in &ranges {
        for right in &ranges {
            // Half-steps distinguish open endpoints without assuming a discrete
            // collator domain; outer points also represent unbounded tails.
            let intersects = (-6..=6).any(|n| includes(left, n) && includes(right, n));
            let contains = (-6..=6).all(|n| !includes(right, n) || includes(left, n));
            assert_eq!(
                left.contains_partial(right, &collator),
                intersects,
                "{left:?} {right:?}"
            );
            assert_eq!(
                OverlapsRange::contains(left, right, &collator),
                contains,
                "{left:?} {right:?}"
            );
            assert_eq!(
                (&left.0, &left.1).contains_partial(&(&right.0, &right.1), &collator),
                intersects
            );
            assert_eq!(
                OverlapsRange::contains(&(&left.0, &left.1), &(&right.0, &right.1), &collator),
                contains
            );
            assert_eq!(right.contains_partial(left, &collator), intersects);
        }
    }
}
