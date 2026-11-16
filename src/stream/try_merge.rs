use std::cmp::Ordering;
use std::pin::Pin;
use std::task::{ready, Context, Poll};

use futures::stream::{IntoStream, Peekable, Stream, StreamExt, TryStream, TryStreamExt};

use crate::CollateRef;

/// The stream returned by [`try_merge`].
pub struct TryMerge<C, T, L: TryStream<Ok = T>, R: TryStream<Ok = T>> {
    collator: C,
    left: Peekable<IntoStream<L>>,
    right: Peekable<IntoStream<R>>,
}

// Only inputs participate in pinning. The collator and buffered values are
// never projected as pinned, so they need no Unpin bound.
impl<C, T, L, R> Unpin for TryMerge<C, T, L, R>
where
    L: TryStream<Ok = T> + Unpin,
    R: TryStream<Ok = T> + Unpin,
{
}

impl<C, T, E, L, R> Stream for TryMerge<C, T, L, R>
where
    C: CollateRef<T>,
    L: TryStream<Ok = T, Error = E> + Unpin,
    R: TryStream<Ok = T, Error = E> + Unpin,
{
    type Item = Result<T, E>;

    fn poll_next(self: Pin<&mut Self>, cxt: &mut Context) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let left = match ready!(Pin::new(&mut this.left).poll_peek(cxt)) {
            Some(Ok(value)) => Some(value),
            Some(Err(_)) => return this.left.poll_next_unpin(cxt),
            None => None,
        };

        let right = match ready!(Pin::new(&mut this.right).poll_peek(cxt)) {
            Some(Ok(value)) => Some(value),
            Some(Err(_)) => return this.right.poll_next_unpin(cxt),
            None => None,
        };

        match (left, right) {
            (Some(left), Some(right)) => match this.collator.cmp_ref(left, right) {
                Ordering::Equal => {
                    ready!(this.right.poll_next_unpin(cxt));
                    this.left.poll_next_unpin(cxt)
                }
                Ordering::Less => this.left.poll_next_unpin(cxt),
                Ordering::Greater => this.right.poll_next_unpin(cxt),
            },
            (Some(_), None) => this.left.poll_next_unpin(cxt),
            (None, Some(_)) => this.right.poll_next_unpin(cxt),
            (None, None) => Poll::Ready(None),
        }
    }
}

/// Merge two collated [`TryStream`]s into one using the given `collator`.
/// Both input streams **must** be collated and have the same error type.
/// Equal heads retain the left value and consume one value from each input;
/// repeated values within a single input are preserved.
/// Errors are emitted in input polling order; polling may continue afterward.
/// Errors are forwarded unchanged and need not implement [`std::error::Error`].
/// If either input stream is not collated, the order of the output stream is undefined.
pub fn try_merge<C, T, E, L, R>(collator: C, left: L, right: R) -> TryMerge<C, T, L, R>
where
    C: CollateRef<T>,
    L: TryStream<Ok = T, Error = E>,
    R: TryStream<Ok = T, Error = E>,
{
    TryMerge {
        collator,
        left: left.into_stream().peekable(),
        right: right.into_stream().peekable(),
    }
}
