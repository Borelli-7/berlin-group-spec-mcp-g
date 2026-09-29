use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, ReadBuf};

pub const MAX_REQUEST_BYTES: usize = 64 * 1024;

/// Bounds newline-delimited input before the SDK allocates/deserializes a full frame.
pub struct BoundedInput<R> {
    inner: R,
    line_bytes: usize,
}

impl<R> BoundedInput<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            line_bytes: 0,
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for BoundedInput<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let initial = buf.filled().len();
        match Pin::new(&mut this.inner).poll_read(cx, buf) {
            Poll::Ready(Ok(())) => {
                for byte in &buf.filled()[initial..] {
                    if *byte == b'\n' {
                        this.line_bytes = 0;
                    } else {
                        this.line_bytes += 1;
                        if this.line_bytes > MAX_REQUEST_BYTES {
                            buf.set_filled(initial);
                            return Poll::Ready(Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "MCP request exceeds 64 KiB; closing transport",
                            )));
                        }
                    }
                }
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    #[tokio::test]
    async fn rejects_oversized_frames_but_allows_multiple_bounded_lines() {
        let oversized = vec![b'x'; MAX_REQUEST_BYTES + 1];
        let mut input = BoundedInput::new(oversized.as_slice());
        assert_eq!(
            input.read_to_end(&mut Vec::new()).await.unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        let mut lines = vec![b'x'; MAX_REQUEST_BYTES];
        lines.push(b'\n');
        lines.extend_from_slice(b"next\n");
        let mut input = BoundedInput::new(lines.as_slice());
        let mut output = Vec::new();
        input.read_to_end(&mut output).await.unwrap();
        assert_eq!(lines, output);
    }
}
