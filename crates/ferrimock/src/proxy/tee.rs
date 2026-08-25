//! A body that streams and keeps a copy at the same time.

use bytes::{Bytes, BytesMut};
use http_body::{Body, Frame, SizeHint};
use pin_project_lite::pin_project;
use std::pin::Pin;
use std::task::{Context, Poll};

/// What a completed body hands to whoever asked for a copy of it.
pub type OnComplete = Box<dyn FnOnce(Bytes) + Send + 'static>;

pin_project! {
    /// Wraps a body, forwarding every frame while accumulating a copy.
    ///
    /// Recording a response by collecting it first would turn every recorded
    /// event stream into a request that never answers. Teeing keeps the
    /// browser's copy flowing and hands the recorder a complete body once the
    /// last frame has gone out.
    ///
    /// `captured` goes to `None` once the response outgrows `cap`: a recording
    /// is a convenience and a 4GB download is not going into memory for one.
    pub struct TeeBody {
        #[pin]
        inner: axum::body::Body,
        captured: Option<BytesMut>,
        cap: usize,
        on_complete: Option<OnComplete>,
    }

    impl PinnedDrop for TeeBody {
        fn drop(this: Pin<&mut Self>) {
            // A body whose length was known is finished as soon as that many
            // bytes have been written, and hyper stops polling there rather
            // than asking once more for the `None` that would have committed
            // the recording. `is_end_stream` is what separates that from a
            // client that disconnected half way, whose partial capture must
            // not be recorded as a complete response.
            let this = this.project();
            if !Body::is_end_stream(&*this.inner) {
                return;
            }
            if let (Some(on_complete), Some(captured)) =
                (this.on_complete.take(), this.captured.take())
            {
                on_complete(captured.freeze());
            }
        }
    }
}

impl TeeBody {
    /// Tee `inner`, calling `on_complete` with the whole body once it ends.
    pub fn new(inner: axum::body::Body, cap: usize, on_complete: OnComplete) -> Self {
        Self {
            inner,
            captured: Some(BytesMut::new()),
            cap,
            on_complete: Some(on_complete),
        }
    }
}

impl Body for TeeBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        let this = self.project();
        let polled = std::task::ready!(this.inner.poll_frame(cx));

        match polled {
            Some(Ok(frame)) => {
                if let Some(data) = frame.data_ref()
                    && let Some(captured) = this.captured.as_mut()
                {
                    if captured.len() + data.len() > *this.cap {
                        *this.captured = None;
                    } else {
                        captured.extend_from_slice(data);
                    }
                }
                Poll::Ready(Some(Ok(frame)))
            }
            Some(Err(error)) => {
                // A body that failed part way through is not a recording of
                // anything, so the callback is dropped rather than handed a
                // truncated response that would later replay as complete.
                this.on_complete.take();
                Poll::Ready(Some(Err(error)))
            }
            None => {
                if let (Some(on_complete), Some(captured)) =
                    (this.on_complete.take(), this.captured.take())
                {
                    on_complete(captured.freeze());
                }
                Poll::Ready(None)
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        Body::is_end_stream(&self.inner)
    }

    fn size_hint(&self) -> SizeHint {
        Body::size_hint(&self.inner)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn every_frame_reaches_both_the_client_and_the_recorder() {
        let captured: Arc<Mutex<Option<Bytes>>> = Arc::new(Mutex::new(None));
        let sink = Arc::clone(&captured);

        let tee = TeeBody::new(
            axum::body::Body::from("hello world"),
            1024,
            Box::new(move |bytes| *sink.lock().unwrap() = Some(bytes)),
        );

        let forwarded = tee.collect().await.unwrap().to_bytes();
        assert_eq!(forwarded, Bytes::from_static(b"hello world"));
        assert_eq!(
            captured.lock().unwrap().clone(),
            Some(Bytes::from_static(b"hello world"))
        );
    }

    #[tokio::test]
    async fn a_body_past_the_cap_still_forwards_but_is_not_recorded() {
        let captured: Arc<Mutex<Option<Bytes>>> = Arc::new(Mutex::new(None));
        let sink = Arc::clone(&captured);

        let tee = TeeBody::new(
            axum::body::Body::from("x".repeat(100)),
            10,
            Box::new(move |bytes| *sink.lock().unwrap() = Some(bytes)),
        );

        let forwarded = tee.collect().await.unwrap().to_bytes();
        assert_eq!(forwarded.len(), 100, "the client still gets the whole body");
        assert!(
            captured.lock().unwrap().is_none(),
            "nothing over the cap should have been kept"
        );
    }
}
