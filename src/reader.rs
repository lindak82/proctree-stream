use std::fmt;
use std::io::{self, BufRead};

use crate::event::{ParseError, ProcessEvent};

/// Reads events from any buffered source one line at a time.
///
/// This is the piece that keeps memory bounded on the input side:
/// each call reads and parses a single line into a reused buffer, so
/// a multi-gigabyte log costs one line's worth of memory, never the
/// whole file.
pub struct EventReader<R> {
    inner: R,
    line: String,
    line_no: usize,
}

impl<R: BufRead> EventReader<R> {
    pub fn new(inner: R) -> Self {
        Self { inner, line: String::new(), line_no: 0 }
    }

    /// Read and parse the next event, or `None` at end of input.
    /// Blank lines are skipped rather than surfaced as errors, since
    /// they turn up naturally at the end of a file.
    pub fn next_event(&mut self) -> Option<Result<ProcessEvent, ReadError>> {
        loop {
            self.line.clear();
            match self.inner.read_line(&mut self.line) {
                Ok(0) => return None,
                Ok(_) => {}
                Err(e) => return Some(Err(ReadError::Io(e))),
            }
            self.line_no += 1;

            if self.line.trim().is_empty() {
                continue;
            }

            return Some(
                ProcessEvent::parse_line(&self.line)
                    .map_err(|source| ReadError::Parse { line: self.line_no, source }),
            );
        }
    }
}

impl<R: BufRead> Iterator for EventReader<R> {
    type Item = Result<ProcessEvent, ReadError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_event()
    }
}

#[derive(Debug)]
pub enum ReadError {
    Io(io::Error),
    Parse { line: usize, source: ParseError },
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadError::Io(e) => write!(f, "io error: {e}"),
            ReadError::Parse { line, source } => write!(f, "line {line}: {source}"),
        }
    }
}

impl std::error::Error for ReadError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn reads_events_and_skips_blank_lines() {
        let input = "start 1 0 0 init\n\nstart 2 1 5 sshd\nexit 2 10\n";
        let mut reader = EventReader::new(Cursor::new(input));

        assert!(matches!(reader.next_event(), Some(Ok(ProcessEvent::Start { pid: 1, .. }))));
        assert!(matches!(reader.next_event(), Some(Ok(ProcessEvent::Start { pid: 2, .. }))));
        assert!(matches!(reader.next_event(), Some(Ok(ProcessEvent::Exit { pid: 2, .. }))));
        assert!(reader.next_event().is_none());
    }

    #[test]
    fn reports_line_number_on_bad_input() {
        let input = "start 1 0 0 init\nnonsense\n";
        let mut reader = EventReader::new(Cursor::new(input));

        reader.next_event().unwrap().unwrap();
        match reader.next_event() {
            Some(Err(ReadError::Parse { line, .. })) => assert_eq!(line, 2),
            other => panic!("expected a parse error on line 2, got {other:?}"),
        }
    }
}
