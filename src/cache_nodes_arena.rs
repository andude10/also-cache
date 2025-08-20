use std::hash::BuildHasher;
use std::{hash::Hash, marker::PhantomData};

use hashbrown::HashTable;

#[derive(Debug, Clone, Copy)]
struct SmallQueue;
#[derive(Debug, Clone, Copy)]
struct MainQueue;
#[derive(Debug, Clone, Copy)]
struct GhostQueue;
#[derive(Debug, Clone, Copy)]
struct NoQueue;

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

#[derive(Debug, Clone, Copy)]
struct Occupied;
#[derive(Debug, Clone, Copy)]
struct Free;

#[derive(Debug, Clone, Copy, PartialEq)]
enum QueueTypeId {
    NoQueue,
    Small,
    Main,
    Ghost,
}

#[derive(Debug, Clone)]
struct Node {
    next: usize,
    prev: usize,
    data: Vec<u8>,
    weight: usize,
    freq: u16,
    queue: QueueTypeId,
}

// TODO: so ideally during the whole lifetime of NodeRef, index should be in the state that NodeRef enforces, but it's not always like that.

// This is represents a reference to a node in a Vec<Node>. Index in Vec<Node> can be
// in many states: occupied or free, part of some queue or not. To make things easier
// when managing nodes, NodeRef enforces state of some node with type safety.
// Q: Queue of which this node is member of (SmallQueue, MainQueue, GhostQueue), or none (NoQueue).
// O: Occupied or Free.
#[derive(Debug)]
pub struct NodeRef<Q, O> {
    idx: usize,
    _occupied: PhantomData<O>,
    _queue: PhantomData<Q>,
}

#[derive(Debug)]
enum QueueHead<Q> {
    Some(NodeRef<Q, Occupied>),
    None,
}

#[derive(Debug)]
pub struct NodeArena<Key, B> {
    map: HashTable<usize>,
    nodes_keys: Vec<Key>,
    hasher: B,

    nodes: Vec<Node>,
    freelist: Vec<NodeRef<NoQueue, Free>>,

    small_size: usize,
    main_size: usize,
    ghost_size: usize,
    small_threshold: usize,
    main_threshold: usize,
    ghost_threshold: usize,

    small_head: QueueHead<SmallQueue>,
    main_head: QueueHead<MainQueue>,
    ghost_head: QueueHead<GhostQueue>,
}

impl<Key: Eq + Hash, B: BuildHasher> NodeArena<Key, B> {
    pub fn new(
        small_threshold: usize,
        main_threshold: usize,
        ghost_threshold: usize,
        hasher: B,
    ) -> Self {
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

    #[inline(always)]
    pub fn get_bytes(&mut self, key: &Key) -> Option<&Vec<u8>> {
        // TODO: checks if key is not freed by checking if data_size is 0, so no type checking
        let hash = self.hasher.hash_one(key);
        let idx: usize = *self.map.find(hash, |&idx| self.nodes_keys[idx] == *key)?;
        if self.nodes[idx].freq < 3 {
            self.nodes[idx].freq += 1;
        }
        return if self.nodes[idx].data.len() > 0 {
            Some(&self.nodes[idx].data)
        } else {
            None
        };
    }

    #[inline(always)]
    pub fn insert_bytes(&mut self, key: Key, data_size: usize, data: Vec<u8>) {
        let hash = self.hasher.hash_one(&key);
        if let Some(&existing_idx) = self.map.find(hash, |&idx| self.nodes_keys[idx] == key) {
            self.nodes[existing_idx].freq += 1;
            self.nodes[existing_idx].data = data;
            return;
        }
        let new_node_ref = self.allocate_small(data_size, data);
        self.map.insert_unique(hash, new_node_ref.idx, |&idx| {
            self.hasher.hash_one(&self.nodes_keys[idx])
        });
        if new_node_ref.idx >= self.nodes_keys.len() {
            self.nodes_keys.push(key);
        } else {
            self.nodes_keys[new_node_ref.idx] = key;
        }

        // TODO: fix problem with NodeRef (if it's possible): here we have valid NodeRef, but after evictions it may become invalid

        self.evict_small_if_needed();
        self.evict_ghost_if_needed();
        self.evict_main_if_needed();
    }

    pub fn delete(&mut self, key: &Key) -> bool {
        let hash = self.hasher.hash_one(key);
        if let Some(idx) = self.map.find(hash, |&idx| self.nodes_keys[idx] == *key) {
            // Check if node is occupied (has data)
            if self.nodes[*idx].data.len() <= 0 {
                return false;
            }
            match self.nodes[*idx].queue {
                QueueTypeId::Small => {
                    self.small_size -= self.nodes[*idx].weight;
                    let node_ref = get_node_ref::<SmallQueue>(*idx, &self.nodes);
                    if node_ref_is_head(&node_ref, &self.small_head) {
                        if let Some(detached_head) = pop_head(&mut self.nodes, &mut self.small_head)
                        {
                            let freed_ref = evict_node(detached_head, &mut self.nodes);
                            self.handle_node_eviction(freed_ref);
                        } else {
                            unreachable!();
                        }
                    } else {
                        let unlinked = unlink_node(node_ref, &mut self.nodes);
                        let freed_ref = evict_node(unlinked, &mut self.nodes);
                        self.handle_node_eviction(freed_ref);
                    }
                }
                QueueTypeId::Main => {
                    self.main_size -= self.nodes[*idx].weight;
                    let node_ref = get_node_ref::<MainQueue>(*idx, &self.nodes);
                    if node_ref_is_head(&node_ref, &self.main_head) {
                        if let Some(detached_head) = pop_head(&mut self.nodes, &mut self.main_head)
                        {
                            let freed_ref = evict_node(detached_head, &mut self.nodes);
                            self.handle_node_eviction(freed_ref);
                        } else {
                            unreachable!();
                        }
                    } else {
                        let unlinked = unlink_node(node_ref, &mut self.nodes);
                        let freed_ref = evict_node(unlinked, &mut self.nodes);
                        self.handle_node_eviction(freed_ref);
                    }
                }
                QueueTypeId::Ghost => {
                    self.ghost_size -= self.nodes[*idx].weight;
                    let node_ref = get_node_ref::<GhostQueue>(*idx, &self.nodes);
                    if node_ref_is_head(&node_ref, &self.ghost_head) {
                        if let Some(detached_head) = pop_head(&mut self.nodes, &mut self.ghost_head)
                        {
                            let freed_ref = evict_node(detached_head, &mut self.nodes);
                            self.handle_node_eviction(freed_ref);
                        } else {
                            unreachable!();
                        }
                    } else {
                        let unlinked = unlink_node(node_ref, &mut self.nodes);
                        let freed_ref = evict_node(unlinked, &mut self.nodes);
                        self.handle_node_eviction(freed_ref);
                    }
                }
                QueueTypeId::NoQueue => {} // Already freed node, nothing to do
            }
            true
        } else {
            false
        }
    }

    fn allocate_small(&mut self, data_size: usize, data: Vec<u8>) -> NodeRef<SmallQueue, Occupied> {
        let new_node = self.create_node(data_size, data);
        self.small_size += data_size;
        move_to_queue::<SmallQueue>(new_node, &mut self.nodes, &mut self.small_head)
    }

    #[inline(always)]
    fn evict_small_if_needed(&mut self) {
        while self.small_size > self.small_threshold {
            if let Some(detached_head) = pop_head(&mut self.nodes, &mut self.small_head) {
                self.small_size -= self.nodes[detached_head.idx].weight;
                if self.nodes[detached_head.idx].freq > 0 {
                    self.promote_to_main(detached_head);
                } else {
                    self.demote_to_ghost(detached_head);
                }
            } else {
                panic!("Tried to evict from small queue, but it is empty (Head of small is None)");
            }
        }
    }

    // TODO: maybe remove code duplication with `evict_small` and `evict_main`
    #[inline(always)]
    fn evict_ghost_if_needed(&mut self) {
        while self.ghost_size > self.ghost_threshold {
            if let Some(detached_head) = pop_head(&mut self.nodes, &mut self.ghost_head) {
                self.ghost_size -= self.nodes[detached_head.idx].weight;
                if self.nodes[detached_head.idx].freq > 0
                    && self.nodes[detached_head.idx].data.len() > 0
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

    #[inline(always)]
    fn evict_main_if_needed(&mut self) {
        while self.main_size > self.main_threshold {
            if let Some(detached_head) = pop_head(&mut self.nodes, &mut self.main_head) {
                if self.nodes[detached_head.idx].freq > 0 {
                    self.reinsert_main(detached_head);
                } else {
                    self.main_size -= self.nodes[detached_head.idx].weight;
                    let freed_ref = evict_node(detached_head, &mut self.nodes);
                    self.handle_node_eviction(freed_ref);
                }
            } else {
                panic!("Tried to evict from main queue, but it is empty (Head of main is None)");
            }
        }
    }

    fn reinsert_main(&mut self, node_ref: NodeRef<NoQueue, Occupied>) {
        self.nodes[node_ref.idx].freq -= 1;
        let _ = move_to_queue::<MainQueue>(node_ref, &mut self.nodes, &mut self.main_head);
    }

    fn promote_to_main(&mut self, node_ref: NodeRef<NoQueue, Occupied>) {
        self.nodes[node_ref.idx].freq = 0;
        self.main_size += self.nodes[node_ref.idx].weight;
        let _ = move_to_queue::<MainQueue>(node_ref, &mut self.nodes, &mut self.main_head);
    }

    fn demote_to_ghost(&mut self, node_ref: NodeRef<NoQueue, Occupied>) {
        self.ghost_size += self.nodes[node_ref.idx].weight;
        let ghost_ref =
            move_to_queue::<GhostQueue>(node_ref, &mut self.nodes, &mut self.ghost_head);
        self.nodes[ghost_ref.idx].data = Vec::new(); // Drop data for ghost nodes
        // do not reset data_size (used to calculate ghost_size)
    }

    fn create_node(&mut self, data_size: usize, data: Vec<u8>) -> NodeRef<NoQueue, Occupied> {
        let idx = if let Some(freed_ref) = self.freelist.pop() {
            // reuse a freed node
            let occupied_ref = occupy_node(freed_ref, &mut self.nodes, data_size, data);
            occupied_ref.idx
        } else {
            // if no free index, we need to allocate a new node
            let new_idx = self.nodes.len();
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
        let hash = self.hasher.hash_one(&self.nodes_keys[node_ref.idx]);
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
                        out.push(format!("{}", current_idx));
                    }

                    current_idx = self.nodes[current_idx].next;
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

fn node_ref_is_head<Q: QueueWithMembers>(
    node_ref: &NodeRef<Q, Occupied>,
    head: &QueueHead<Q>,
) -> bool {
    match head {
        QueueHead::Some(head_ref) => head_ref.idx == node_ref.idx,
        QueueHead::None => false,
    }
}

fn get_node_ref<Q: QueueWithMembers>(idx: usize, nodes: &Vec<Node>) -> NodeRef<Q, Occupied> {
    match nodes[idx].queue {
        QueueTypeId::NoQueue => panic!("Node at index {} is not part of any queue", idx),
        _ => NodeRef {
            idx,
            _occupied: PhantomData,
            _queue: PhantomData::<Q>,
        },
    }
}

fn unlink_node<Q: QueueWithMembers>(
    node_ref: NodeRef<Q, Occupied>,
    nodes: &mut Vec<Node>,
) -> NodeRef<NoQueue, Occupied> {
    nodes[node_ref.idx].queue = QueueTypeId::NoQueue;

    // link the previous and next nodes to each other
    let next_idx = nodes[node_ref.idx].next;
    let prev_idx = nodes[node_ref.idx].prev;
    nodes[prev_idx].next = next_idx;
    nodes[next_idx].prev = prev_idx;

    // link to itself
    nodes[node_ref.idx].next = node_ref.idx;
    nodes[node_ref.idx].prev = node_ref.idx;

    NodeRef {
        idx: node_ref.idx,
        _occupied: PhantomData,
        _queue: PhantomData::<NoQueue>,
    }
}

fn prev_node<Q: QueueWithMembers>(
    node_ref: &NodeRef<Q, Occupied>,
    nodes: &Vec<Node>,
) -> Option<NodeRef<Q, Occupied>> {
    if nodes[node_ref.idx].prev == node_ref.idx {
        // if the prev node is itself, it means it's the only node in the queue
        return None;
    }
    // otherwise, return the previous node
    let prev_idx = nodes[node_ref.idx].prev;
    Some(NodeRef {
        idx: prev_idx,
        _occupied: PhantomData,
        _queue: PhantomData::<Q>,
    })
}

fn evict_node(
    node_ref: NodeRef<NoQueue, Occupied>,
    nodes: &mut Vec<Node>,
) -> NodeRef<NoQueue, Free> {
    nodes[node_ref.idx].data = Vec::new();
    nodes[node_ref.idx].weight = 0;
    nodes[node_ref.idx].freq = 0;
    nodes[node_ref.idx].next = usize::MAX; // set to usize::MAX so any use as an index will panic
    nodes[node_ref.idx].prev = usize::MAX;
    NodeRef {
        idx: node_ref.idx,
        _occupied: PhantomData,
        _queue: PhantomData::<NoQueue>,
    }
}

fn occupy_node(
    node_ref: NodeRef<NoQueue, Free>,
    nodes: &mut Vec<Node>,
    data_size: usize,
    data: Vec<u8>,
) -> NodeRef<NoQueue, Occupied> {
    nodes[node_ref.idx].data = data;
    nodes[node_ref.idx].weight = data_size;
    nodes[node_ref.idx].freq = 0;
    nodes[node_ref.idx].next = node_ref.idx;
    nodes[node_ref.idx].prev = node_ref.idx;
    NodeRef {
        idx: node_ref.idx,
        _occupied: PhantomData,
        _queue: PhantomData::<NoQueue>,
    }
}

fn move_to_queue<Q: QueueWithMembers>(
    node_ref: NodeRef<NoQueue, Occupied>,
    nodes: &mut Vec<Node>,
    head: &mut QueueHead<Q>,
) -> NodeRef<Q, Occupied> {
    nodes[node_ref.idx].queue = Q::QUEUE_ID;
    match head {
        QueueHead::Some(head_ref) => {
            // link to the head of the new queue
            let tail_idx = nodes[head_ref.idx].next;
            nodes[tail_idx].prev = node_ref.idx;
            nodes[node_ref.idx].prev = head_ref.idx;
            nodes[node_ref.idx].next = tail_idx;
            nodes[head_ref.idx].next = node_ref.idx;
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
        _queue: PhantomData::<Q>,
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
