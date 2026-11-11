use std::cmp::Ordering;
use std::pin::Pin;
use std::task::{ready, Context, Poll};

use futures::stream::{Fuse, Stream, StreamExt, TryStream};
use pin_project::pin_project;

use super::READY_WORK_STEPS;
use crate::CollateRef;

/// The stream type returned by [`diff`].
/// The implementation of this stream is based on
/// [`stream::select`](https://github.com/rust-lang/futures-rs/blob/master/futures-util/src/stream/select.rs).
#[pin_project]
pub struct TryDiff<C, T, L, R> {
    collator: C,

    #[pin]
    left: Fuse<L>,
    #[pin]
    right: Fuse<R>,

    pending_left: Option<T>,
    pending_right: Option<T>,
}

impl<C, T, E, L, R> Stream for TryDiff<C, T, L, R>
where
    C: CollateRef<T>,
    E: std::error::Error,
    Fuse<L>: TryStream<Ok = T, Error = E> + Unpin,
    Fuse<R>: TryStream<Ok = T, Error = E> + Unpin,
{
    type Item = Result<T, E>;

    fn poll_next(self: Pin<&mut Self>, cxt: &mut Context) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
        let mut remaining = READY_WORK_STEPS;

        Poll::Ready(loop {
            if remaining == 0 {
                cxt.waker().wake_by_ref();
                return Poll::Pending;
            }
            remaining -= 1;

            let left_done = if this.left.is_done() {
                true
            } else if this.pending_left.is_none() {
                match ready!(this.left.as_mut().try_poll_next(cxt)) {
                    Some(Ok(value)) => {
                        *this.pending_left = Some(value);
                        false
                    }
                    Some(Err(cause)) => break Some(Err(cause)),
                    None => true,
                }
            } else {
                false
            };

            if left_done {
                break None;
            }

            let right_done = if this.right.is_done() {
                true
            } else if this.pending_right.is_none() {
                match ready!(this.right.as_mut().try_poll_next(cxt)) {
                    Some(Ok(value)) => {
                        *this.pending_right = Some(value);
                        false
                    }
                    Some(Err(cause)) => break Some(Err(cause)),
                    None => true,
                }
            } else {
                false
            };

            if this.pending_left.is_some() && this.pending_right.is_some() {
                let l_value = this.pending_left.as_ref().unwrap();
                let r_value = this.pending_right.as_ref().unwrap();

                match this.collator.cmp_ref(l_value, r_value) {
                    Ordering::Equal => {
                        // this value is present in the right stream, so drop it
                        this.pending_left.take();
                        this.pending_right.take();
                    }
                    Ordering::Less => {
                        // this value is not present in the right stream, so return it
                        break this.pending_left.take().map(Ok);
                    }
                    Ordering::Greater => {
                        // this value could be present in the right stream--wait and see
                        this.pending_right.take();
                    }
                }
            } else if right_done && this.pending_left.is_some() {
                break this.pending_left.take().map(Ok);
            }
        })
    }
}

/// Compute the difference of two collated [`TryStream`]s,
/// i.e. return the items in `left` that are not in `right`.
/// Both input streams **must** be collated.
/// If either input stream is not collated, the behavior of the output stream is undefined.
pub fn try_diff<C, T, E, L, R>(collator: C, left: L, right: R) -> TryDiff<C, T, L, R>
where
    C: CollateRef<T>,
    E: std::error::Error,
    L: TryStream<Ok = T, Error = E>,
    R: TryStream<Ok = T, Error = E>,
{
    TryDiff {
        collator,
        left: left.fuse(),
        right: right.fuse(),
        pending_left: None,
        pending_right: None,
    }
}
