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
    data_size: usize,
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
            main_threshold: (size as f64 * 0.9) as usize,
            ghost_threshold: (size as f64 * 0.9) as usize,
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

        if self.nodes[node_idx].frequency < 3 {
            self.nodes[node_idx].frequency += 1;
        }

        if self.nodes[node_idx].fifo != FifoName::Ghost {
            Some(&self.nodes[node_idx].data)
        } else {
            None
        }
    }

    pub fn insert_bytes(&mut self, key: Key, val: Vec<u8>) {
        let hash = self.hasher.hash_one(&key);

        if let Some(&existing_idx) = self.map.find(hash, |&idx| self.nodes_keys[idx] == key) {
            self.nodes[existing_idx].data = val;
            if self.nodes[existing_idx].frequency < 3 {
                self.nodes[existing_idx].frequency += 1;
            }
            // if key is in ghost, put data into it and promote to main queue
            if self.nodes[existing_idx].fifo == FifoName::Ghost {
                self.node_advance(existing_idx, FifoName::Main, false);
            }
            return;
        }

        // if no node with the hash was found, create new node
        let node = Node {
            next: 0,
            prev: 0,
            data_size: self.weighter.weight(&val),
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
        self.node_advance(idx, FifoName::Small, true);
    }

    // TODO: so currently the solution is kind of works, but the flow and state mutations are
    // really ambiguous, because of recursion calls depending on the exact state of the cache
    // (like was some node unlinked before / size subtracted/etc. This requires rewrite)
    fn node_advance(&mut self, node_idx: usize, queue_name: FifoName, new_node: bool) {
        if !new_node {
            // check if the node was the head of its current queue
            // if yes, update the head to the next node
            match self.nodes[node_idx].fifo {
                FifoName::Small => {
                    if self.small_head == Some(node_idx) {
                        self.small_head = Some(self.nodes[node_idx].prev);
                    }
                }
                FifoName::Main => {
                    if self.main_head == Some(node_idx) {
                        self.main_head = Some(self.nodes[node_idx].prev);
                    }
                }
                FifoName::Ghost => {
                    if self.ghost_head == Some(node_idx) {
                        self.ghost_head = Some(self.nodes[node_idx].prev);
                    }
                }
            }

            // unlink node from its current queue
            self.unlink_from_queue(node_idx);
        }

        // TODO: ugly
        if queue_name == FifoName::Ghost {
            // drop data from node which goes into ghost (data will be inserted again if this node is accessed):
            self.nodes[node_idx].data = Vec::new();
        }

        // set the node's fifo to the new queue
        self.nodes[node_idx].fifo = queue_name;

        // insert node at the front of the queue
        let head = match queue_name {
            FifoName::Small => &mut self.small_head,
            FifoName::Main => &mut self.main_head,
            FifoName::Ghost => &mut self.ghost_head,
        };
        if let Some(head_idx) = *head {
            self.put_into_queue(node_idx, head_idx);
        } else {
            *head = Some(node_idx); // TODO: ugly
            match queue_name {
                FifoName::Small => self.small_size += self.nodes[node_idx].data_size,
                FifoName::Main => self.main_size += self.nodes[node_idx].data_size,
                FifoName::Ghost => self.ghost_size += self.nodes[node_idx].data_size,
            }
        }

        // evict if the queue exceeds its threshold after insertion
        let (size, threshold) = match queue_name {
            FifoName::Small => (self.small_size, self.small_threshold),
            FifoName::Main => (self.main_size, self.main_threshold),
            FifoName::Ghost => (self.ghost_size, self.ghost_threshold),
        };

        // introduce batching instead
        if size > threshold {
            self.node_evict(queue_name);
        }
    }

    fn node_evict(&mut self, queue_name: FifoName) {
        let (mut size, threshold, head) = match queue_name {
            FifoName::Small => (self.small_size, self.small_threshold, self.small_head),
            FifoName::Main => (self.main_size, self.main_threshold, self.main_head),
            FifoName::Ghost => (self.ghost_size, self.ghost_threshold, self.ghost_head),
        };
        let mut head = head.expect("head to exist when evicting");

        while size > threshold {
            // if the head is the only node in the queue, drop it and exit
            if self.nodes[head].prev == head && self.nodes[head].next == head {
                match queue_name {
                    FifoName::Small => self.small_head = None,
                    FifoName::Main => self.main_head = None,
                    FifoName::Ghost => self.ghost_head = None,
                }
                self.drop_node(head);
                break;
            }

            // make the second oldest node a new head (prev of head)
            let new_head = self.nodes[head].prev;
            match queue_name {
                FifoName::Small => self.small_head = Some(new_head),
                FifoName::Main => self.main_head = Some(new_head),
                FifoName::Ghost => self.ghost_head = Some(new_head),
            }

            // advance or remove old head
            match (queue_name, self.nodes[head].frequency > 0) {
                (FifoName::Small, true) => {
                    self.nodes[head].frequency = 0;
                    self.node_advance(head, FifoName::Main, false);
                }
                (FifoName::Small, false) => {
                    self.node_advance(head, FifoName::Ghost, false);
                }
                (FifoName::Ghost, true) => {
                    self.nodes[head].frequency = 0;
                    self.node_advance(head, FifoName::Main, false);
                }
                (FifoName::Ghost, false) => {
                    self.unlink_from_queue(head);
                    self.drop_node(head);
                }
                (FifoName::Main, true) => {
                    self.nodes[head].frequency -= 1;
                    self.unlink_from_queue(head);
                    self.put_into_queue(head, new_head);
                }
                (FifoName::Main, false) => {
                    self.unlink_from_queue(head);
                    self.drop_node(head);
                    // if let Some(head_idx) = self.ghost_head {
                    //     if self.nodes[head_idx].next == usize::MAX {
                    //         eprintln!();
                    //         eprintln!("AlsoCache state:");
                    //         eprintln!("  small_head: {:?}", self.small_head);
                    //         eprintln!("  main_head: {:?}", self.main_head);
                    //         eprintln!("  ghost_head: {:?}", self.ghost_head);
                    //         eprintln!();
                    //         eprintln!("  small_threshold: {}", self.small_threshold);
                    //         eprintln!("  main_threshold: {}", self.main_threshold);
                    //         eprintln!("  ghost_threshold: {}", self.ghost_threshold);
                    //         eprintln!();
                    //         eprintln!("  small_size: {}", self.small_size);
                    //         eprintln!("  main_size: {}", self.main_size);
                    //         eprintln!("  ghost_size: {}", self.ghost_size);
                    //         eprintln!();
                    //         eprintln!("  nodes.len(): {}", self.nodes.len());
                    //         eprintln!("  nodes_freelist: {:?}", self.nodes_freelist);
                    //         eprintln!("  nodes_keys.len(): {}", self.nodes_keys.len());
                    //         eprintln!("  map.len(): {}", self.map.len());
                    //         eprintln!();
                    //         eprintln!("  Nodes:");
                    //         for (i, node) in self.nodes.iter().enumerate() {
                    //             if i == 711 || i == 158 || i == 1682 || i == 342 {
                    //                 eprintln!(
                    //                     "    [{}] next: {}, prev: {}, data_size: {}, frequency: {}, fifo: {:?}",
                    //                     i,
                    //                     node.next,
                    //                     node.prev,
                    //                     node.data_size,
                    //                     node.frequency,
                    //                     node.fifo
                    //                 );
                    //             }
                    //         }
                    //         eprintln!();
                    //         eprint!(
                    //             " head_idx: {}, head: {}, new_head: {}",
                    //             head, head_idx, new_head
                    //         );

                    //         panic!(
                    //             "Ghost head index must be valid in 7, {}",
                    //             head == head_idx && head != new_head
                    //         );
                    //     }
                    // }
                }
            }

            size = match queue_name {
                FifoName::Small => self.small_size,
                FifoName::Main => self.main_size,
                FifoName::Ghost => self.ghost_size,
            };
            head = new_head;
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
            data_size: 0,
            frequency: 0,
            fifo: FifoName::Small,
        };

        // put index into freelist
        self.nodes_freelist.push(node_idx);
    }

    fn unlink_from_queue(&mut self, node_idx: usize) {
        // decrement the size of the queue
        let size = match self.nodes[node_idx].fifo {
            FifoName::Small => &mut self.small_size,
            FifoName::Main => &mut self.main_size,
            FifoName::Ghost => &mut self.ghost_size,
        };
        *size -= self.nodes[node_idx].data_size;

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
        // increment the size of the head's queue
        let size = match self.nodes[head_idx].fifo {
            FifoName::Small => &mut self.small_size,
            FifoName::Main => &mut self.main_size,
            FifoName::Ghost => &mut self.ghost_size,
        };
        *size += self.nodes[node_idx].data_size;

        // link the node to the head of the queue
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
                    if count > 10_000 {
                        panic!("Too many elements in queue, something is wrong");
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
