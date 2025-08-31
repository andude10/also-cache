use std::hash::BuildHasher;
use std::{hash::Hash, marker::PhantomData};

use hashbrown::HashTable;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QueueTypeId {
    NoQueue,
    Small,
    Main,
    Ghost,
}

// Cache entry, stores the actual data as bytes on the heap
#[derive(Debug, Clone)]
struct Node {
    data: Vec<u8>,
    weight: u64,

    // single cache shard would never have more than 2^32 nodes,
    // so we use u32 for indexes to reduce memory usage a little
    next: u32,
    prev: u32,

    freq: u8,
    queue: QueueTypeId,
}

#[derive(Debug)]
pub struct CacheShard<Key, B> {
    map: HashTable<u32>,
    nodes_keys: Vec<Key>,
    hasher: B,

    nodes: Vec<Node>,
    freelist: Vec<NodeRef<NoQueue, Free>>,

    // if size of queue is more then threshold, next
    // insert will cause eviction in that queue
    small_size: u64,
    main_size: u64,
    ghost_size: u64,
    small_threshold: u64,
    main_threshold: u64,
    ghost_threshold: u64,

    small_head: QueueHead<SmallQueue>,
    main_head: QueueHead<MainQueue>,
    ghost_head: QueueHead<GhostQueue>,
}

// This represents a reference to a node in a Vec<Node>. Nodes can be in different states:
// occupied or free, and part of some queue or not. To make node management easier and safer,
// NodeRef uses phantom types to track the assumed state of the node at compile time.
//
// So for example, if a function takes NodeRef<NoQueue, Occupied>, it means that function
// assumes that this node is not part of any queue and is occupied (not freed).
//
// Q: Queue type - SmallQueue, MainQueue, GhostQueue, or NoQueue if not in any queue
// H: Node state - Occupied (has data) or Free (available for reuse)
#[derive(Debug)]
pub struct NodeRef<Q, H> {
    idx: u32,
    _occupied: PhantomData<H>,
    _queue: PhantomData<Q>,
}

// Type-level markers for NodeRef
#[derive(Debug, Clone, Copy)]
struct SmallQueue;
#[derive(Debug, Clone, Copy)]
struct MainQueue;
#[derive(Debug, Clone, Copy)]
struct GhostQueue;
#[derive(Debug, Clone, Copy)]
struct NoQueue;

// Type-level markers for NodeRef
#[derive(Debug, Clone, Copy)]
struct Occupied;
#[derive(Debug, Clone, Copy)]
struct Free;

#[derive(Debug)]
enum QueueHead<Q> {
    Some(NodeRef<Q, Occupied>),
    None,
}

trait QueueWithMembers {
    const QUEUE_ID: QueueTypeId;
}
impl QueueWithMembers for SmallQueue {
    const QUEUE_ID: QueueTypeId = QueueTypeId::Small;
}
impl QueueWithMembers for MainQueue {
    const QUEUE_ID: QueueTypeId = QueueTypeId::Main;
}
impl QueueWithMembers for GhostQueue {
    const QUEUE_ID: QueueTypeId = QueueTypeId::Ghost;
}

impl<Key: Eq + Hash, B: BuildHasher> CacheShard<Key, B> {
    pub fn new(small_threshold: u64, main_threshold: u64, ghost_threshold: u64, hasher: B) -> Self {
        Self {
            map: HashTable::new(),
            nodes_keys: Vec::new(),
            hasher,
            nodes: Vec::new(),
            freelist: Vec::new(),
            small_size: 0,
            main_size: 0,
            ghost_size: 0,
            small_threshold,
            main_threshold,
            ghost_threshold,
            small_head: QueueHead::None,
            main_head: QueueHead::None,
            ghost_head: QueueHead::None,
        }
    }

    pub fn with_estimated_count(
        estimated_items_count: usize,
        small_threshold: u64,
        main_threshold: u64,
        ghost_threshold: u64,
        hasher: B,
    ) -> Self {
        Self {
            map: HashTable::with_capacity(estimated_items_count),
            nodes_keys: Vec::with_capacity(estimated_items_count),
            hasher,
            nodes: Vec::with_capacity(estimated_items_count),
            freelist: Vec::with_capacity(estimated_items_count / 4),
            small_size: 0,
            main_size: 0,
            ghost_size: 0,
            small_threshold,
            main_threshold,
            ghost_threshold,
            small_head: QueueHead::None,
            main_head: QueueHead::None,
            ghost_head: QueueHead::None,
        }
    }

    /// Retrieves a cache entry by key.
    #[inline(always)]
    pub fn get_bytes(&mut self, key: &Key) -> Option<&Vec<u8>> {
        let hash = self.hasher.hash_one(key);
        let idx = self
            .map
            .find(hash, |&idx| self.nodes_keys[idx as usize] == *key)
            .map(|&idx| idx as usize)?;
        if self.nodes[idx].freq < 3 {
            self.nodes[idx].freq += 1;
        }
        (!self.nodes[idx].data.is_empty()).then_some(&self.nodes[idx].data)
    }

    /// Inserts or updates a cache entry by key.
    #[inline(always)]
    pub fn insert_bytes(&mut self, key: Key, data_size: u64, data: Vec<u8>) {
        let hash = self.hasher.hash_one(&key);

        if let Some(idx) = self
            .map
            .find(hash, |&idx| self.nodes_keys[idx as usize] == key)
            .map(|&idx| idx as usize)
        {
            // update node if it already exists
            if self.nodes[idx].freq < 3 {
                self.nodes[idx].freq += 1;
            }
            let weight_diff = data_size - self.nodes[idx].weight;
            match self.nodes[idx].queue {
                QueueTypeId::Small => self.small_size += weight_diff,
                QueueTypeId::Main => self.main_size += weight_diff,
                QueueTypeId::Ghost => self.main_size += weight_diff,
                QueueTypeId::NoQueue => {}
            }
            self.nodes[idx].data = data;
            self.nodes[idx].weight = data_size;
        } else {
            // otherwise, create a new node, insert it into the map and store the key
            let new_idx = self.allocate_small(data_size, data).idx;
            if new_idx as usize == self.nodes_keys.len() {
                self.nodes_keys.push(key);
            } else {
                self.nodes_keys[new_idx as usize] = key;
            }
            self.map.insert_unique(hash, new_idx, |&idx| {
                self.hasher.hash_one(&self.nodes_keys[idx as usize])
            });
        }

        // if after insertion, we exceed thresholds, evict nodes
        self.evict_small_if_needed();
        self.evict_ghost_if_needed();
        self.evict_main_if_needed();
    }

    /// Deletes (deallocates) a cache entry by key.
    /// Returns true if the node was found and deleted, false otherwise.
    pub fn delete(&mut self, key: &Key) -> bool {
        let hash = self.hasher.hash_one(key);
        let Some(idx) = self
            .map
            .find(hash, |&idx| self.nodes_keys[idx as usize] == *key)
            .map(|idx| (*idx) as usize)
        else {
            return false;
        };

        // check if node is occupied (has data)
        if self.nodes[idx].data.len() <= 0 {
            return false;
        }

        // remove node from its queue and update size
        match self.nodes[idx].queue {
            QueueTypeId::Small => {
                self.small_size -= self.nodes[idx].weight;
                let node_ref = get_node_ref::<SmallQueue>(idx, &self.nodes);
                let freed_ref = delete_node(node_ref, &mut self.small_head, &mut self.nodes);
                self.handle_node_eviction(freed_ref);
            }
            QueueTypeId::Main => {
                self.main_size -= self.nodes[idx].weight;
                let node_ref = get_node_ref::<MainQueue>(idx, &self.nodes);
                let freed_ref = delete_node(node_ref, &mut self.main_head, &mut self.nodes);
                self.handle_node_eviction(freed_ref);
            }
            QueueTypeId::Ghost => {
                self.ghost_size -= self.nodes[idx].weight;
                let node_ref = get_node_ref::<GhostQueue>(idx, &self.nodes);
                let freed_ref = delete_node(node_ref, &mut self.ghost_head, &mut self.nodes);
                self.handle_node_eviction(freed_ref);
            }
            QueueTypeId::NoQueue => return false,
        }
        true
    }

    fn allocate_small(&mut self, data_size: u64, data: Vec<u8>) -> NodeRef<SmallQueue, Occupied> {
        let new_node = self.create_node(data_size, data);
        self.small_size += data_size;
        move_to_queue::<SmallQueue>(new_node, &mut self.nodes, &mut self.small_head)
    }

    /// If small queue exceeds threshold, evict nodes from the head of the small queue:
    /// - if node has freq > 0, promote it to main queue
    /// - if node has freq == 0, demote it to ghost queue
    fn evict_small_if_needed(&mut self) {
        // TODO: maybe remove code duplication with `evict_ghost` and `evict_main`
        while self.small_size > self.small_threshold {
            if let Some(detached_head) = pop_head(&mut self.nodes, &mut self.small_head) {
                self.small_size -= self.nodes[detached_head.idx as usize].weight;
                if self.nodes[detached_head.idx as usize].freq > 0 {
                    self.promote_to_main(detached_head);
                } else {
                    self.demote_to_ghost(detached_head);
                }
            } else {
                // TODO: remove panic? (here and in other evict's)
                panic!("Tried to evict from small queue, but it is empty (Head of small is None)");
            }
        }
    }

    /// If ghost queue exceeds threshold, evict nodes from the head of the ghost queue:
    /// - if node has freq > 0 and has data, promote it to main queue
    /// - otherwise, evict it
    fn evict_ghost_if_needed(&mut self) {
        while self.ghost_size > self.ghost_threshold {
            if let Some(detached_head) = pop_head(&mut self.nodes, &mut self.ghost_head) {
                self.ghost_size -= self.nodes[detached_head.idx as usize].weight;
                if self.nodes[detached_head.idx as usize].freq > 0
                    && self.nodes[detached_head.idx as usize].data.len() > 0
                {
                    self.promote_to_main(detached_head);
                } else {
                    let freed_ref = evict_node(detached_head, &mut self.nodes);
                    self.handle_node_eviction(freed_ref);
                }
            } else {
                panic!("Tried to evict from ghost queue, but it is empty (Head of ghost is None)");
            }
        }
    }

    /// If main queue exceeds threshold, evict nodes from the head of the main queue:
    /// - if node has freq > 0, reinsert it back to main queue (with freq - 1)
    /// - otherwise, evict it
    fn evict_main_if_needed(&mut self) {
        while self.main_size > self.main_threshold {
            if let Some(detached_head) = pop_head(&mut self.nodes, &mut self.main_head) {
                if self.nodes[detached_head.idx as usize].freq > 0 {
                    // reinsert back to main queue
                    self.nodes[detached_head.idx as usize].freq -= 1;
                    let _ = move_to_queue::<MainQueue>(
                        detached_head,
                        &mut self.nodes,
                        &mut self.main_head,
                    );
                } else {
                    self.main_size -= self.nodes[detached_head.idx as usize].weight;
                    let freed_ref = evict_node(detached_head, &mut self.nodes);
                    self.handle_node_eviction(freed_ref);
                }
            } else {
                panic!("Tried to evict from main queue, but it is empty (Head of main is None)");
            }
        }
    }

    fn promote_to_main(&mut self, node_ref: NodeRef<NoQueue, Occupied>) {
        self.nodes[node_ref.idx as usize].freq = 0;
        self.main_size += self.nodes[node_ref.idx as usize].weight;
        let _ = move_to_queue::<MainQueue>(node_ref, &mut self.nodes, &mut self.main_head);
    }

    fn demote_to_ghost(&mut self, node_ref: NodeRef<NoQueue, Occupied>) {
        self.ghost_size += self.nodes[node_ref.idx as usize].weight;
        let ghost_ref =
            move_to_queue::<GhostQueue>(node_ref, &mut self.nodes, &mut self.ghost_head);
        self.nodes[ghost_ref.idx as usize].data = Vec::new(); // Drop data for ghost nodes
        // do not reset data_size (used to calculate ghost_size)
    }

    fn create_node(&mut self, data_size: u64, data: Vec<u8>) -> NodeRef<NoQueue, Occupied> {
        let idx = if let Some(freed_ref) = self.freelist.pop() {
            // reuse a freed node
            let occupied_ref = occupy_node(freed_ref, &mut self.nodes, data_size, data);
            occupied_ref.idx
        } else {
            // if none index is free, we need to allocate a new node
            let new_idx = self.nodes.len() as u32;
            self.nodes.push(Node {
                next: new_idx,
                prev: new_idx,
                data,
                weight: data_size,
                freq: 0,
                queue: QueueTypeId::NoQueue,
            });
            new_idx
        };

        NodeRef {
            idx,
            _occupied: PhantomData,
            _queue: PhantomData,
        }
    }

    fn handle_node_eviction(&mut self, node_ref: NodeRef<NoQueue, Free>) {
        // remove associated key
        let hash = self
            .hasher
            .hash_one(&self.nodes_keys[node_ref.idx as usize]);
        if let Ok(entry) = self.map.find_entry(hash, |&idx| idx == node_ref.idx) {
            entry.remove();
        }

        // add the freed node to the freelist
        self.freelist.push(node_ref);
    }

    pub fn print_queues(&self, truncate_count: usize) {
        self.print_queue("Small", &self.small_head, truncate_count);
        self.print_queue("Main", &self.main_head, truncate_count);
        self.print_queue("Ghost", &self.ghost_head, truncate_count);
    }

    pub fn get_small_size(&self) -> u64 {
        self.small_size
    }

    pub fn get_main_size(&self) -> u64 {
        self.main_size
    }

    pub fn get_ghost_size(&self) -> u64 {
        self.ghost_size
    }

    fn print_queue(
        &self,
        queue_name: &str,
        head: &QueueHead<impl QueueWithMembers + Copy>,
        truncate_count: usize,
    ) {
        let queue_label = format!("{} queue", queue_name);
        let pad = 12;

        println!("\n{:->width$}", "", width = pad + 30);

        match head {
            QueueHead::None => {
                println!("{:<pad$}[empty]", queue_label, pad = pad);
                println!("count: 0");
            }
            QueueHead::Some(start) => {
                let mut current_idx = start.idx;
                let mut out = Vec::new();
                let mut count = 0;

                loop {
                    if count <= truncate_count {
                        out.push(format!("{:?}", current_idx));
                    }

                    current_idx = self.nodes[current_idx as usize].next;
                    count += 1;
                    if count > 10_000 {
                        panic!("Too many elements in queue, something is wrong");
                    }

                    // If we've looped back to the start, we're done
                    if current_idx == start.idx {
                        break;
                    }
                }

                // if more than one element, show as: 87 -> 90 -> 89 -*> 87
                let joined = if out.len() == 1 {
                    format!("{} -*> {}", out[0], out[0])
                } else {
                    let mut s = out[..out.len().min(out.len() - 1)].join(" -> ");
                    s.push_str(" -*> ");
                    s.push_str(&out[0]);
                    s
                };

                println!("{:<pad$}{}", queue_label, joined, pad = pad);
                if count > truncate_count {
                    println!("{:>pad$} ... (truncated)", "", pad = pad);
                }
                println!("count: {}", count + 1); // +1 because we start at 0
            }
        }
        println!("{:->width$}\n", "", width = pad + 30);
    }
}

// Pop the head of the queue. Unlink the head if it exists, make previous node a new head, and return the unlinked node.
fn pop_head<Q: QueueWithMembers>(
    nodes: &mut Vec<Node>,
    head: &mut QueueHead<Q>,
) -> Option<NodeRef<NoQueue, Occupied>> {
    match head {
        // if head is Some, unlink it and return the unlinked node
        QueueHead::Some(head_ref) => {
            // if there is a previous node, set it as the new head
            if let Some(prev_ref) = prev_node(&head_ref, &nodes) {
                let old_head = std::mem::replace(head_ref, prev_ref); // hacky, it's here because unlink_node consumes NodeRef
                let unlinked_head = unlink_node(old_head, nodes);
                Some(unlinked_head)
            } else {
                // Single node in queue - remove it and set head to None
                let old_head = NodeRef {
                    idx: head_ref.idx,
                    _occupied: PhantomData,
                    _queue: PhantomData::<Q>,
                };
                *head = QueueHead::None;
                let unlinked_head = unlink_node(old_head, nodes);
                Some(unlinked_head)
            }
        }
        // if no head found, return None
        QueueHead::None => None,
    }
}

fn move_to_queue<Q: QueueWithMembers>(
    node_ref: NodeRef<NoQueue, Occupied>,
    nodes: &mut Vec<Node>,
    head: &mut QueueHead<Q>,
) -> NodeRef<Q, Occupied> {
    nodes[node_ref.idx as usize].queue = Q::QUEUE_ID;
    match head {
        QueueHead::Some(head_ref) => {
            // link to the head of the new queue
            let tail_idx = nodes[head_ref.idx as usize].next;
            nodes[tail_idx as usize].prev = node_ref.idx;
            nodes[node_ref.idx as usize].prev = head_ref.idx;
            nodes[node_ref.idx as usize].next = tail_idx;
            nodes[head_ref.idx as usize].next = node_ref.idx;
        }
        QueueHead::None => {
            let new_head_ref = NodeRef {
                idx: node_ref.idx,
                _occupied: PhantomData,
                _queue: PhantomData::<Q>,
            };
            // if the head of is None, we set new queue as the head
            *head = QueueHead::Some(new_head_ref);
        }
    }
    NodeRef {
        idx: node_ref.idx,
        _occupied: PhantomData,
        _queue: PhantomData,
    }
}

fn unlink_node<Q: QueueWithMembers>(
    node_ref: NodeRef<Q, Occupied>,
    nodes: &mut Vec<Node>,
) -> NodeRef<NoQueue, Occupied> {
    nodes[node_ref.idx as usize].queue = QueueTypeId::NoQueue;

    // link the previous and next nodes to each other
    let next_idx = nodes[node_ref.idx as usize].next;
    let prev_idx = nodes[node_ref.idx as usize].prev;
    nodes[prev_idx as usize].next = next_idx;
    nodes[next_idx as usize].prev = prev_idx;

    // link to itself
    nodes[node_ref.idx as usize].next = node_ref.idx;
    nodes[node_ref.idx as usize].prev = node_ref.idx;

    NodeRef {
        idx: node_ref.idx,
        _occupied: PhantomData,
        _queue: PhantomData::<NoQueue>,
    }
}

// Evicts node from its queue and frees it
// Handles the case when the node is the head of the queue (updating head accordingly)
fn delete_node<Q: QueueWithMembers>(
    node_ref: NodeRef<Q, Occupied>,
    head: &mut QueueHead<Q>,
    nodes: &mut Vec<Node>,
) -> NodeRef<NoQueue, Free> {
    let is_head = match head {
        QueueHead::Some(head_ref) => head_ref.idx == node_ref.idx,
        QueueHead::None => false,
    };
    if is_head {
        if let Some(detached_head) = pop_head(nodes, head) {
            evict_node(detached_head, nodes)
        } else {
            unreachable!();
        }
    } else {
        let unlinked = unlink_node(node_ref, nodes);
        evict_node(unlinked, nodes)
    }
}

fn prev_node<Q: QueueWithMembers>(
    node_ref: &NodeRef<Q, Occupied>,
    nodes: &Vec<Node>,
) -> Option<NodeRef<Q, Occupied>> {
    if nodes[node_ref.idx as usize].prev == node_ref.idx {
        // if the prev node is itself, it means it's the only node in the queue
        return None;
    }
    let prev_idx = nodes[node_ref.idx as usize].prev;
    Some(NodeRef {
        idx: prev_idx,
        _occupied: PhantomData,
        _queue: PhantomData,
    })
}

fn evict_node(
    node_ref: NodeRef<NoQueue, Occupied>,
    nodes: &mut Vec<Node>,
) -> NodeRef<NoQueue, Free> {
    nodes[node_ref.idx as usize].data = Vec::new();
    nodes[node_ref.idx as usize].weight = 0;
    nodes[node_ref.idx as usize].freq = 0;
    nodes[node_ref.idx as usize].next = u32::MAX; // set to u32::MAX so any use as an index will panic
    nodes[node_ref.idx as usize].prev = u32::MAX;
    NodeRef {
        idx: node_ref.idx,
        _occupied: PhantomData,
        _queue: PhantomData::<NoQueue>,
    }
}

fn occupy_node(
    node_ref: NodeRef<NoQueue, Free>,
    nodes: &mut Vec<Node>,
    data_size: u64,
    data: Vec<u8>,
) -> NodeRef<NoQueue, Occupied> {
    nodes[node_ref.idx as usize].data = data;
    nodes[node_ref.idx as usize].weight = data_size;
    nodes[node_ref.idx as usize].freq = 0;
    nodes[node_ref.idx as usize].next = node_ref.idx;
    nodes[node_ref.idx as usize].prev = node_ref.idx;
    NodeRef {
        idx: node_ref.idx,
        _occupied: PhantomData,
        _queue: PhantomData::<NoQueue>,
    }
}

// Get NodeRef<Q: QueueWithMembers, Occupied> given index. Does not check if Node is actually in the state that NodeRef assumes.
// Panics if the node is not part of any queue.
fn get_node_ref<Q: QueueWithMembers>(idx: usize, nodes: &Vec<Node>) -> NodeRef<Q, Occupied> {
    match nodes[idx].queue {
        QueueTypeId::NoQueue => panic!("Node at index {} is not part of any queue", idx),
        _ => NodeRef {
            idx: idx as u32,
            _occupied: PhantomData,
            _queue: PhantomData::<Q>,
        },
    }
}
