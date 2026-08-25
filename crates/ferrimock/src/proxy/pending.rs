//! A request body the proxy has not committed to reading yet.
//!
//! Whether the body has to be in memory is not knowable when the request
//! arrives: it depends on whether any mock matches on a body, and then on
//! whether the mock that matched wants to patch one. So the body is carried
//! in a state that can still be collected on demand, and is otherwise handed
//! to the upstream untouched.

use axum::body::Body;
use bytes::{Bytes, BytesMut};
use futures::StreamExt;
use http_body_util::BodyExt;

/// A request body in one of the three states a proxy can leave it in.
pub enum PendingBody {
    /// Untouched. Forwarding it costs one frame of memory whatever its size.
    Stream(Body),
    /// Read into memory, and therefore available to the matcher and to patches.
    Buffered(Bytes),
    /// Read part way and then abandoned for being over the cap. The prefix
    /// still has to reach the upstream, so it is re-emitted ahead of the rest
    /// of the stream rather than dropped.
    Chained { prefix: Bytes, rest: Body },
}

impl PendingBody {
    /// Wrap an incoming body.
    pub fn new(body: Body) -> Self {
        Self::Stream(body)
    }

    /// The body as bytes, collecting it if that has not happened yet.
    ///
    /// Returns `None` when the body is larger than `cap`, or when reading it
    /// failed. Both mean the same thing to a caller: this body is not
    /// available for matching or patching, and the request still forwards.
    pub async fn bytes(&mut self, cap: usize) -> Option<Bytes> {
        match self {
            Self::Buffered(bytes) => Some(bytes.clone()),
            Self::Chained { .. } => None,
            Self::Stream(_) => {
                // An upper bound over the cap is a decision that can be made
                // without reading anything, which is what keeps a large
                // upload from being touched at all.
                let Self::Stream(body) = std::mem::replace(self, Self::Buffered(Bytes::new()))
                else {
                    return None;
                };

                if http_body::Body::size_hint(&body)
                    .upper()
                    .is_some_and(|upper| upper > cap as u64)
                {
                    *self = Self::Chained {
                        prefix: Bytes::new(),
                        rest: body,
                    };
                    return None;
                }

                match collect_up_to(body, cap).await {
                    Ok(Collected::Whole(bytes)) => {
                        *self = Self::Buffered(bytes.clone());
                        Some(bytes)
                    }
                    Ok(Collected::OverCap { prefix, rest }) => {
                        *self = Self::Chained { prefix, rest };
                        None
                    }
                    Err(_) => {
                        *self = Self::Buffered(Bytes::new());
                        None
                    }
                }
            }
        }
    }

    /// Replace the body with these bytes, as a request patch does.
    pub fn replace(&mut self, bytes: Bytes) {
        *self = Self::Buffered(bytes);
    }

    /// Hand the body to the upstream request.
    pub fn into_request_body(self) -> Body {
        match self {
            Self::Stream(inner) => inner,
            Self::Buffered(bytes) => Body::from(bytes),
            // Frames already read cannot be pushed back into the stream, so
            // the prefix goes out ahead of what is left of it.
            Self::Chained { prefix, rest } => {
                if prefix.is_empty() {
                    return rest;
                }
                let head = futures::stream::once(async move { Ok::<_, axum::Error>(prefix) });
                Body::from_stream(head.chain(rest.into_data_stream()))
            }
        }
    }
}

/// What collecting a body up to a cap produced.
enum Collected {
    /// The whole body fit.
    Whole(Bytes),
    /// It did not, and this is how far reading got.
    OverCap { prefix: Bytes, rest: Body },
}

/// Read frames until the body ends or `cap` is passed.
///
/// Frames already read cannot be pushed back, so passing the cap returns them
/// rather than discarding them; the caller re-emits the prefix ahead of the
/// remaining stream.
async fn collect_up_to(mut body: Body, cap: usize) -> Result<Collected, axum::Error> {
    let mut buffer = BytesMut::new();

    while let Some(frame) = body.frame().await {
        let frame = frame?;
        if let Some(data) = frame.data_ref() {
            if buffer.len() + data.len() > cap {
                buffer.extend_from_slice(data);
                return Ok(Collected::OverCap {
                    prefix: buffer.freeze(),
                    rest: body,
                });
            }
            buffer.extend_from_slice(data);
        }
    }

    Ok(Collected::Whole(buffer.freeze()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn a_buffered_body_hands_back_the_same_bytes_every_time() {
        let mut pending = PendingBody::Buffered(Bytes::from_static(b"payload"));
        let first = futures::executor::block_on(pending.bytes(1024)).unwrap();
        let second = futures::executor::block_on(pending.bytes(1024)).unwrap();
        assert_eq!(first, second);
        assert_eq!(first, Bytes::from_static(b"payload"));
    }

    #[tokio::test]
    async fn a_replaced_body_forwards_the_replacement() {
        use http_body_util::BodyExt;

        let mut pending = PendingBody::Buffered(Bytes::from_static(b"before"));
        pending.replace(Bytes::from_static(b"after"));

        let forwarded = pending
            .into_request_body()
            .collect()
            .await
            .unwrap()
            .to_bytes();
        assert_eq!(forwarded, Bytes::from_static(b"after"));
    }

    #[tokio::test]
    async fn an_empty_buffered_body_forwards_as_no_body_at_all() {
        use http_body_util::BodyExt;

        let pending = PendingBody::Buffered(Bytes::new());
        let forwarded = pending
            .into_request_body()
            .collect()
            .await
            .unwrap()
            .to_bytes();
        assert!(forwarded.is_empty());
    }
}
