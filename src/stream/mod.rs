pub use diff::*;
pub use merge::*;
pub use try_diff::*;
pub use try_merge::*;
pub use try_union::try_union;

mod diff;
mod merge;
mod try_diff;
mod try_merge;
mod try_union;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Collator;
    use futures::stream::{self, StreamExt, TryStreamExt};
    use std::fmt;

    #[derive(Debug)]
    struct Error(String);

    impl fmt::Display for Error {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            self.0.fmt(f)
        }
    }

    impl std::error::Error for Error {}

    #[tokio::test]
    async fn test_diff() {
        let collator = Collator::<u32>::default();

        let left = vec![1, 3, 5, 7, 8, 9, 20];
        let right = vec![2, 4, 5, 6, 8, 9];

        let expected = vec![1, 3, 7, 20];
        let actual = diff(collator, stream::iter(left), stream::iter(right))
            .collect::<Vec<u32>>()
            .await;

        assert_eq!(expected, actual);
    }

    #[tokio::test]
    async fn test_try_diff() {
        let collator = Collator::<u32>::default();

        let left = vec![1, 3, 5, 7, 8, 9, 20];
        let right = vec![2, 4, 5, 6, 8, 9];

        let expected = vec![1, 3, 7, 20];
        let mut actual = Vec::with_capacity(expected.len());

        let mut stream = try_diff(
            collator,
            stream::iter(left).map(Result::<u32, Error>::Ok),
            stream::iter(right).map(Result::<u32, Error>::Ok),
        );

        while let Some(n) = stream.try_next().await.expect("n") {
            actual.push(n);
        }

        assert_eq!(expected, actual);
    }

    #[tokio::test]
    async fn test_merge() {
        let collator = Collator::<u32>::default();

        let left = vec![1, 3, 5, 7, 8, 9, 20];
        let right = vec![2, 4, 6, 8, 9, 10, 11, 12];

        let expected = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 20];
        let actual = merge(collator, stream::iter(left), stream::iter(right))
            .collect::<Vec<u32>>()
            .await;

        assert_eq!(expected, actual);
    }

    #[tokio::test]
    async fn test_try_merge() {
        let collator = Collator::<u32>::default();

        let left = vec![1, 3, 5, 7, 8, 9, 20];
        let right = vec![2, 4, 6, 8, 9, 10, 11, 12];

        let expected = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 20];
        let mut actual = Vec::with_capacity(expected.len());

        let mut stream = try_merge(
            collator,
            stream::iter(left).map(Result::<u32, Error>::Ok),
            stream::iter(right).map(Result::<u32, Error>::Ok),
        );

        while let Some(n) = stream.try_next().await.expect("n") {
            actual.push(n);
        }

        assert_eq!(expected, actual);
    }
}
