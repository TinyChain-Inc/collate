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
of sorted fallible streams into sorted unique values. It retains one head per
input, polls inputs in order, and releases them on completion, error, or drop.
The flat traversal yields cooperatively without nesting two-input merges.
