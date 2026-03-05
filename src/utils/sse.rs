//! Utilities to deal with SSE (Server-Sent Events).

use async_stream::stream;
use futures::Stream;

const EVENT_TERMINATOR: &[u8] = b"\n\n";

/// Split from the last SSE event terminator.
///
/// On the left side you will get the side of the buffer that contains completed messages.
/// Although, the last terminator has been removed, this side may still contain multiple
/// messages that need to be split.
///
/// On the right side you will get the side of the buffer that contains the incomplete data,
/// so you should keep this on the buffer until completed.
///
/// Returns `None` if no terminator is found.
///
/// This splitter handles LF-only delimiters. Normalize CRLF before calling it.
fn rsplit_once_terminator(buffer: &[u8]) -> Option<(&[u8], &[u8])> {
    let pos = buffer
        .windows(EVENT_TERMINATOR.len())
        .enumerate()
        .rev()
        .find(|(_, w)| *w == EVENT_TERMINATOR)
        .map(|(pos, _)| pos)?;

    let (before, after_with_terminator) = buffer.split_at(pos);
    let after = &after_with_terminator[EVENT_TERMINATOR.len()..];
    Some((before, after))
}

fn extract_sse_data(message: &str) -> Option<String> {
    let mut data_lines = Vec::new();

    for line in message.lines() {
        if line.starts_with(':') {
            continue;
        }

        let Some((field, value)) = line.split_once(':') else {
            continue;
        };

        if field.trim() == "data" {
            let value = value.strip_prefix(' ').unwrap_or(value);
            data_lines.push(value);
        }
    }

    if data_lines.is_empty() {
        return None;
    }

    let data = data_lines.join("\n");
    if data.trim() == "[DONE]" {
        return None;
    }

    Some(data)
}

/// Convert a stream of bytes into a stream of SSE messages.
pub fn parse_sse<S, B, E>(s: S) -> impl Stream<Item = Result<String, E>>
where
    S: Stream<Item = Result<B, E>>,
    B: AsRef<[u8]>,
{
    stream! {
        let event_terminator_str = std::str::from_utf8(EVENT_TERMINATOR).unwrap();
        let mut buffer: Vec<u8> = Vec::new();

        for await chunk in s {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(error) => {
                    yield Err(error);
                    return;
                }
            };

            let chunk = chunk.as_ref();

            buffer.extend_from_slice(chunk);
            buffer.retain(|&b| b != b'\r');

            let Some((completed_messages, incomplete_message)) =
                rsplit_once_terminator(&buffer)
            else {
                continue;
            };

            // Silently drop any invalid utf8 bytes from the completed messages.
            let completed_messages = String::from_utf8_lossy(completed_messages);

            let messages = completed_messages
                .split(event_terminator_str)
                .filter_map(extract_sse_data);

            for m in messages {
                yield Ok(m.to_string());
            }

            buffer = incomplete_message.to_vec();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{StreamExt, executor::block_on};

    #[test]
    fn test_rsplit_once_terminator() {
        let buffer = b"data: 1\n\ndata: 2\n\ndata: incomplete mes";
        let (completed, incomplete) = rsplit_once_terminator(buffer).unwrap();
        assert_eq!(completed, b"data: 1\n\ndata: 2");
        assert_eq!(incomplete, b"data: incomplete mes");
    }

    #[test]
    fn test_extract_sse_data_ignores_non_data_event() {
        let message = "event: ping\nid: 1";
        assert_eq!(extract_sse_data(message), None);
    }

    #[test]
    fn test_extract_sse_data_with_data_field() {
        let message = "event: message\ndata: {\"ok\":true}";
        assert_eq!(extract_sse_data(message), Some("{\"ok\":true}".to_string()));
    }

    #[test]
    fn test_parse_sse_skips_non_data_event() {
        let input = futures::stream::iter(vec![Ok::<_, ()>(
            b"event: ping\n\n\
              data: hello\n\n"
                .to_vec(),
        )]);

        let mut output = std::pin::pin!(parse_sse(input));
        let first = block_on(output.next());
        let second = block_on(output.next());

        assert_eq!(first, Some(Ok("hello".to_string())));
        assert_eq!(second, None);
    }

    #[test]
    fn test_parse_sse_with_crlf_terminator() {
        let input = futures::stream::iter(vec![Ok::<_, ()>(
            b"data: first\r\n\r\ndata: second\r\n\r\n".to_vec(),
        )]);

        let mut output = std::pin::pin!(parse_sse(input));
        let first = block_on(output.next());
        let second = block_on(output.next());
        let third = block_on(output.next());

        assert_eq!(first, Some(Ok("first".to_string())));
        assert_eq!(second, Some(Ok("second".to_string())));
        assert_eq!(third, None);
    }
}
