use std::cell::Cell;
use std::io;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::task::{waker, ArcWake};
use futures::{stream, FutureExt, Stream, StreamExt};

use crate::{diff, try_diff, Collator};

#[derive(Default)]
struct Wakes(AtomicUsize);

impl ArcWake for Wakes {
    fn wake_by_ref(this: &Arc<Self>) {
        this.0.fetch_add(1, Ordering::Relaxed);
    }
}

fn observed(
    mut values: std::ops::Range<u64>,
    count: Rc<Cell<usize>>,
    owner: Arc<()>,
) -> impl Stream<Item = u64> + Unpin {
    stream::poll_fn(move |_| {
        let _retain = &owner;
        count.set(count.get() + 1);
        Poll::Ready(values.next())
    })
}

#[test]
fn differences_yield_during_ready_runs_and_release_inputs_on_drop() {
    for matching in [false, true] {
        let expected = if matching { vec![] } else { vec![1024] };
        for fallible in [false, true] {
            let polls = Rc::new(Cell::new(0));
            let owner = Arc::new(());
            let retained = Arc::downgrade(&owner);
            let left = if matching { 0..1024 } else { 1024..1025 };
            let left = observed(left, polls.clone(), owner.clone());
            let right = observed(0..1024, polls.clone(), owner);
            let wakes = Arc::new(Wakes::default());
            let waker = waker(wakes.clone());
            let mut cx = Context::from_waker(&waker);

            if fallible {
                let mut result = try_diff(
                    Collator::default(),
                    left.map(Ok::<_, io::Error>),
                    right.map(Ok::<_, io::Error>),
                );
                assert!(Pin::new(&mut result).poll_next(&mut cx).is_pending());
                assert!(polls.get() <= 256);
                assert_eq!(wakes.0.load(Ordering::Relaxed), 1);
                let actual = futures::executor::block_on(result.by_ref().collect::<Vec<_>>());
                assert_eq!(
                    actual.into_iter().collect::<Result<Vec<_>, _>>().unwrap(),
                    expected
                );
            } else {
                let mut result = diff(Collator::default(), left, right);
                assert!(Pin::new(&mut result).poll_next(&mut cx).is_pending());
                assert!(polls.get() <= 256);
                assert_eq!(wakes.0.load(Ordering::Relaxed), 1);
                assert_eq!(
                    futures::executor::block_on(result.by_ref().collect::<Vec<_>>()),
                    expected
                );
            }
            assert!(retained.upgrade().is_none());
        }
    }

    for fallible in [false, true] {
        let count = Rc::new(Cell::new(0));
        let owner = Arc::new(());
        let retained = Arc::downgrade(&owner);
        let left = observed(0..1024, count.clone(), owner.clone());
        let right = observed(0..1024, count, owner);
        let wakes = Arc::new(Wakes::default());
        let waker = waker(wakes);
        let mut cx = Context::from_waker(&waker);
        if fallible {
            let mut result = try_diff(
                Collator::default(),
                left.map(Ok::<_, io::Error>),
                right.map(Ok::<_, io::Error>),
            );
            assert!(Pin::new(&mut result).poll_next(&mut cx).is_pending());
            assert!(retained.upgrade().is_some());
            drop(result);
        } else {
            let mut result = diff(Collator::default(), left, right);
            assert!(Pin::new(&mut result).poll_next(&mut cx).is_pending());
            assert!(retained.upgrade().is_some());
            drop(result);
        }
        assert!(retained.upgrade().is_none());
    }
}

#[test]
fn fallible_difference_preserves_error_order_and_continuation_after_yielding() {
    let left =
        stream::iter(
            (0..1024)
                .map(Ok)
                .chain([Err(io::Error::other("left")), Ok(1025), Ok(1027)]),
        );
    let right = stream::iter(
        (0..1024)
            .map(Ok)
            .chain([Err(io::Error::other("right")), Ok(1026)]),
    );
    let mut result = try_diff(Collator::default(), left, right);
    futures::executor::block_on(async {
        assert_eq!(
            result.next().await.unwrap().unwrap_err().to_string(),
            "left"
        );
        assert_eq!(
            result.next().await.unwrap().unwrap_err().to_string(),
            "right"
        );
        assert_eq!(result.next().await.unwrap().unwrap(), 1025);
        assert_eq!(result.next().await.unwrap().unwrap(), 1027);
        assert!(result.next().await.is_none());
    });
}

#[test]
fn empty_left_difference_does_not_poll_the_right_source() {
    let right = stream::poll_fn(|_| -> std::task::Poll<Option<u64>> {
        panic!("right source must not be polled")
    });
    let mut result = diff(Collator::default(), stream::empty(), right);
    assert_eq!(result.next().now_or_never(), Some(None));
    let right = stream::poll_fn(
        |_| -> std::task::Poll<Option<Result<u64, std::io::Error>>> {
            panic!("right source must not be polled")
        },
    );
    let mut result = try_diff(Collator::default(), stream::empty(), right);
    assert!(matches!(result.next().now_or_never(), Some(None)));
}
