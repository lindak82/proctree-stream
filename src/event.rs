use std::fmt;
use std::num::ParseIntError;

/// A single line of process activity, as it would appear in an audit
/// trail or a periodic snapshot converted to one event per line.
///
/// Line format (space-separated, fixed field order):
///   `start <pid> <ppid> <ts> <name>`
///   `exit  <pid> <ts>`
///
/// A name with whitespace, quotes or backslashes (or an empty name) is
/// written in double quotes, with `\"`, `\\`, `\n`, `\r` and `\t` as
/// escapes. A name that does not start with a quote is a single bare
/// token, as before.
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
    BadName(&'static str),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Empty => write!(f, "empty line"),
            ParseError::UnknownKind(kind) => write!(f, "unknown event kind '{kind}'"),
            ParseError::MissingField(name) => write!(f, "missing field '{name}'"),
            ParseError::BadNumber(e) => write!(f, "bad number: {e}"),
            ParseError::BadName(why) => write!(f, "bad name: {why}"),
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
        let mut rest = line.trim();
        let kind = next_token(&mut rest).ok_or(ParseError::Empty)?;
        match kind {
            "start" => {
                let pid = next_token(&mut rest).ok_or(ParseError::MissingField("pid"))?.parse()?;
                let ppid = next_token(&mut rest).ok_or(ParseError::MissingField("ppid"))?.parse()?;
                let ts = next_token(&mut rest).ok_or(ParseError::MissingField("ts"))?.parse()?;
                let name = parse_name(rest)?;
                Ok(ProcessEvent::Start { pid, ppid, ts, name })
            }
            "exit" => {
                let pid = next_token(&mut rest).ok_or(ParseError::MissingField("pid"))?.parse()?;
                let ts = next_token(&mut rest).ok_or(ParseError::MissingField("ts"))?.parse()?;
                Ok(ProcessEvent::Exit { pid, ts })
            }
            other => Err(ParseError::UnknownKind(other.to_string())),
        }
    }
}

/// Split off the next whitespace-delimited token, leaving the rest
/// (with leading whitespace removed) in `rest`. Needed instead of
/// `split_whitespace` because the name field may need the raw remainder.
fn next_token<'a>(rest: &mut &'a str) -> Option<&'a str> {
    let s = rest.trim_start();
    if s.is_empty() {
        *rest = s;
        return None;
    }
    let end = s.find(char::is_whitespace).unwrap_or(s.len());
    let (token, tail) = s.split_at(end);
    *rest = tail.trim_start();
    Some(token)
}

fn parse_name(rest: &str) -> Result<String, ParseError> {
    let Some(quoted) = rest.strip_prefix('"') else {
        let mut rest = rest;
        return next_token(&mut rest)
            .map(str::to_string)
            .ok_or(ParseError::MissingField("name"));
    };
    let mut name = String::new();
    let mut chars = quoted.chars();
    loop {
        match chars.next() {
            None => return Err(ParseError::BadName("unterminated quote")),
            Some('"') => break,
            Some('\\') => match chars.next() {
                Some('"') => name.push('"'),
                Some('\\') => name.push('\\'),
                Some('n') => name.push('\n'),
                Some('r') => name.push('\r'),
                Some('t') => name.push('\t'),
                _ => return Err(ParseError::BadName("unknown escape")),
            },
            Some(c) => name.push(c),
        }
    }
    if !chars.as_str().trim().is_empty() {
        return Err(ParseError::BadName("text after closing quote"));
    }
    Ok(name)
}

fn needs_quoting(name: &str) -> bool {
    name.is_empty() || name.chars().any(|c| c.is_whitespace() || c == '"' || c == '\\')
}

/// Writes the event in the same format `parse_line` reads, so a name
/// survives a round trip.
impl fmt::Display for ProcessEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProcessEvent::Exit { pid, ts } => write!(f, "exit {pid} {ts}"),
            ProcessEvent::Start { pid, ppid, ts, name } => {
                write!(f, "start {pid} {ppid} {ts} ")?;
                if !needs_quoting(name) {
                    return f.write_str(name);
                }
                f.write_str("\"")?;
                for c in name.chars() {
                    match c {
                        '"' => f.write_str("\\\"")?,
                        '\\' => f.write_str("\\\\")?,
                        '\n' => f.write_str("\\n")?,
                        '\r' => f.write_str("\\r")?,
                        '\t' => f.write_str("\\t")?,
                        c => write!(f, "{c}")?,
                    }
                }
                f.write_str("\"")
            }
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
    fn parses_quoted_name_with_spaces() {
        let event = ProcessEvent::parse_line(r#"start 7 1 9 "python my script.py""#).unwrap();
        assert_eq!(
            event,
            ProcessEvent::Start { pid: 7, ppid: 1, ts: 9, name: "python my script.py".to_string() }
        );
    }

    #[test]
    fn rejects_malformed_quotes() {
        for line in [
            r#"start 7 1 9 "open"#,
            r#"start 7 1 9 "a" b"#,
            r#"start 7 1 9 "bad\q""#,
        ] {
            assert!(matches!(ProcessEvent::parse_line(line), Err(ParseError::BadName(_))), "{line}");
        }
    }

    #[test]
    fn display_round_trips_awkward_names() {
        for name in ["sshd", "", "a b", "say \"hi\"", "back\\slash", "tab\tand\nnewline", "\"lead"] {
            let event = ProcessEvent::Start { pid: 1, ppid: 0, ts: 2, name: name.to_string() };
            assert_eq!(ProcessEvent::parse_line(&event.to_string()).unwrap(), event, "{name:?}");
        }
    }

    #[test]
    fn bare_name_is_not_quoted_on_output() {
        let event = ProcessEvent::Start { pid: 1, ppid: 0, ts: 2, name: "init".to_string() };
        assert_eq!(event.to_string(), "start 1 0 2 init");
    }

    #[test]
    fn rejects_unknown_kind() {
        assert!(matches!(
            ProcessEvent::parse_line("fork 1 2 3"),
            Err(ParseError::UnknownKind(_))
        ));
    }
}
