use std::cmp::Ordering;
use std::pin::Pin;
use std::task::{ready, Context, Poll};

use futures::stream::{IntoStream, Peekable, Stream, StreamExt, TryStream, TryStreamExt};

use super::READY_WORK_STEPS;
use crate::CollateRef;

/// The stream returned by [`try_diff`].
pub struct TryDiff<C, T, L: TryStream<Ok = T>, R: TryStream<Ok = T>> {
    collator: C,
    left: Peekable<IntoStream<L>>,
    right: Peekable<IntoStream<R>>,
}

// Only inputs participate in pinning. The collator and buffered values are
// never projected as pinned, so they need no Unpin bound.
impl<C, T, L, R> Unpin for TryDiff<C, T, L, R>
where
    L: TryStream<Ok = T> + Unpin,
    R: TryStream<Ok = T> + Unpin,
{
}

impl<C, T, E, L, R> Stream for TryDiff<C, T, L, R>
where
    C: CollateRef<T>,
    L: TryStream<Ok = T, Error = E> + Unpin,
    R: TryStream<Ok = T, Error = E> + Unpin,
{
    type Item = Result<T, E>;

    fn poll_next(self: Pin<&mut Self>, cxt: &mut Context) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        for _ in 0..READY_WORK_STEPS {
            let left = match ready!(Pin::new(&mut this.left).poll_peek(cxt)) {
                Some(Ok(value)) => value,
                Some(Err(_)) => return this.left.poll_next_unpin(cxt),
                None => return Poll::Ready(None),
            };

            let right = match ready!(Pin::new(&mut this.right).poll_peek(cxt)) {
                Some(Ok(value)) => value,
                Some(Err(_)) => return this.right.poll_next_unpin(cxt),
                None => return this.left.poll_next_unpin(cxt),
            };

            match this.collator.cmp_ref(left, right) {
                Ordering::Equal => {
                    ready!(this.left.poll_next_unpin(cxt));
                    ready!(this.right.poll_next_unpin(cxt));
                }
                Ordering::Less => return this.left.poll_next_unpin(cxt),
                Ordering::Greater => {
                    ready!(this.right.poll_next_unpin(cxt));
                }
            }
        }

        cxt.waker().wake_by_ref();
        Poll::Pending
    }
}

/// Compute the difference of two collated [`TryStream`]s,
/// i.e. return the items in `left` that are not in `right`.
/// Both input streams **must** be collated.
/// Each matching right value removes one occurrence from the left input.
/// Errors are emitted in input polling order; polling may continue afterward.
/// Errors are forwarded unchanged and need not implement [`std::error::Error`].
/// If either input stream is not collated, the behavior of the output stream is undefined.
pub fn try_diff<C, T, E, L, R>(collator: C, left: L, right: R) -> TryDiff<C, T, L, R>
where
    C: CollateRef<T>,
    L: TryStream<Ok = T, Error = E>,
    R: TryStream<Ok = T, Error = E>,
{
    TryDiff {
        collator,
        left: left.into_stream().peekable(),
        right: right.into_stream().peekable(),
    }
}
