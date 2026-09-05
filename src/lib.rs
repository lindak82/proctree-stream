//! Build a process tree from a stream of start/exit events without
//! ever holding the full input in memory.
//!
//! Read events from any `BufRead` (a file, a pipe, stdin) through
//! [`EventReader`], applying each one to a [`ProcessTree`] as it
//! arrives. See the README for a runnable example and the log format.

pub mod event;
pub mod reader;
pub mod tree;

pub use event::{ParseError, ProcessEvent};
pub use reader::{EventReader, ReadError};
pub use tree::{ProcessNode, ProcessTree};
