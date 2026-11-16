use std::pin::Pin;
use std::task::{Context, Poll};

use futures::stream::{Stream, StreamExt};

use super::{infallible, unwrap_infallible, InfallibleInput, TryMerge};
use crate::CollateRef;

/// The stream returned by [`merge`].
pub struct Merge<C, T, L: Stream<Item = T>, R: Stream<Item = T>> {
    inner: TryMerge<C, T, InfallibleInput<L>, InfallibleInput<R>>,
}

impl<C, T, L, R> Stream for Merge<C, T, L, R>
where
    C: CollateRef<T>,
    L: Stream<Item = T> + Unpin,
    R: Stream<Item = T> + Unpin,
{
    type Item = T;

    fn poll_next(self: Pin<&mut Self>, cxt: &mut Context) -> Poll<Option<Self::Item>> {
        self.get_mut()
            .inner
            .poll_next_unpin(cxt)
            .map(|value| value.map(unwrap_infallible))
    }
}

/// Merge two collated [`Stream`]s into one using the given `collator`.
/// Both input streams **must** be collated.
/// Equal heads retain the left value and consume one value from each input;
/// repeated values within a single input are preserved.
/// If either input stream is not collated, the order of the output stream is undefined.
pub fn merge<C, T, L, R>(collator: C, left: L, right: R) -> Merge<C, T, L, R>
where
    C: CollateRef<T>,
    L: Stream<Item = T>,
    R: Stream<Item = T>,
{
    Merge {
        inner: super::try_merge(collator, infallible(left), infallible(right)),
    }
}
