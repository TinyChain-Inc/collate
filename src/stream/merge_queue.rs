use std::cmp::Ordering;

use crate::CollateRef;

/// A bounded priority queue of source indices for merging sorted inputs.
///
/// Each source has at most one queued head. Values remain with the caller;
/// operations borrow their heads and collator without requiring `Ord`, cloning,
/// or shared ownership. Equal heads are dequeued in source-index order.
///
/// Source indices must be less than the construction-time input count. Queued
/// heads must remain available and unchanged until popped, and the collation
/// must remain consistent. The queue never polls inputs or resolves equal values.
pub struct MergeQueue {
    inputs: usize,
    tree: Vec<Option<usize>>,
}

impl MergeQueue {
    /// Construct an empty queue with one slot per input source.
    pub fn new(inputs: usize) -> Self {
        Self {
            inputs,
            tree: vec![None; inputs.max(1).next_power_of_two() * 2],
        }
    }

    /// Return the index of the source whose queued head comes first.
    pub fn peek(&self) -> Option<usize> {
        self.tree[1]
    }

    /// Queue a source after supplying its ready head.
    ///
    /// Calling this again for an already queued source leaves one entry for it.
    pub fn push<'a, T: 'a, C: CollateRef<T>>(
        &mut self,
        input: usize,
        head: impl Fn(usize) -> &'a T,
        collator: &C,
    ) {
        assert!(input < self.inputs, "source outside merge queue");
        self.set(input, Some(input), head, collator);
    }

    /// Dequeue the first source before consuming or replacing its head.
    pub fn pop<'a, T: 'a, C: CollateRef<T>>(
        &mut self,
        head: impl Fn(usize) -> &'a T,
        collator: &C,
    ) -> Option<usize> {
        let input = self.peek()?;
        self.set(input, None, head, collator);
        Some(input)
    }

    // Cache the earliest source in each subtree. Only the changed source's
    // root path needs recomputing, keeping each push/pop logarithmic in inputs.
    fn set<'a, T: 'a, C: CollateRef<T>>(
        &mut self,
        input: usize,
        value: Option<usize>,
        head: impl Fn(usize) -> &'a T,
        collator: &C,
    ) {
        let mut node = self.tree.len() / 2 + input;
        self.tree[node] = value;
        while node > 1 {
            node /= 2;
            self.tree[node] = match (self.tree[node * 2], self.tree[node * 2 + 1]) {
                (Some(left), Some(right)) => {
                    let order = collator
                        .cmp_ref(head(left), head(right))
                        .then_with(|| left.cmp(&right));

                    Some(if order == Ordering::Greater {
                        right
                    } else {
                        left
                    })
                }
                (left, right) => left.or(right),
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use super::MergeQueue;
    use crate::Collate;

    struct Item(i32);

    #[derive(Eq, PartialEq)]
    struct ByKey(bool);

    impl Collate for ByKey {
        type Value = Item;

        fn cmp(&self, left: &Item, right: &Item) -> Ordering {
            let order = left.0.cmp(&right.0);
            if self.0 {
                order.reverse()
            } else {
                order
            }
        }
    }

    #[test]
    fn queued_non_ord_heads_follow_collation_and_source_order() {
        for inputs in [0, 1, 4, 7] {
            for reverse in [false, true] {
                let collator = ByKey(reverse);
                let mut heads: Vec<_> = (0..inputs).map(|i| Some(Item((i % 3) as i32))).collect();
                let mut queue = MergeQueue::new(inputs);
                let mut expected: Vec<_> = (0..inputs).collect();
                expected.sort_by(|&left, &right| {
                    collator
                        .cmp(
                            heads[left].as_ref().unwrap(),
                            heads[right].as_ref().unwrap(),
                        )
                        .then_with(|| left.cmp(&right))
                });

                for input in (0..inputs).rev() {
                    queue.push(input, |i| heads[i].as_ref().unwrap(), &collator);
                }

                for input in expected {
                    assert_eq!(queue.peek(), Some(input));
                    assert_eq!(
                        queue.pop(|i| heads[i].as_ref().unwrap(), &collator),
                        Some(input)
                    );

                    heads[input] = None;
                }

                assert_eq!(queue.peek(), None);
                assert_eq!(queue.pop(|i| heads[i].as_ref().unwrap(), &collator), None);
            }
        }
    }

    #[test]
    fn popped_sources_can_be_refilled_without_duplicate_entries() {
        let collator = ByKey(false);
        let mut heads = [Some(Item(5)), Some(Item(3))];
        let mut queue = MergeQueue::new(heads.len());
        for input in [0, 1, 1] {
            queue.push(input, |i| heads[i].as_ref().unwrap(), &collator);
        }

        assert_eq!(
            queue.pop(|i| heads[i].as_ref().unwrap(), &collator),
            Some(1)
        );
        heads[1] = None;
        assert_eq!(queue.peek(), Some(0));

        heads[1] = Some(Item(8));
        queue.push(1, |i| heads[i].as_ref().unwrap(), &collator);
        assert_eq!(
            queue.pop(|i| heads[i].as_ref().unwrap(), &collator),
            Some(0)
        );
        heads[0] = Some(Item(1));
        queue.push(0, |i| heads[i].as_ref().unwrap(), &collator);

        assert_eq!(
            queue.pop(|i| heads[i].as_ref().unwrap(), &collator),
            Some(0)
        );
        assert_eq!(
            queue.pop(|i| heads[i].as_ref().unwrap(), &collator),
            Some(1)
        );
        assert_eq!(queue.peek(), None);
    }
}
