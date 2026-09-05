use std::fmt;
use std::num::ParseIntError;

/// A single line of process activity, as it would appear in an audit
/// trail or a periodic snapshot converted to one event per line.
///
/// Line format (space-separated, fixed field order):
///   `start <pid> <ppid> <ts> <name>`
///   `exit  <pid> <ts>`
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessEvent {
    Start {
        pid: u32,
        ppid: u32,
        ts: u64,
        name: String,
    },
    Exit {
        pid: u32,
        ts: u64,
    },
}

#[derive(Debug)]
pub enum ParseError {
    Empty,
    UnknownKind(String),
    MissingField(&'static str),
    BadNumber(ParseIntError),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Empty => write!(f, "empty line"),
            ParseError::UnknownKind(kind) => write!(f, "unknown event kind '{kind}'"),
            ParseError::MissingField(name) => write!(f, "missing field '{name}'"),
            ParseError::BadNumber(e) => write!(f, "bad number: {e}"),
        }
    }
}

impl std::error::Error for ParseError {}

impl From<ParseIntError> for ParseError {
    fn from(e: ParseIntError) -> Self {
        ParseError::BadNumber(e)
    }
}

impl ProcessEvent {
    /// Parse a single line. Blank lines are an error here rather than
    /// silently ignored, so the reader (which does skip them) is the
    /// only place that decides what to do with them.
    pub fn parse_line(line: &str) -> Result<Self, ParseError> {
        let mut fields = line.trim().split_whitespace();
        let kind = fields.next().ok_or(ParseError::Empty)?;
        match kind {
            "start" => {
                let pid = fields.next().ok_or(ParseError::MissingField("pid"))?.parse()?;
                let ppid = fields.next().ok_or(ParseError::MissingField("ppid"))?.parse()?;
                let ts = fields.next().ok_or(ParseError::MissingField("ts"))?.parse()?;
                let name = fields
                    .next()
                    .ok_or(ParseError::MissingField("name"))?
                    .to_string();
                Ok(ProcessEvent::Start { pid, ppid, ts, name })
            }
            "exit" => {
                let pid = fields.next().ok_or(ParseError::MissingField("pid"))?.parse()?;
                let ts = fields.next().ok_or(ParseError::MissingField("ts"))?.parse()?;
                Ok(ProcessEvent::Exit { pid, ts })
            }
            other => Err(ParseError::UnknownKind(other.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_start() {
        let event = ProcessEvent::parse_line("start 42 1 100 sshd").unwrap();
        assert_eq!(
            event,
            ProcessEvent::Start { pid: 42, ppid: 1, ts: 100, name: "sshd".to_string() }
        );
    }

    #[test]
    fn parses_exit() {
        let event = ProcessEvent::parse_line("exit 42 200").unwrap();
        assert_eq!(event, ProcessEvent::Exit { pid: 42, ts: 200 });
    }

    #[test]
    fn rejects_unknown_kind() {
        assert!(matches!(
            ProcessEvent::parse_line("fork 1 2 3"),
            Err(ParseError::UnknownKind(_))
        ));
    }
}
