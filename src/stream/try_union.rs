use std::cmp::Ordering;
use std::task::Poll;

use futures::{Stream, TryStream, TryStreamExt};

use crate::CollateRef;

// Bound synchronous progress across ready inputs, including duplicate runs.
const READY_WORK_STEPS: usize = 128;

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
    let inputs: Vec<_> = streams.into_iter().map(|s| (Some(s), None::<T>)).collect();
    futures::stream::try_unfold(
        (inputs, collator, None::<T>, READY_WORK_STEPS),
        |(mut inputs, collator, previous, mut remaining)| async move {
            let mut first: Option<usize> = None;

            for i in 0..inputs.len() {
                loop {
                    futures::future::poll_fn(|cx| {
                        if remaining == 0 {
                            remaining = READY_WORK_STEPS;
                            cx.waker().wake_by_ref();
                            Poll::Pending
                        } else {
                            remaining -= 1;
                            Poll::Ready(())
                        }
                    })
                    .await;

                    let (stream, head) = &mut inputs[i];
                    if let Some(value) = head.as_ref() {
                        if previous.as_ref().is_none_or(|previous| {
                            collator.cmp_ref(value, previous) != Ordering::Equal
                        }) {
                            break;
                        }
                        *head = None;
                    }

                    let Some(source) = stream.as_mut() else {
                        break;
                    };
                    if let Some(value) = source.try_next().await? {
                        *head = Some(value);
                    } else {
                        *stream = None;
                        break;
                    }
                }

                if let Some(value) = inputs[i].1.as_ref() {
                    if first.is_none_or(|first| {
                        collator.cmp_ref(value, inputs[first].1.as_ref().expect("ready head"))
                            == Ordering::Less
                    }) {
                        first = Some(i);
                    }
                }
            }

            match first {
                Some(first) => {
                    let value = inputs[first].1.take().expect("selected head");
                    let previous = Some(value.clone());
                    Ok(Some((value, (inputs, collator, previous, remaining))))
                }
                None => Ok(None),
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::task::Poll;

    use futures::stream::{self, BoxStream};
    use futures::{StreamExt, TryStreamExt};

    use super::try_union;
    use crate::Collator;

    #[tokio::test]
    async fn union_deduplicates_and_releases_inputs() {
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
    }

    #[tokio::test]
    async fn union_polls_in_order_and_stops_at_first_error() {
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
    }

    #[tokio::test]
    async fn union_yields_and_cancellation_releases_inputs() {
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
    }

    #[tokio::test]
    async fn exhausted_input_is_released_before_other_inputs() {
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
    }
}
