//! An incremental `text/event-stream` parser (WHATWG HTML §9.2.6).
//!
//! Bytes arrive in arbitrary chunks; [`SseParser::feed`] returns the events
//! completed so far. Lines end in LF, CRLF or CR (a CR at the end of a chunk
//! waits for the next one to see whether an LF follows); `data:` lines are
//! joined with `\n`; comments (`:`) and `retry:` are ignored.

/// One dispatched event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    /// The `event:` field, or `"message"`.
    pub event: String,
    /// The joined `data:` lines.
    pub data: String,
    /// The last `id:` seen, if any.
    pub id: Option<String>,
}

/// The stream exceeded the parser's size limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("an event stream line or event is larger than {0} bytes")]
pub struct SseTooLarge(pub usize);

/// Parser state between chunks.
#[derive(Debug)]
pub struct SseParser {
    line: Vec<u8>,
    pending_cr: bool,
    started: bool,
    data: String,
    has_data: bool,
    event: String,
    last_id: Option<String>,
    retry_ms: Option<u64>,
    max: usize,
}

impl SseParser {
    /// A parser that refuses lines or events over `max` bytes.
    pub fn new(max: usize) -> SseParser {
        SseParser {
            line: Vec::new(),
            pending_cr: false,
            started: false,
            data: String::new(),
            has_data: false,
            event: String::new(),
            last_id: None,
            retry_ms: None,
            max,
        }
    }

    /// Consumes a chunk and returns the events it completed.
    ///
    /// # Errors
    /// [`SseTooLarge`] when a line or event grows past the limit.
    pub fn feed(&mut self, chunk: &[u8]) -> Result<Vec<SseEvent>, SseTooLarge> {
        let mut events = Vec::new();
        for &byte in chunk {
            if self.pending_cr {
                self.pending_cr = false;
                if byte == b'\n' {
                    continue; // CRLF: the line already ended at CR.
                }
            }
            match byte {
                b'\n' => self.end_line(&mut events)?,
                b'\r' => {
                    self.end_line(&mut events)?;
                    self.pending_cr = true;
                }
                _ => {
                    if self.line.len() >= self.max {
                        return Err(SseTooLarge(self.max));
                    }
                    self.line.push(byte);
                }
            }
        }
        Ok(events)
    }

    fn end_line(&mut self, events: &mut Vec<SseEvent>) -> Result<(), SseTooLarge> {
        let mut raw = std::mem::take(&mut self.line);
        if !self.started {
            self.started = true;
            if raw.starts_with(&[0xEF, 0xBB, 0xBF]) {
                raw.drain(..3);
            }
        }
        let line = String::from_utf8_lossy(&raw);
        if line.is_empty() {
            if self.has_data {
                events.push(SseEvent {
                    event: if self.event.is_empty() {
                        "message".into()
                    } else {
                        std::mem::take(&mut self.event)
                    },
                    data: std::mem::take(&mut self.data),
                    id: self.last_id.clone(),
                });
            }
            self.data.clear();
            self.event.clear();
            self.has_data = false;
            return Ok(());
        }
        if line.starts_with(':') {
            return Ok(());
        }
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line.as_ref(), ""),
        };
        match field {
            "data" => {
                if self.has_data {
                    self.data.push('\n');
                }
                self.data.push_str(value);
                self.has_data = true;
                if self.data.len() > self.max {
                    return Err(SseTooLarge(self.max));
                }
            }
            "event" => self.event = value.to_owned(),
            "id" if !value.contains('\0') => self.last_id = Some(value.to_owned()),
            "retry" if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) => {
                self.retry_ms = value.parse().ok();
            }
            _ => {}
        }
        Ok(())
    }

    /// The last event id seen, for `Last-Event-ID` when resuming.
    pub fn last_event_id(&self) -> Option<&str> {
        self.last_id.as_deref()
    }

    /// The reconnection delay the server asked for, in milliseconds.
    pub fn retry_ms(&self) -> Option<u64> {
        self.retry_ms
    }

    /// Starts reading a resumed stream: a half-read line or event of the old
    /// one is dropped; the last event id and retry delay are kept.
    pub fn next_stream(&mut self) {
        self.line.clear();
        self.pending_cr = false;
        self.started = false;
        self.data.clear();
        self.has_data = false;
        self.event.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn ev(data: &str) -> SseEvent {
        SseEvent {
            event: "message".into(),
            data: data.into(),
            id: None,
        }
    }

    fn feed_all(chunks: &[&[u8]]) -> Vec<SseEvent> {
        let mut p = SseParser::new(1 << 20);
        chunks.iter().flat_map(|c| p.feed(c).unwrap()).collect()
    }

    #[test]
    fn one_event_in_one_chunk() {
        assert_eq!(feed_all(&[b"data: {\"a\":1}\n\n"]), vec![ev("{\"a\":1}")]);
    }

    #[test]
    fn events_split_at_every_byte_boundary() {
        let stream = b"event: message\nid: 7\ndata: {\"jsonrpc\":\"2.0\",\ndata: \"id\":1}\n\n: keep-alive\n\ndata: second\r\n\r\n";
        let whole = feed_all(&[stream]);
        assert_eq!(whole.len(), 2);
        assert_eq!(whole[0].data, "{\"jsonrpc\":\"2.0\",\n\"id\":1}");
        assert_eq!(whole[0].id.as_deref(), Some("7"));
        assert_eq!(whole[1].data, "second");
        for split in 1..stream.len() {
            let (a, b) = stream.split_at(split);
            assert_eq!(feed_all(&[a, b]), whole, "split at {split}");
        }
        let bytes: Vec<&[u8]> = stream.chunks(1).collect();
        assert_eq!(feed_all(&bytes), whole);
    }

    #[test]
    fn cr_only_line_endings_and_a_cr_at_a_chunk_end() {
        assert_eq!(feed_all(&[b"data: a\r", b"\r"]), vec![ev("a")]);
        assert_eq!(
            feed_all(&[b"data: a\r", b"\ndata: b\r\n\r\n"]),
            vec![ev("a\nb")]
        );
    }

    #[test]
    fn comments_retry_and_empty_events_dispatch_nothing() {
        assert!(feed_all(&[b": ping\n\nretry: 100\n\nevent: x\n\n"]).is_empty());
    }

    #[test]
    fn a_leading_bom_and_a_field_without_a_colon() {
        assert_eq!(feed_all(&[b"\xEF\xBB\xBFdata\n\n"]), vec![ev("")]);
    }

    #[test]
    fn an_unterminated_event_is_not_dispatched() {
        assert!(feed_all(&[b"data: half"]).is_empty());
    }

    #[test]
    fn retry_and_the_last_id_survive_into_a_resumed_stream() {
        let mut p = SseParser::new(1 << 20);
        assert!(
            p.feed(b"id: ev-1\nretry: 250\ndata:\n\nretry: x\n\ndata: half")
                .unwrap()
                .iter()
                .all(|e| e.data.is_empty())
        );
        assert_eq!(p.last_event_id(), Some("ev-1"));
        assert_eq!(p.retry_ms(), Some(250));
        p.next_stream();
        assert_eq!(
            p.feed(b"data: whole\n\n").unwrap(),
            vec![SseEvent {
                event: "message".into(),
                data: "whole".into(),
                id: Some("ev-1".into()),
            }]
        );
    }

    #[test]
    fn oversized_lines_are_refused() {
        let mut p = SseParser::new(8);
        assert_eq!(p.feed(b"data: 0123456789"), Err(SseTooLarge(8)));
    }
}
