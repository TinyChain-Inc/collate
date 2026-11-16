use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::stream::Stream;
use pin_project_lite::pin_project;

pub use diff::*;
pub use merge::*;
pub use merge_queue::MergeQueue;
pub use try_diff::*;
pub use try_merge::*;
pub use try_union::try_union;

mod diff;
mod merge;
mod merge_queue;
mod try_diff;
mod try_merge;
mod try_union;

// Bound synchronous progress across ready inputs, including matching runs.
const READY_WORK_STEPS: usize = 128;

pin_project! {
    struct InfallibleInput<S> {
        #[pin]
        stream: S,
    }
}

impl<S: Stream> Stream for InfallibleInput<S> {
    type Item = Result<S::Item, Infallible>;

    fn poll_next(self: Pin<&mut Self>, cxt: &mut Context) -> Poll<Option<Self::Item>> {
        self.project()
            .stream
            .poll_next(cxt)
            .map(|value| value.map(Ok))
    }
}

fn infallible<S: Stream>(stream: S) -> InfallibleInput<S> {
    InfallibleInput { stream }
}

fn unwrap_infallible<T>(value: Result<T, Infallible>) -> T {
    match value {
        Ok(value) => value,
        Err(never) => match never {},
    }
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;
    use futures::stream::{self, StreamExt, TryStreamExt};

    use super::{diff, merge, try_diff, try_merge};
    use crate::Collator;

    #[test]
    fn test_diff() {
        block_on(async {
            let collator = Collator::<u32>::default();
            let left = [1, 3, 5, 7, 8, 9, 20];
            let right = [2, 4, 5, 6, 8, 9];
            let expected = [1, 3, 7, 20];
            let actual = diff(collator, stream::iter(left), stream::iter(right))
                .collect::<Vec<_>>()
                .await;

            assert_eq!(actual, expected);

            let actual = try_diff(
                collator,
                stream::iter(left).map(Ok::<_, ()>),
                stream::iter(right).map(Ok::<_, ()>),
            )
            .try_collect::<Vec<_>>()
            .await
            .unwrap();

            assert_eq!(actual, expected);
        });
    }

    #[test]
    fn test_merge() {
        block_on(async {
            let collator = Collator::<u32>::default();
            let left = [1, 3, 5, 7, 8, 9, 20];
            let right = [2, 4, 6, 8, 9, 10, 11, 12];
            let expected = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 20];
            let actual = merge(collator, stream::iter(left), stream::iter(right))
                .collect::<Vec<_>>()
                .await;

            assert_eq!(actual, expected);

            let actual = try_merge(
                collator,
                stream::iter(left).map(Ok::<_, ()>),
                stream::iter(right).map(Ok::<_, ()>),
            )
            .try_collect::<Vec<_>>()
            .await
            .unwrap();

            assert_eq!(actual, expected);
        });
    }
}
