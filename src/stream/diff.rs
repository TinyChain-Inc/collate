use std::pin::Pin;
use std::task::{Context, Poll};

use futures::stream::{Stream, StreamExt};

use super::{infallible, unwrap_infallible, InfallibleInput, TryDiff};
use crate::CollateRef;

/// The stream returned by [`diff`].
pub struct Diff<C, T, L: Stream<Item = T>, R: Stream<Item = T>> {
    inner: TryDiff<C, T, InfallibleInput<L>, InfallibleInput<R>>,
}

impl<C, T, L, R> Stream for Diff<C, T, L, R>
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

/// Compute the difference of two collated [`Stream`]s,
/// i.e. return the items in `left` that are not in `right`.
/// Both input streams **must** be collated.
/// Each matching right value removes one occurrence from the left input.
/// If either input stream is not collated, the behavior of the output stream is undefined.
pub fn diff<C, T, L, R>(collator: C, left: L, right: R) -> Diff<C, T, L, R>
where
    C: CollateRef<T>,
    L: Stream<Item = T>,
    R: Stream<Item = T>,
{
    Diff {
        inner: super::try_diff(collator, infallible(left), infallible(right)),
    }
}
