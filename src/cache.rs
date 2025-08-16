use std::{
    collections::{HashMap, VecDeque},
    hash::{BuildHasher, Hash},
};

use hashbrown::HashTable;

#[derive(Debug, Clone)]
struct Node {
    next: usize,
    prev: usize,
    data: Vec<u8>,
    frequency: u16,
    fifo: FifoName,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum FifoName {
    Small,
    Ghost,
    Main,
}

pub struct Cache<Key, We, B> {
    map: HashTable<usize>,
    nodes: Vec<Node>, // need to access nodes in O(1), reads are frequent
    nodes_keys: Vec<Key>,
    nodes_freelist: Vec<usize>,

    // Heads of queues (head = the oldest item)
    small_head: Option<usize>,
    main_head: Option<usize>,
    ghost_head: Option<usize>,

    // If size of queue is more then threshold, next insertion will cause eviction
    small_threshold: usize,
    main_threshold: usize,
    ghost_threshold: usize,

    small_size: usize,
    main_size: usize,
    ghost_size: usize,

    weighter: We,
    hasher: B,
}

pub trait Weighter {
    fn weight(&self, val: &Vec<u8>) -> usize;
}

// TODO: remove clone from key
impl<Key: Eq + Hash + Clone, We: Weighter, B: BuildHasher> Cache<Key, We, B> {
    pub fn new(size: usize, weighter: We, hash_builder: B) -> Self {
        Cache {
            map: HashTable::with_capacity(size),
            nodes_keys: Vec::with_capacity(0),
            nodes: Vec::with_capacity(0),
            nodes_freelist: Vec::with_capacity(0),
            small_head: None,
            main_head: None,
            ghost_head: None,
            small_threshold: (size as f64 * 0.1) as usize,
            main_threshold: size,
            ghost_threshold: (size as f64 * 0.4) as usize,
            small_size: 0,
            main_size: 0,
            ghost_size: 0,
            weighter,
            hasher: hash_builder,
        }
    }

    pub fn get_bytes(&mut self, key: &Key) -> Option<&Vec<u8>> {
        let hash = self.hasher.hash_one(key);
        let node_idx: usize = *self.map.find(hash, |&idx| self.nodes_keys[idx] == *key)?;
        self.node_read(node_idx);
        Some(&self.nodes[node_idx].data)
    }

    pub fn insert_bytes(&mut self, key: &Key, val: Vec<u8>) {
        let hash = self.hasher.hash_one(key);
        let idx = self
            .nodes_freelist
            .pop()
            .unwrap_or(self.nodes_keys.len() - 1);

        self.nodes_keys[idx] = key.clone();
        self.nodes[idx] = Node {
            next: idx,
            prev: idx,
            data: val,
            frequency: 0,
            fifo: FifoName::Small,
        };
        self.map.insert_unique(hash, idx, |_| hash);
        self.node_advance(FifoName::Small, idx);
    }

    fn node_read(&mut self, node_idx: usize) {
        let node = &mut self.nodes[node_idx];
        if node.fifo == FifoName::Ghost {
            self.node_advance(FifoName::Main, node_idx);
        } else if node.frequency < 3 {
            node.frequency += 1;
        }
    }

    fn node_advance(&mut self, queue_name: FifoName, node_idx: usize) {
        // evict if queue is full
        let (size, threshold) = match queue_name {
            FifoName::Small => (self.small_size, self.small_threshold),
            FifoName::Main => (self.main_size, self.main_threshold),
            FifoName::Ghost => (self.ghost_size, self.ghost_threshold),
        };
        if size >= threshold {
            self.node_evict(queue_name);
        }

        // add node's weight to the queue size
        let size = match queue_name {
            FifoName::Small => &mut self.small_size,
            FifoName::Main => &mut self.main_size,
            FifoName::Ghost => &mut self.ghost_size,
        };
        *size += self.weighter.weight(&self.nodes[node_idx].data);

        // put node into the beginning of the queue
        let head = match queue_name {
            FifoName::Small => &mut self.small_head,
            FifoName::Main => &mut self.main_head,
            FifoName::Ghost => &mut self.ghost_head,
        };
        if let Some(head_idx) = *head {
            self.put_into_queue(node_idx, head_idx);
        } else {
            *head = Some(node_idx);
        }

        self.nodes[node_idx].fifo = queue_name;
    }

    fn node_evict(&mut self, queue_name: FifoName) {
        let (size, threshold) = match queue_name {
            FifoName::Small => (self.small_size, self.small_threshold),
            FifoName::Main => (self.main_size, self.main_threshold),
            FifoName::Ghost => (self.ghost_size, self.ghost_threshold),
        };

        let mut ind = match queue_name {
            FifoName::Small => self.small_head,
            FifoName::Main => self.main_head,
            FifoName::Ghost => self.ghost_head,
        }
        .expect("head to exist when evicting");

        while size > threshold {
            // make previous node the new head
            let prev_node = self.nodes[ind].prev;
            match queue_name {
                FifoName::Small => {
                    self.small_head = Some(prev_node);
                }
                FifoName::Main => {
                    self.main_head = Some(prev_node);
                }
                FifoName::Ghost => {
                    self.ghost_head = Some(prev_node);
                }
            }

            // advance/remove old head
            self.remove_from_queue(ind);
            match (queue_name, self.nodes[ind].frequency > 0) {
                (FifoName::Small, true) => {
                    self.nodes[ind].frequency -= 1;
                    self.node_advance(FifoName::Main, ind);
                }
                (FifoName::Small, false) => self.node_advance(FifoName::Ghost, ind),
                (FifoName::Ghost, true) => {
                    self.nodes[ind].frequency -= 1;
                    self.node_advance(FifoName::Main, ind);
                }
                (FifoName::Ghost, false) => {
                    self.drop_node(ind);
                    self.ghost_size -= self.weighter.weight(&self.nodes[ind].data)
                }
                (FifoName::Main, true) => {
                    self.nodes[ind].frequency -= 1;
                    self.put_into_queue(ind, prev_node);
                }
                (FifoName::Main, false) => {
                    self.drop_node(ind);
                    self.main_size -= self.weighter.weight(&self.nodes[ind].data)
                }
            }

            ind = prev_node;
        }
    }

    fn drop_node(&mut self, node_idx: usize) {
        self.nodes[node_idx] = Node {
            next: node_idx,
            prev: node_idx,
            data: Vec::new(),
            frequency: 0,
            fifo: FifoName::Small,
        };
        self.nodes_freelist.push(node_idx);
    }

    fn remove_from_queue(&mut self, node_idx: usize) {
        let node = &mut self.nodes[node_idx];
        let prev_idx = node.prev;
        let next_idx = node.next;
        self.nodes[prev_idx].next = next_idx;
        self.nodes[next_idx].prev = prev_idx;
    }

    fn put_into_queue(&mut self, node_idx: usize, head_idx: usize) {
        let tail_idx = self.nodes[head_idx].next;
        self.nodes[tail_idx].prev = node_idx;
        self.nodes[node_idx].prev = head_idx;
        self.nodes[node_idx].next = tail_idx;
        self.nodes[head_idx].next = node_idx;
    }
}
