use std::{cmp::Ordering, task::Poll};

use futures::{Stream, TryStream, TryStreamExt};

use super::{MergeQueue, READY_WORK_STEPS};
use crate::CollateRef;

/// Return the unique union of a finite collection of collated streams.
///
/// Inputs must have the same error type and be collated by `collator`. Duplicates
/// within and across inputs are omitted. Input order determines polling and first
/// error precedence; unordered input has unspecified output order.
///
/// Retains one head per input and one previous output. Exhausted inputs are
/// released immediately, as are all inputs on the first error. Polling and drop
/// are flat, and ready work yields cooperatively even for long duplicate runs.
pub fn try_union<C, T, E, S>(collator: C, streams: Vec<S>) -> impl Stream<Item = Result<T, E>>
where
    C: CollateRef<T>,
    T: Clone,
    S: TryStream<Ok = T, Error = E> + Unpin,
{
    let queue = MergeQueue::new(streams.len());
    let refill: Vec<_> = (0..streams.len()).collect();
    let inputs: Vec<_> = streams
        .into_iter()
        .map(|source| (Some(source), None::<T>))
        .collect();

    futures::stream::try_unfold(
        (inputs, queue, refill, collator, None::<T>, READY_WORK_STEPS),
        |(mut inputs, mut queue, mut refill, collator, previous, mut remaining)| async move {
            // Consume other heads equal to the preceding output before polling
            // any source. Refill in input order, preserving first-error precedence.
            while let Some(first) = queue.peek() {
                if previous.as_ref().is_none_or(|previous| {
                    collator.cmp_ref(inputs[first].1.as_ref().expect("ready head"), previous)
                        != Ordering::Equal
                }) {
                    break;
                }
                progress(&mut remaining).await;
                queue
                    .pop(
                        |input| inputs[input].1.as_ref().expect("ready head"),
                        &collator,
                    )
                    .expect("selected source");

                inputs[first].1 = None;
                refill.push(first);
            }

            refill.sort_unstable_by(|left, right| right.cmp(left));
            while let Some(input) = refill.pop() {
                loop {
                    progress(&mut remaining).await;
                    let Some(source) = inputs[input].0.as_mut() else {
                        break;
                    };

                    match source.try_next().await? {
                        Some(value) => {
                            if previous.as_ref().is_some_and(|previous| {
                                collator.cmp_ref(&value, previous) == Ordering::Equal
                            }) {
                                continue;
                            }
                            inputs[input].1 = Some(value);
                            queue.push(
                                input,
                                |index| inputs[index].1.as_ref().expect("ready head"),
                                &collator,
                            );

                            break;
                        }
                        None => {
                            inputs[input].0 = None;
                            break;
                        }
                    }
                }
            }

            match queue.pop(
                |input| inputs[input].1.as_ref().expect("ready head"),
                &collator,
            ) {
                Some(first) => {
                    let value = inputs[first].1.take().expect("selected head");
                    refill.push(first);
                    let previous = Some(value.clone());
                    Ok(Some((
                        value,
                        (inputs, queue, refill, collator, previous, remaining),
                    )))
                }
                None => Ok(None),
            }
        },
    )
}

async fn progress(remaining: &mut usize) {
    futures::future::poll_fn(|cx| {
        if *remaining == 0 {
            *remaining = READY_WORK_STEPS;
            cx.waker().wake_by_ref();
            Poll::Pending
        } else {
            *remaining -= 1;
            Poll::Ready(())
        }
    })
    .await;
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::sync::{Arc, Mutex};
    use std::task::Poll;

    use futures::executor::block_on;
    use futures::stream::{self, BoxStream};
    use futures::{StreamExt, TryStreamExt};

    use super::try_union;
    use crate::Collator;

    #[test]
    fn union_preserves_send_with_non_ord_values_and_non_sync_collators() {
        block_on(async {
            #[derive(Clone)]
            struct Item(Cell<usize>);

            #[derive(Eq, PartialEq)]
            struct Counted(Cell<usize>);

            impl crate::Collate for Counted {
                type Value = Item;

                fn cmp(&self, left: &Item, right: &Item) -> std::cmp::Ordering {
                    self.0.set(self.0.get() + 1);
                    left.0.get().cmp(&right.0.get())
                }
            }

            let inputs = [vec![1, 3], vec![2, 3]]
                .into_iter()
                .map(|values| {
                    stream::iter(
                        values
                            .into_iter()
                            .map(|value| Ok::<_, ()>(Item(Cell::new(value)))),
                    )
                })
                .collect();

            let merged = try_union(Counted(Cell::new(0)), inputs);

            fn assert_send<T: Send>(_: &T) {}

            assert_send(&merged);

            let values = merged.try_collect::<Vec<_>>().await.unwrap();
            assert_eq!(
                values
                    .into_iter()
                    .map(|value| value.0.get())
                    .collect::<Vec<_>>(),
                vec![1, 2, 3]
            );
        });
    }

    #[test]
    fn high_fanout_selection_is_logarithmic() {
        block_on(async {
            use std::sync::atomic::{AtomicUsize, Ordering};

            #[derive(Clone)]
            struct Counted(Arc<AtomicUsize>);

            impl PartialEq for Counted {
                fn eq(&self, other: &Self) -> bool {
                    Arc::ptr_eq(&self.0, &other.0)
                }
            }

            impl Eq for Counted {}

            impl crate::Collate for Counted {
                type Value = usize;

                fn cmp(&self, left: &usize, right: &usize) -> std::cmp::Ordering {
                    self.0.fetch_add(1, Ordering::Relaxed);
                    left.cmp(right)
                }
            }

            let count = Arc::new(AtomicUsize::new(0));
            let inputs = (0..1024)
                .map(|input| {
                    stream::iter((0..32).map(move |index| Ok::<_, ()>(index * 1024 + input)))
                })
                .collect();

            let mut merged = Box::pin(try_union(Counted(count.clone()), inputs));
            let mut expected = 0;
            while let Some(value) = merged.try_next().await.unwrap() {
                assert_eq!(value, expected);
                expected += 1;
            }

            assert_eq!(expected, 1024 * 32);
            assert!(count.load(Ordering::Relaxed) < expected * 32);
        });
    }

    #[test]
    fn duplicate_refills_preserve_error_order_after_output() {
        block_on(async {
            for failing in [0, 1] {
                let inputs = (0..3)
                    .map(|input| {
                        let mut values = vec![Ok(1); 3];
                        values.push(Err(if input == failing { 0 } else { input + 1 }));
                        stream::iter(values)
                    })
                    .collect();

                let mut merged = Box::pin(try_union(Collator::default(), inputs));
                assert_eq!(merged.try_next().await, Ok(Some(1)));
                assert_eq!(
                    merged.try_next().await,
                    Err(if failing == 0 { 0 } else { 1 })
                );

                assert_eq!(merged.try_next().await, Ok(None));
            }
        });
    }

    #[test]
    fn union_deduplicates_and_releases_inputs() {
        block_on(async {
            for (inputs, expected) in [
                (vec![], vec![]),
                (vec![vec![]], vec![]),
                (vec![vec![1, 1, 2, 2]], vec![1, 2]),
                (vec![vec![1, 3, 3], vec![], vec![1, 2, 4]], vec![1, 2, 3, 4]),
            ] {
                let lease = Arc::new(());
                let weak = Arc::downgrade(&lease);
                let streams: Vec<_> = inputs
                    .into_iter()
                    .map(|input| {
                        let lease = Arc::clone(&lease);
                        stream::iter(input).map(move |value| {
                            let _ = &lease;
                            Ok::<_, &'static str>(value)
                        })
                    })
                    .collect();

                drop(lease);
                let mut union = Box::pin(try_union(Collator::default(), streams));
                let mut values = Vec::new();

                while let Some(value) = union.try_next().await.unwrap() {
                    values.push(value);
                }

                assert_eq!(values, expected);
                assert!(weak.upgrade().is_none());
                assert_eq!(union.try_next().await.unwrap(), None);
            }
        });
    }

    #[test]
    fn union_polls_in_order_and_stops_at_first_error() {
        block_on(async {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let lease = Arc::new(());
            let weak = Arc::downgrade(&lease);
            let streams: Vec<BoxStream<'_, Result<u64, usize>>> = (0..3)
                .map(|index| {
                    let lease = Arc::clone(&lease);
                    let calls = Arc::clone(&calls);
                    let mut pending = index == 0;
                    stream::poll_fn(move |cx| {
                        let _ = &lease;
                        calls.lock().unwrap().push(index);
                        if pending {
                            pending = false;
                            cx.waker().wake_by_ref();
                            Poll::Pending
                        } else {
                            Poll::Ready(Some(Err(index)))
                        }
                    })
                    .boxed()
                })
                .collect();

            drop(lease);
            let mut union = Box::pin(try_union(Collator::default(), streams));
            assert!(matches!(futures::poll!(union.try_next()), Poll::Pending));
            assert_eq!(*calls.lock().unwrap(), vec![0]);
            assert_eq!(union.try_next().await, Err(0));
            assert_eq!(*calls.lock().unwrap(), vec![0, 0]);
            assert!(weak.upgrade().is_none());
            assert_eq!(union.try_next().await, Ok(None));
        });
    }

    #[test]
    fn union_yields_and_cancellation_releases_inputs() {
        block_on(async {
            for inputs in [1, 257] {
                let lease = Arc::new(());
                let weak = Arc::downgrade(&lease);
                let streams: Vec<_> = (0..inputs)
                    .map(|_| {
                        let lease = Arc::clone(&lease);
                        stream::repeat_with(move || {
                            let _ = &lease;
                            Ok::<_, ()>(0u64)
                        })
                    })
                    .collect();

                drop(lease);
                let mut union = Box::pin(try_union(Collator::default(), streams));
                if inputs == 1 {
                    assert_eq!(union.try_next().await, Ok(Some(0)));
                }

                assert!(matches!(futures::poll!(union.try_next()), Poll::Pending));
                assert!(weak.upgrade().is_some());
                drop(union);
                assert!(weak.upgrade().is_none());
            }
        });
    }

    #[test]
    fn exhausted_input_is_released_before_other_inputs() {
        block_on(async {
            let lease = Arc::new(());
            let weak = Arc::downgrade(&lease);
            let mut inputs: Vec<BoxStream<'_, Result<u64, ()>>> = Vec::new();
            inputs.push(
                stream::empty()
                    .map(move |item| {
                        let _ = &lease;
                        item
                    })
                    .boxed(),
            );

            inputs.push(stream::iter([Ok(1), Ok(2)]).boxed());
            let mut union = Box::pin(try_union(Collator::default(), inputs));
            assert_eq!(union.try_next().await, Ok(Some(1)));
            assert!(weak.upgrade().is_none());
            assert_eq!(union.try_next().await, Ok(Some(2)));
        });
    }
}
