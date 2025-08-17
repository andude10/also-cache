# also-cache-rs (work in progress)

Low-overhead, low-latency replicated cache in Rust.

Plug and play library for replicated cache. With also-cache-lib multiple instances of your server can share consistent in-memory cache. It is designed to be easy to use and to integrate into existing codebase, as well as migrate to another cache implementation if your needs overgrow also-cache-lib.

Features:

- Peer-to-peer distributed cache
- Cache-aside
- Fast cache recovery after node startup

Main goals:

- Low overhead
- Highly available
- Transparent API (inform the user of important implementation details)

### Storage

Currently cache is stored as binary objects of arbitrary size, each allocated on the heap. I was thinking it might be a performance concern because of heap fragmentation and no cache-locality. If it is the problem, the best solution I think is to implement a custom allocator, although it will require switching to nightly Rust (https://github.com/rust-lang/rust/issues/32838). I also had a quirky idea to place cache with mmap, this can be done with custom allocator as well. Discussion on why (not) use mmap: https://news.ycombinator.com/item?id=36563187

### P2P Concerns:

Cache is not consistent, but maybe it's good enough? But idea of library is to provide small quick cache, and it means frequent updates, so inconsistency becomes a fatal problem.
idea: initiate sync with database if node does not respond to heartbeat. Other recovery strategies.

Q: How cache is kept consistent with database? (biggest problem btw)
A: Each node is expected first to write to database, then to cache. Library intends to provide eventual consistency. (TODO: if server writes to database but fails to send cache update, cache becomes inconsistent. Use TTL or something better. But not sure if it's even possible to achieve 100% consistency without things like Raft or Paxos, but maybe it's good enough? )

Q: What if a node goes down?
A: Each node sends heartbeat broadcasts to each node (TODO: improve with SWIM protocol?)

Q: How other nodes know when new node joins cluster?
A: New node sends heartbeat broadcast to all nodes, which then set it as alive (TODO: improve with SWIM protocol?).

Q: What if node receives two cache updates for the same object?
A: Each cache update has timestamp, the latest update wins (TODO: possible improvement: logical timestamps, Lamport Timestamps, CRDTs)

Q: What if node crashes before it sends cache update to other nodes?
A: ¯\_(ツ)\_/¯ (maybe ensure broadcast is sent in each update/delete operations, but this introduces huge overhead?)

Q: How new node loads cache at start-up?
A: ¯\_(ツ)\_/¯

### References

The implementation is heavily inspired by:

- [quick_cache](https://github.com/arthurprs/quick-cache)
- [hiqlite](https://github.com/sebadob/hiqlite)
