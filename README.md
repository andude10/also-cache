# also-cache-rs (work in progress)

Low-overhead, low-latency replicated cache in Rust.

For the specific cases when you need consistent cache with high hit rates.

Features:

- (TODO) Peer-to-peer cache
- Highly available
- High hit rates (distributed S3-FIFO eviction strategy)
- (TODO) Fast cache recovery after node startup

Main goals:

- [Eventual consistency](https://en.wikipedia.org/wiki/Eventual_consistency)
- Low overhead
- Robustness and simplicity
- Small dependency tree
- Transparent API (inform the user of important implementation details)

### Storage

Currently cache is stored as binary objects of arbitrary size, each allocated on the heap. I was thinking it might be a performance concern because of heap fragmentation and no cache-locality. If it is the problem, the best solution I think is to implement a custom allocator, although it will require switching to nightly Rust (https://github.com/rust-lang/rust/issues/32838).

### P2P Concerns:

Cache is not consistent, but maybe it's good enough? But idea of library is to provide small quick cache, and it means frequent updates, so inconsistency becomes a fatal problem.
idea: initiate sync with database if node does not respond to heartbeat. Other recovery strategies.

Q: How cache is kept consistent with database? (biggest problem btw)
A: Each node is expected first to write to database, then to cache. (TODO) Cache entries can also have TTL, so eventually they will be consistent with database.

Q: What if a node goes down?
A: Each node sends heartbeat broadcasts to each node (TODO: improve with SWIM protocol?)

Q: How other nodes know when new node joins cluster?
A: New node sends heartbeat broadcast to all nodes, which then set it as alive (TODO: improve with SWIM protocol?).

Q: What if node receives two cache updates for the same object?
A: Each cache update has timestamp, the latest update wins (TODO: possible improvement: logical timestamps, Lamport Timestamps, some CRDTs)

Q: What if node crashes before it sends cache update to other nodes?
A: ¯\_(ツ)\_/¯ (maybe ensure broadcast is sent in each update/delete operations, but this introduces huge overhead?)

Q: How new node loads cache at start-up?
A: ¯\_(ツ)\_/¯

### References

The implementation is heavily inspired by:

- [quick_cache](https://github.com/arthurprs/quick-cache)
- [hiqlite](https://github.com/sebadob/hiqlite)
