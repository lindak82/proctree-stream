use std::collections::HashMap;

use crate::event::ProcessEvent;

/// One process as recorded by the tree. Nodes reference each other by
/// index into `ProcessTree`'s arena rather than by pointer, so the
/// whole structure lives in a single Vec with no Rc/RefCell bookkeeping.
#[derive(Debug, Clone)]
pub struct ProcessNode {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
    pub start_ts: u64,
    pub exit_ts: Option<u64>,
    pub children: Vec<usize>,
}

impl ProcessNode {
    pub fn is_running(&self) -> bool {
        self.exit_ts.is_none()
    }
}

/// Builds a process tree incrementally from a stream of events.
///
/// Feed events one at a time with [`apply`](ProcessTree::apply); the
/// caller never needs to hold the source log in memory, only whatever
/// it reads one line at a time (see [`crate::reader::EventReader`]).
/// Memory used by the tree itself grows with the number of processes
/// recorded, not with the size of the input stream.
#[derive(Debug, Default)]
pub struct ProcessTree {
    nodes: Vec<ProcessNode>,
    // pid -> index of the currently live node with that pid. Removed
    // on exit so a reused pid starts a fresh entry instead of being
    // confused with its predecessor.
    live: HashMap<u32, usize>,
    roots: Vec<usize>,
}

impl ProcessTree {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply(&mut self, event: ProcessEvent) {
        match event {
            ProcessEvent::Start { pid, ppid, ts, name } => self.start(pid, ppid, ts, name),
            ProcessEvent::Exit { pid, ts } => self.exit(pid, ts),
        }
    }

    fn start(&mut self, pid: u32, ppid: u32, ts: u64, name: String) {
        let index = self.nodes.len();
        self.nodes.push(ProcessNode {
            pid,
            ppid,
            name,
            start_ts: ts,
            exit_ts: None,
            children: Vec::new(),
        });

        match self.live.get(&ppid) {
            Some(&parent_index) => self.nodes[parent_index].children.push(index),
            // Parent not currently live (log started mid-stream, or
            // this really is pid 0/1): treat it as a root.
            None => self.roots.push(index),
        }

        self.live.insert(pid, index);
    }

    fn exit(&mut self, pid: u32, ts: u64) {
        if let Some(index) = self.live.remove(&pid) {
            self.nodes[index].exit_ts = Some(ts);
        }
        // An exit for a pid we never saw start is dropped rather than
        // treated as an error: the log may not begin at boot.
    }

    pub fn node(&self, index: usize) -> &ProcessNode {
        &self.nodes[index]
    }

    pub fn roots(&self) -> &[usize] {
        &self.roots
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_parent_child_link() {
        let mut tree = ProcessTree::new();
        tree.apply(ProcessEvent::Start { pid: 1, ppid: 0, ts: 0, name: "init".into() });
        tree.apply(ProcessEvent::Start { pid: 42, ppid: 1, ts: 5, name: "sshd".into() });

        assert_eq!(tree.roots(), &[0]);
        assert_eq!(tree.node(0).children, vec![1]);
        assert_eq!(tree.node(1).pid, 42);
    }

    #[test]
    fn exit_marks_node_without_removing_it() {
        let mut tree = ProcessTree::new();
        tree.apply(ProcessEvent::Start { pid: 1, ppid: 0, ts: 0, name: "init".into() });
        tree.apply(ProcessEvent::Exit { pid: 1, ts: 10 });

        assert_eq!(tree.len(), 1);
        assert_eq!(tree.node(0).exit_ts, Some(10));
        assert!(!tree.node(0).is_running());
    }

    #[test]
    fn reused_pid_starts_a_fresh_node() {
        let mut tree = ProcessTree::new();
        tree.apply(ProcessEvent::Start { pid: 1, ppid: 0, ts: 0, name: "init".into() });
        tree.apply(ProcessEvent::Start { pid: 7, ppid: 1, ts: 1, name: "sh".into() });
        tree.apply(ProcessEvent::Exit { pid: 7, ts: 2 });
        tree.apply(ProcessEvent::Start { pid: 7, ppid: 1, ts: 3, name: "curl".into() });

        assert_eq!(tree.len(), 3);
        assert_eq!(tree.node(0).children, vec![1, 2]);
        assert_eq!(tree.node(2).name, "curl");
        assert!(tree.node(2).is_running());
    }
}
