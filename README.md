# collate
Rust collation utilities

Example usage:
```rust
use collate::{Collator, Overlap, OverlapsRange};

let collator = Collator::<i32>::default();
assert_eq!((1..4).overlaps(&(3..5), &collator), Overlap::WideLess);
assert!(OverlapsRange::contains(&(1..=4), &(2..=4), &collator));
```

With the `stream` feature, `try_union(collator, streams)` combines a finite vector
of sorted fallible streams into sorted values unique by collation (not necessarily by value equality). It retains one head per
input, polls inputs in order, and releases them on completion, error, or drop.
The flat traversal yields cooperatively without nesting two-input merges.

Two-input `merge` and `try_merge` retain the left representative when heads
collate equally, consuming one occurrence from each input. Repeated values
within one input are preserved. `diff` and `try_diff` remove one left occurrence
for each matching right occurrence and stop when the left input ends. The
fallible two-input streams emit errors in polling order and allow subsequent
polling to continue. All four APIs retain buffered heads until consumption or
drop; their values need no `Ord` or `Clone`, and collators need no `Clone` or `Sync`.

`MergeQueue` provides a bounded priority queue of ready source indices. `push`
queues a ready source, `peek` identifies the first source, and `pop` dequeues it
before its head is consumed or replaced. Equal heads follow input order.
Operations borrow caller-owned heads and their collator, with logarithmic work
per push/pop. Values need no `Ord`, and the collator needs no cloning or shared
ownership. Consumers retain their own polling, deduplication, and value resolution.

`Collate::cmp` defines collation equivalence independently of `Eq`: equal values
must collate equally, but equal collation need not imply equal values. A complex
norm collator, for example, can group different complex values. Native lookups
must inspect that group for semantic equality; stream union deliberately emits
one representative per collation class.

Each `Collate` implementation defines one comparison. Owners may compose ordinary
comparators with `Ordering::then_with`; complete-key comparisons apply the secondary
order only after all primary columns compare equal. Comparators require no intrinsic
`Ord` implementation. Index owners define which comparator serves each operation.

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
