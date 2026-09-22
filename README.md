# proctree

Reconstruct process trees (who spawned whom, and when) from a log of
start/exit events, without loading the log into memory first.

## The problem

If you're looking at process lineage from an audit trail, EDR export,
or your own instrumentation, the usual approach is: read the whole
file, parse every line into a list, then walk the list building a
tree. That's fine for a quick script against a small file. It falls
over once the log covers a busy host for days or weeks — you end up
holding gigabytes of raw lines in memory just to produce a tree that's
a small fraction of that size.

`proctree` processes the log one line at a time. Memory use tracks the
number of processes recorded, not the number of bytes read.

## Log format

One event per line, space-separated:

```
start <pid> <ppid> <ts> <name>
exit  <pid> <ts>
```

```
start 1 0 0 init
start 42 1 5 sshd
start 731 42 12 bash
exit 731 40
start 900 731 41 curl
exit 900 42
```

Timestamps are opaque `u64`s — seconds, milliseconds, whatever your
source uses, as long as they're comparable.

## Usage

```rust
use std::fs::File;
use std::io::BufReader;
use proctree::{EventReader, ProcessTree};

fn main() -> std::io::Result<()> {
    let file = File::open("boot.log")?;
    let mut reader = EventReader::new(BufReader::new(file));
    let mut tree = ProcessTree::new();

    while let Some(event) = reader.next_event() {
        match event {
            Ok(event) => tree.apply(event),
            Err(e) => eprintln!("skipping bad line: {e}"),
        }
    }

    for &root in tree.roots() {
        print_subtree(&tree, root, 0);
    }
    Ok(())
}

fn print_subtree(tree: &ProcessTree, index: usize, depth: usize) {
    let node = tree.node(index);
    let status = if node.is_running() { "" } else { " [exited]" };
    println!("{}{} (pid {}){status}", "  ".repeat(depth), node.name, node.pid);
    for &child in &node.children {
        print_subtree(tree, child, depth + 1);
    }
}
```

`EventReader` reads and discards one line at a time via `BufRead`, so
`File` is never fully buffered. `ProcessTree` stores nodes in a flat
arena and links them by index (pid -> index of the currently live
process with that pid), which also means a reused pid doesn't get
confused with its predecessor.

## Pruning finished subtrees

The tree keeps every process it has seen until you tell it to let go.
Call `ProcessTree::prune_finished` periodically while consuming a
long-running stream and it will drop any subtree where the process and
every descendant have exited — freeing the names and child lists, and
recycling the slot for a future `start`. A process that runs for the
life of the log is never pruned (its subtree is never "finished"), but
short-lived subtrees under it no longer sit around forever:

```rust
let mut since_prune = 0;
while let Some(event) = reader.next_event() {
    if let Ok(event) = event {
        tree.apply(event);
    }
    since_prune += 1;
    if since_prune >= 10_000 {
        tree.prune_finished();
        since_prune = 0;
    }
}
```

Pruning invalidates indices into whatever it removed, so only treat an
index as valid if you got it from `roots()` or a node's `children`
after the most recent prune.

## What this doesn't do (yet)

Process names are split on whitespace with no escaping, so a name
containing a space (rare, but real for some interpreters and scripts)
will be truncated at the first space and the rest silently dropped as
an extra field.

## License

MIT, see `LICENSE`.
