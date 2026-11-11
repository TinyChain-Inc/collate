# collate
Rust collation utilities

Example usage:
```rust
use collate::*;

let collator = Collator::default();
let collection = [
    [1, 2, 3],
    [2, 3, 4],
    [3, 4, 5],
];

assert_eq!(collator.bisect_left(&collection, &[1]), 0);
assert_eq!(collator.bisect_right(&collection, &[1]), 1);
```

With the `stream` feature, `try_union(collator, streams)` combines a finite vector
of sorted fallible streams into sorted values unique by collation (not necessarily by value equality). It retains one head per
input, polls inputs in order, and releases them on completion, error, or drop.
The flat traversal yields cooperatively without nesting two-input merges.

`Collate::cmp` defines collation equivalence independently of `Eq`: equal values
must collate equally, but equal collation need not imply equal values. A complex
norm collator, for example, can group different complex values. Native lookups
must inspect that group for semantic equality; stream union deliberately emits
one representative per collation class.

Range containment includes inner ranges sharing either endpoint. Partial overlaps
remain directional; owned and borrowed bounds use the same comparison.
Empty or reversed ranges intersect nothing (`Overlap::Empty`), including other
empty ranges. Every range contains an empty range; an empty range contains no
nonempty range or value. Combining overlaps with `then` treats `Empty` as the
identity. Difference streams yield cooperatively during long matching runs.
Touching ranges intersect at their shared endpoint only when both include it.
Inclusive standard ranges support the same left- and right-operand comparisons
as the other standard ranges; their containment and empty-range rules agree
with explicit bounds.
