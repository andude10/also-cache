use std::hash::{BuildHasher, Hash, RandomState};

use bincode::{
    config::standard,
    error::{DecodeError, EncodeError},
};
use hashbrown::HashTable;
use serde::{Serialize, de::DeserializeOwned};

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

#[derive(Debug, Clone)]
pub struct AlsoCache<Key, We, B> {
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

#[derive(Debug, Clone)]
pub struct DefaultWeighter;

impl Weighter for DefaultWeighter {
    fn weight(&self, val: &Vec<u8>) -> usize {
        val.len()
    }
}

#[derive(Debug)]
pub enum CacheError {
    Decode(DecodeError),
    Encode(EncodeError),
    KeyNotFound,
}

impl<Key: Eq + Hash, We: Weighter, B: BuildHasher> AlsoCache<Key, We, B> {
    pub fn with(size: usize, weighter: We, hash_builder: B) -> Self {
        AlsoCache {
            map: HashTable::with_capacity(size),
            nodes_keys: Vec::with_capacity(0),
            nodes: Vec::with_capacity(0),
            nodes_freelist: Vec::with_capacity(0),
            small_head: None,
            main_head: None,
            ghost_head: None,
            small_threshold: (size as f64 * 0.1) as usize,
            main_threshold: size - (size as f64 * 0.5) as usize,
            ghost_threshold: (size as f64 * 0.4) as usize,
            small_size: 0,
            main_size: 0,
            ghost_size: 0,
            weighter,
            hasher: hash_builder,
        }
    }
    pub fn get<V: DeserializeOwned>(&mut self, key: &Key) -> Result<V, CacheError> {
        let bytes = self.get_bytes(key).ok_or(CacheError::KeyNotFound)?;
        deserialize(bytes).map_err(CacheError::Decode)
    }

    pub fn insert<V: Serialize>(&mut self, key: Key, val: &V) -> Result<(), CacheError> {
        let bytes = serialize(val).map_err(CacheError::Encode)?;
        self.insert_bytes(key, bytes);
        Ok(())
    }

    pub fn get_bytes(&mut self, key: &Key) -> Option<&Vec<u8>> {
        let hash = self.hasher.hash_one(key);
        let node_idx: usize = *self.map.find(hash, |&idx| self.nodes_keys[idx] == *key)?;
        self.node_read(node_idx);
        Some(&self.nodes[node_idx].data)
    }

    pub fn insert_bytes(&mut self, key: Key, val: Vec<u8>) {
        let hash = self.hasher.hash_one(&key);

        let node = Node {
            next: 0,
            prev: 0,
            data: val,
            frequency: 0,
            fifo: FifoName::Small,
        };
        let idx = if let Some(free_idx) = self.nodes_freelist.pop() {
            self.nodes_keys[free_idx] = key;
            self.nodes[free_idx] = node;
            free_idx
        } else {
            self.nodes_keys.push(key);
            self.nodes.push(node);
            self.nodes.len() - 1
        };

        // self-referential for empty queue
        self.nodes[idx].next = idx;
        self.nodes[idx].prev = idx;

        self.map.insert_unique(hash, idx, |_| hash);
        self.node_advance(FifoName::Small, idx);
    }

    fn node_read(&mut self, node_idx: usize) {
        assert!(
            self.nodes[node_idx].next != usize::MAX,
            "Expect valid index in node_read"
        );
        let node = &mut self.nodes[node_idx];
        if node.fifo == FifoName::Ghost {
            self.node_advance(FifoName::Main, node_idx);
        } else if node.frequency < 3 {
            node.frequency += 1;
        }
    }

    fn node_advance(&mut self, queue_name: FifoName, node_idx: usize) {
        assert!(
            self.nodes[node_idx].next != usize::MAX,
            "Expect valid index in node_advance"
        );

        // evict an item if the queue exceeds its threshold
        let (size, threshold) = match queue_name {
            FifoName::Small => (self.small_size, self.small_threshold),
            FifoName::Main => (self.main_size, self.main_threshold),
            FifoName::Ghost => (self.ghost_size, self.ghost_threshold),
        };
        if size > threshold {
            self.node_evict(queue_name);
        }

        // increment queue size by the node's weigh
        let size = match queue_name {
            FifoName::Small => &mut self.small_size,
            FifoName::Main => &mut self.main_size,
            FifoName::Ghost => &mut self.ghost_size,
        };
        *size += self.weighter.weight(&self.nodes[node_idx].data);

        // insert node at the front of the queue
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

    // TODO: make more straightforward
    fn node_evict(&mut self, queue_name: FifoName) {
        let (mut size, threshold, ind) = match queue_name {
            FifoName::Small => (self.small_size, self.small_threshold, self.small_head),
            FifoName::Main => (self.main_size, self.main_threshold, self.main_head),
            FifoName::Ghost => (self.ghost_size, self.ghost_threshold, self.ghost_head),
        };
        let mut next_idx = ind.expect("head to exist when evicting");
        let mut old_head_idx;

        while size > threshold {
            assert!(
                self.nodes[next_idx].next != usize::MAX,
                "Expect valid index while evicting"
            );

            // if the head is the only node in the queue, exit
            if self.nodes[next_idx].prev == self.nodes[next_idx].next
                && self.nodes[next_idx].next == next_idx
            {
                match queue_name {
                    FifoName::Small => {
                        self.small_head = None;
                        self.small_size = 0;
                    }
                    FifoName::Main => {
                        self.main_head = None;
                        self.main_size = 0;
                    }
                    FifoName::Ghost => {
                        self.ghost_head = None;
                        self.ghost_size = 0;
                    }
                }
                self.drop_node(next_idx);
                break;
            }

            // make previous node the new head
            let prev_node = self.nodes[next_idx].prev;
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
            old_head_idx = next_idx;
            next_idx = prev_node;

            // advance or remove old head
            self.unlink_from_queue(old_head_idx);
            match (queue_name, self.nodes[old_head_idx].frequency > 0) {
                (FifoName::Small, true) => {
                    self.nodes[old_head_idx].frequency -= 1;
                    self.small_size -= self.weighter.weight(&self.nodes[old_head_idx].data);
                    self.node_advance(FifoName::Main, old_head_idx);
                }
                (FifoName::Small, false) => {
                    self.small_size -= self.weighter.weight(&self.nodes[old_head_idx].data);
                    self.node_advance(FifoName::Ghost, old_head_idx);
                }
                (FifoName::Ghost, true) => {
                    self.nodes[old_head_idx].frequency -= 1;
                    self.ghost_size -= self.weighter.weight(&self.nodes[old_head_idx].data);
                    self.node_advance(FifoName::Main, old_head_idx);
                }
                (FifoName::Ghost, false) => {
                    self.ghost_size -= self.weighter.weight(&self.nodes[old_head_idx].data);
                    self.drop_node(old_head_idx);
                }
                (FifoName::Main, true) => {
                    self.nodes[old_head_idx].frequency -= 1;
                    // main queue size does not change
                    self.put_into_queue(old_head_idx, prev_node);
                }
                (FifoName::Main, false) => {
                    self.main_size -= self.weighter.weight(&self.nodes[old_head_idx].data);
                    self.drop_node(old_head_idx);
                }
            }

            size = match queue_name {
                FifoName::Small => self.small_size,
                FifoName::Main => self.main_size,
                FifoName::Ghost => self.ghost_size,
            };
        }
    }

    fn drop_node(&mut self, node_idx: usize) {
        // remove associated key from map
        let hash = self.hasher.hash_one(&self.nodes_keys[node_idx]);
        if let Ok(entry) = self.map.find_entry(hash, |&idx| idx == node_idx) {
            entry.remove();
        }

        // drop
        self.nodes[node_idx] = Node {
            next: usize::MAX, // set to usize::MAX so any use as an index will panic
            prev: usize::MAX,
            data: Vec::new(),
            frequency: 0,
            fifo: FifoName::Small,
        };

        // put index into freelist
        self.nodes_freelist.push(node_idx);
    }

    fn unlink_from_queue(&mut self, node_idx: usize) {
        // link the previous and next to each other
        let node = &mut self.nodes[node_idx];
        let prev_idx = node.prev;
        let next_idx = node.next;
        self.nodes[prev_idx].next = next_idx;
        self.nodes[next_idx].prev = prev_idx;

        // link to itself
        self.nodes[node_idx].next = node_idx;
        self.nodes[node_idx].prev = node_idx;
    }

    fn put_into_queue(&mut self, node_idx: usize, head_idx: usize) {
        let tail_idx = self.nodes[head_idx].next;
        self.nodes[tail_idx].prev = node_idx;
        self.nodes[node_idx].prev = head_idx;
        self.nodes[node_idx].next = tail_idx;
        self.nodes[head_idx].next = node_idx;
    }

    pub fn print_queues(&self, truncate_count: usize) {
        self.print_queue(FifoName::Small, truncate_count);
        self.print_queue(FifoName::Main, truncate_count);
        self.print_queue(FifoName::Ghost, truncate_count);
    }

    fn print_queue(&self, queue_name: FifoName, truncate_count: usize) {
        let head = match queue_name {
            FifoName::Small => self.small_head,
            FifoName::Main => self.main_head,
            FifoName::Ghost => self.ghost_head,
        };

        let queue_label = format!("{:?} queue", queue_name);
        let pad = 12;

        println!("\n{:->width$}", "", width = pad + 30);

        match head {
            None => {
                println!("{:<pad$}[empty]", queue_label, pad = pad);
                println!("count: 0");
            }
            Some(start) => {
                let mut idx = start;
                let mut out = Vec::new();
                let mut count = 0;
                loop {
                    if count <= truncate_count {
                        out.push(format!("{}", idx));
                    }
                    idx = self.nodes[idx].next;
                    count += 1;
                    if idx == start {
                        break;
                    }
                }
                // if more than one element, show as: 87 -> 90 -> 89 -*> 87
                let joined = if out.len() == 1 {
                    format!("{} -*> {}", out[0], out[0])
                } else {
                    let mut s = out[..out.len() - 1].join(" -> ");
                    s.push_str(" -*> ");
                    s.push_str(&out[0]);
                    s
                };
                println!("{:<pad$}{}", queue_label, joined, pad = pad);
                if count > truncate_count {
                    println!("{:>pad$} ... (truncated)", "", pad = pad);
                }
                println!("count: {}", count);
            }
        }
        println!("{:->width$}\n", "", width = pad + 30);
    }
}

impl<Key: Eq + Hash> AlsoCache<Key, DefaultWeighter, RandomState> {
    pub fn new(size: usize) -> Self {
        let weighter = DefaultWeighter;
        let hash_builder = RandomState::new();
        AlsoCache::with(size, weighter, hash_builder)
    }
}

#[inline(always)]
pub fn serialize<T: Serialize>(value: &T) -> Result<Vec<u8>, EncodeError> {
    bincode::serde::encode_to_vec(value, standard())
}

#[inline(always)]
pub fn deserialize<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, DecodeError> {
    bincode::serde::decode_from_slice::<T, _>(bytes, standard()).map(|(res, _)| res)
}
