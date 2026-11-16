use std::cell::{Cell, RefCell};
use std::cmp;
use std::io;
use std::marker::PhantomPinned;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::task::{noop_waker_ref, waker, ArcWake};
use futures::{stream, FutureExt, Stream, StreamExt};

use crate::{diff, merge, try_diff, try_merge, Collate, Collator};

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

#[test]
fn binary_streams_preserve_occurrences_and_support_non_ord_non_unpin_values() {
    struct Item {
        key: i32,
        source: char,
        index: usize,
        _pin: PhantomPinned,
        _local: Cell<()>,
    }

    #[derive(Eq, PartialEq)]
    struct ByKey {
        reverse: bool,
        comparisons: Cell<usize>,
        _pin: PhantomPinned,
    }

    impl Collate for ByKey {
        type Value = Item;

        fn cmp(&self, left: &Item, right: &Item) -> cmp::Ordering {
            self.comparisons.set(self.comparisons.get() + 1);
            let order = left.key.cmp(&right.key);
            if self.reverse {
                order.reverse()
            } else {
                order
            }
        }
    }

    fn collator(reverse: bool) -> ByKey {
        ByKey {
            reverse,
            comparisons: Cell::new(0),
            _pin: PhantomPinned,
        }
    }

    fn input(reverse: bool, source: char, keys: &[i32]) -> impl Stream<Item = Item> + Unpin {
        stream::iter(
            keys.iter()
                .enumerate()
                .map(|(index, &key)| Item {
                    key: if reverse { -key } else { key },
                    source,
                    index,
                    _pin: PhantomPinned,
                    _local: Cell::new(()),
                })
                .collect::<Vec<_>>(),
        )
    }

    async fn collect(values: impl Stream<Item = Item> + Send) -> Vec<(i32, char, usize)> {
        values
            .map(|item| (item.key, item.source, item.index))
            .collect()
            .await
    }

    futures::executor::block_on(async {
        for reverse in [false, true] {
            let expected_merge = [
                (1, 'L', 0),
                (1, 'L', 1),
                (1, 'L', 2),
                (2, 'R', 2),
                (2, 'R', 3),
                (3, 'L', 3),
            ]
            .map(|(key, source, index)| (if reverse { -key } else { key }, source, index));

            let expected_diff = [(1, 'L', 2), (3, 'L', 3)]
                .map(|(key, source, index)| (if reverse { -key } else { key }, source, index));

            let left = || input(reverse, 'L', &[1, 1, 1, 3]);
            let right = || input(reverse, 'R', &[1, 1, 2, 2]);

            assert_eq!(
                collect(merge(collator(reverse), left(), right())).await,
                expected_merge
            );

            assert_eq!(
                collect(diff(collator(reverse), left(), right())).await,
                expected_diff
            );

            assert_eq!(
                collect(
                    try_merge(
                        collator(reverse),
                        left().map(Ok::<_, io::Error>),
                        right().map(Ok::<_, io::Error>)
                    )
                    .map(Result::unwrap)
                )
                .await,
                expected_merge
            );

            assert_eq!(
                collect(
                    try_diff(
                        collator(reverse),
                        left().map(Ok::<_, io::Error>),
                        right().map(Ok::<_, io::Error>)
                    )
                    .map(Result::unwrap)
                )
                .await,
                expected_diff
            );
        }
    });
}

#[test]
fn binary_errors_preserve_poll_order_buffered_heads_and_continuation() {
    fn check(
        mut result: impl Stream<Item = Result<u64, &'static str>> + Unpin,
        polls: Rc<RefCell<Vec<&'static str>>>,
        expected: Vec<u64>,
    ) {
        let mut cx = Context::from_waker(noop_waker_ref());
        assert!(Pin::new(&mut result).poll_next(&mut cx).is_pending());
        assert_eq!(*polls.borrow(), ["left"]);
        assert_eq!(
            result.next().now_or_never().unwrap().unwrap().unwrap_err(),
            "left"
        );

        assert_eq!(*polls.borrow(), ["left", "left"]);
        assert_eq!(
            result.next().now_or_never().unwrap().unwrap().unwrap_err(),
            "right"
        );

        assert_eq!(*polls.borrow(), ["left", "left", "left", "right"]);
        let actual = futures::executor::block_on(result.by_ref().collect::<Vec<_>>());
        assert_eq!(
            actual.into_iter().collect::<Result<Vec<_>, _>>().unwrap(),
            expected
        );

        let before = polls.borrow().len();
        assert!(matches!(
            Pin::new(&mut result).poll_next(&mut cx),
            Poll::Ready(None)
        ));

        assert_eq!(polls.borrow().len(), before);
    }

    for difference in [false, true] {
        let polls = Rc::new(RefCell::new(Vec::new()));
        let left_polls = polls.clone();
        let mut left_steps = [
            Poll::Pending,
            Poll::Ready(Some(Err("left"))),
            Poll::Ready(Some(Ok(1))),
            Poll::Ready(Some(Ok(3))),
            Poll::Ready(None),
        ]
        .into_iter();

        let left = stream::poll_fn(move |_| {
            left_polls.borrow_mut().push("left");
            left_steps.next().expect("left polled after EOF")
        });

        let right_polls = polls.clone();
        let mut right_steps = [Some(Err("right")), Some(Ok(2)), None].into_iter();
        let right = stream::poll_fn(move |_| {
            right_polls.borrow_mut().push("right");
            Poll::Ready(right_steps.next().expect("right polled after EOF"))
        });

        if difference {
            check(
                try_diff(Collator::default(), left, right),
                polls,
                vec![1, 3],
            );
        } else {
            check(
                try_merge(Collator::default(), left, right),
                polls,
                vec![1, 2, 3],
            );
        }
    }
}

#[test]
fn dropping_pending_binary_streams_releases_buffered_heads() {
    fn check(mut result: impl Stream + Unpin, retained: std::sync::Weak<()>) {
        let mut cx = Context::from_waker(noop_waker_ref());
        assert!(Pin::new(&mut result).poll_next(&mut cx).is_pending());
        assert!(retained.upgrade().is_some());
        drop(result);
        assert!(retained.upgrade().is_none());
    }

    for difference in [false, true] {
        for fallible in [false, true] {
            let owner = Arc::new(());
            let retained = Arc::downgrade(&owner);
            let left = stream::iter([owner]);
            let right = stream::pending();

            if fallible {
                let left = left.map(Ok::<_, io::Error>);
                let right = right.map(Ok::<_, io::Error>);
                if difference {
                    check(try_diff(Collator::default(), left, right), retained);
                } else {
                    check(try_merge(Collator::default(), left, right), retained);
                }
            } else if difference {
                check(diff(Collator::default(), left, right), retained);
            } else {
                check(merge(Collator::default(), left, right), retained);
            }
        }
    }
}
