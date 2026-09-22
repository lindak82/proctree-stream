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
/// recorded, not with the size of the input stream — and, once
/// [`prune_finished`](ProcessTree::prune_finished) is used, only with
/// the number *currently* recorded, since fully-finished subtrees can
/// be dropped as the stream progresses.
#[derive(Debug, Default)]
pub struct ProcessTree {
    // `None` marks a slot freed by pruning; the index is recycled by
    // `start` via `free` rather than left as a permanent hole, so the
    // arena doesn't grow without bound on a long-running stream that
    // gets pruned regularly.
    nodes: Vec<Option<ProcessNode>>,
    free: Vec<usize>,
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
        let node = ProcessNode {
            pid,
            ppid,
            name,
            start_ts: ts,
            exit_ts: None,
            children: Vec::new(),
        };

        let index = match self.free.pop() {
            Some(index) => {
                self.nodes[index] = Some(node);
                index
            }
            None => {
                self.nodes.push(Some(node));
                self.nodes.len() - 1
            }
        };

        match self.live.get(&ppid) {
            Some(&parent_index) => self.nodes[parent_index]
                .as_mut()
                .expect("live index always points at an unpruned node")
                .children
                .push(index),
            // Parent not currently live (log started mid-stream, or
            // this really is pid 0/1): treat it as a root.
            None => self.roots.push(index),
        }

        self.live.insert(pid, index);
    }

    fn exit(&mut self, pid: u32, ts: u64) {
        if let Some(index) = self.live.remove(&pid) {
            self.nodes[index]
                .as_mut()
                .expect("live index always points at an unpruned node")
                .exit_ts = Some(ts);
        }
        // An exit for a pid we never saw start is dropped rather than
        // treated as an error: the log may not begin at boot.
    }

    /// Drop every subtree that has fully finished: the process itself
    /// has exited and every descendant has too (recursively — a
    /// subtree with even one still-running leaf is left alone). Once
    /// a node has exited it can never gain new children (`start`
    /// looks it up in `live`, which no longer holds it), so a
    /// finished subtree will never change again and is safe to
    /// discard.
    ///
    /// Returns the number of nodes freed. Call this periodically
    /// while consuming a long-running stream to keep memory bounded
    /// by the number of processes *currently* live plus whatever
    /// finished subtrees haven't been pruned yet, rather than by
    /// every process the log has ever mentioned.
    ///
    /// Pruning invalidates any index into a removed subtree; only
    /// indices still reachable from [`roots`](ProcessTree::roots) or
    /// a node's `children` after the call are safe to use.
    pub fn prune_finished(&mut self) -> usize {
        let roots = std::mem::take(&mut self.roots);
        let mut remaining_roots = Vec::with_capacity(roots.len());
        let mut pruned = 0;

        for root in roots {
            if self.prune_node(root, &mut pruned) {
                continue;
            }
            remaining_roots.push(root);
        }

        self.roots = remaining_roots;
        pruned
    }

    /// Post-order: prune finished children first, then decide whether
    /// this node itself is now finished (exited, and left with no
    /// children because it had none or they were all just pruned).
    /// Returns whether `index` was pruned.
    fn prune_node(&mut self, index: usize, pruned: &mut usize) -> bool {
        let children = self.nodes[index]
            .as_ref()
            .expect("index reachable from roots/children is never pruned")
            .children
            .clone();

        let mut remaining = Vec::with_capacity(children.len());
        for child in children {
            if !self.prune_node(child, pruned) {
                remaining.push(child);
            }
        }

        let node = self.nodes[index].as_mut().expect("checked above");
        node.children = remaining;

        if node.exit_ts.is_some() && node.children.is_empty() {
            self.nodes[index] = None;
            self.free.push(index);
            *pruned += 1;
            true
        } else {
            false
        }
    }

    pub fn node(&self, index: usize) -> &ProcessNode {
        self.nodes[index]
            .as_ref()
            .expect("node index is stale: it was pruned by prune_finished")
    }

    pub fn roots(&self) -> &[usize] {
        &self.roots
    }

    /// Number of process nodes currently held in memory: started, and
    /// not yet dropped by [`prune_finished`](ProcessTree::prune_finished).
    pub fn len(&self) -> usize {
        self.nodes.len() - self.free.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
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

    #[test]
    fn prune_drops_a_fully_finished_subtree() {
        let mut tree = ProcessTree::new();
        tree.apply(ProcessEvent::Start { pid: 1, ppid: 0, ts: 0, name: "init".into() });
        tree.apply(ProcessEvent::Start { pid: 2, ppid: 1, ts: 1, name: "bash".into() });
        tree.apply(ProcessEvent::Start { pid: 3, ppid: 2, ts: 2, name: "curl".into() });
        tree.apply(ProcessEvent::Exit { pid: 3, ts: 3 });
        tree.apply(ProcessEvent::Exit { pid: 2, ts: 4 });

        // init (pid 1) is still running, so nothing under it is
        // finished yet even though bash and curl both exited.
        assert_eq!(tree.prune_finished(), 0);
        assert_eq!(tree.len(), 3);

        tree.apply(ProcessEvent::Exit { pid: 1, ts: 5 });
        assert_eq!(tree.prune_finished(), 3);
        assert_eq!(tree.len(), 0);
        assert!(tree.roots().is_empty());
    }

    #[test]
    fn prune_leaves_running_leaves_and_their_ancestors() {
        let mut tree = ProcessTree::new();
        tree.apply(ProcessEvent::Start { pid: 1, ppid: 0, ts: 0, name: "init".into() });
        tree.apply(ProcessEvent::Start { pid: 2, ppid: 1, ts: 1, name: "bash".into() });
        tree.apply(ProcessEvent::Start { pid: 3, ppid: 2, ts: 2, name: "curl".into() });
        tree.apply(ProcessEvent::Exit { pid: 3, ts: 3 });
        // bash (pid 2) never exits: its subtree, and everything above
        // it up to the root, must survive pruning.

        assert_eq!(tree.prune_finished(), 1);
        assert_eq!(tree.len(), 2);
        assert_eq!(tree.roots(), &[0]);
        assert_eq!(tree.node(0).children, vec![1]);
        assert!(tree.node(1).children.is_empty());
    }

    #[test]
    fn freed_slots_are_recycled_by_later_starts() {
        let mut tree = ProcessTree::new();
        tree.apply(ProcessEvent::Start { pid: 1, ppid: 0, ts: 0, name: "a".into() });
        tree.apply(ProcessEvent::Exit { pid: 1, ts: 1 });
        tree.prune_finished();
        assert_eq!(tree.len(), 0);

        tree.apply(ProcessEvent::Start { pid: 2, ppid: 0, ts: 2, name: "b".into() });
        assert_eq!(tree.len(), 1);
        assert_eq!(tree.node(0).name, "b");
    }
}
