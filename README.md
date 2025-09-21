# also-cache

A highly available replicated in-memory cache with high hit rates in Rust.

**WIP**. Right now only local cache is implemented, without the distributed part.

This cache is designed for scenarios where you want consistency and high hit rates in your distributed cluster. Instead of each node maintaining its own separate cache, all nodes send updates to each other, ensuring that frequently accessed data is available on all of them. It will also mean that cache will have (mostly) the same latency on each node.

Features:

- (WIP) Peer-to-peer cache
- Highly available
- High hit rates (distributed S3-FIFO eviction strategy)
- (WIP) Fast cache recovery after node startup
- (WIP) TTL for cache entries

Main goals:

- [Eventual consistency](https://en.wikipedia.org/wiki/Eventual_consistency)
- Low overhead
- Robustness and simplicity
- Small dependency tree
- Transparent API

### Getting started

- [examples](./examples) directory of this repo
- [benches](./benches) performance benchmarks (run with `cargo bench`).

### Implementation

Currently each cache entry is stored as raw bytes on the heap. It might be a performance concern because of many allocations and heap fragmentation.

### References

The implementation is heavily inspired by:

- [quick_cache](https://github.com/arthurprs/quick-cache). Low-overhead in-memory cache in Rust.
- [hiqlite](https://github.com/sebadob/hiqlite). Distributed SQLite + in-memory cache, implements Raft consensus algorithm which guarantees strong consistency.
