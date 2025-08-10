use std::{collections::HashMap, hash::Hash};

#[derive(Debug, Clone, Copy)]
struct Node {
    next: usize,
    prev: usize,
    data_ind: usize,
    frequency: u16,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum FifoName {
    Small,
    Ghost,
    Main,
}

pub struct Cache<Key> {
    // broken: index shift after removal, all nodes and nodes_keys, nodes_fifo become invalid
    key_to_node: HashMap<Key, usize>,
    nodes: Vec<Option<Node>>, // need to access nodes in O(1), reads a frequent
    nodes_keys: Vec<Key>,
    nodes_fifo: Vec<FifoName>,
    data: Vec<Vec<u8>>,

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
}

// TODO: remove clone from key
impl<Key: Eq + Hash + Clone> Cache<Key> {
    pub fn new(size: usize) -> Self {
        Cache {
            key_to_node: HashMap::with_capacity(0),
            nodes_keys: Vec::with_capacity(0),
            nodes_fifo: Vec::with_capacity(0),
            data: Vec::with_capacity(0),
            nodes: Vec::with_capacity(0),
            small_head: None,
            main_head: None,
            ghost_head: None,
            small_threshold: size * 0.1 as usize,
            main_threshold: size,
            ghost_threshold: size * 0.4 as usize,
            small_size: 0,
            main_size: 0,
            ghost_size: 0,
        }
    }

    pub fn get_bytes(&mut self, key: &Key) -> Option<&Vec<u8>> {
        let node_idx = *self.key_to_node.get(key)?;
        let data_idx = self.nodes[node_idx]
            .expect("node_idx pointing to existing node in nodes") //TODO: enforce this with types?
            .data_ind;
        self.node_read(node_idx);
        Some(self.data[data_idx].as_ref())
    }

    pub fn insert_bytes(&mut self, key: &Key, val: Vec<u8>) {
        let data_idx = self.data.len();
        let (node_idx, node) = self.insert_node(Node {
            next: 0,
            prev: 0,
            data_ind: data_idx,
            frequency: 0,
        });

        node.next = node_idx;
        node.prev = node_idx;

        self.nodes_keys[node_idx] = key.clone();
        self.nodes_fifo[node_idx] = FifoName::Small;
        self.data[node_idx] = val;

        if let Some(existing_idx) = self.key_to_node.insert(key.clone(), node_idx) {
            self.remove_from_queue(existing_idx);
            self.free_node(existing_idx);
        }

        self.node_advance(FifoName::Small, node_idx);
    }

    fn node_read(&mut self, node_idx: usize) {
        let mut node = self.nodes[node_idx].expect("node_idx pointing to existing node in nodes");
        match self.nodes_fifo[node_idx] {
            FifoName::Ghost => {
                self.node_advance(FifoName::Main, node_idx);
            }
            _ => {
                if node.frequency < 3 {
                    node.frequency += 1;
                }
            }
        }
    }

    fn node_advance(&mut self, queue_name: FifoName, node_idx: usize) {
        let capacity_reached = match queue_name {
            FifoName::Small => self.small_size >= self.small_threshold,
            FifoName::Main => self.main_size >= self.main_threshold,
            FifoName::Ghost => self.ghost_size >= self.ghost_threshold,
        };
        let data_ind = self.nodes[node_idx]
            .expect("node_idx pointing to existing node in nodes")
            .data_ind;
        let new_item_size = self.data.get(data_ind).map_or(0, |v| v.len());
        match (queue_name, capacity_reached) {
            (FifoName::Small, true) => {
                self.node_evict(queue_name);
            }
            (FifoName::Main, true) => {
                self.node_evict(queue_name);
            }
            (FifoName::Ghost, true) => {
                self.node_evict(queue_name);
            }
            (FifoName::Small, false) => {
                self.small_size += new_item_size;
                if let Some(head) = self.small_head {
                    self.put_into_queue(node_idx, head);
                } else {
                    self.small_head = Some(node_idx);
                }
            }
            (FifoName::Main, false) => {
                self.main_size += new_item_size;
                if let Some(head) = self.main_head {
                    self.put_into_queue(node_idx, head);
                } else {
                    self.main_head = Some(node_idx);
                }
                let node_fifo = &self.nodes_fifo[node_idx];
                match node_fifo {
                    FifoName::Small => {
                        self.small_size -= new_item_size;
                    }
                    FifoName::Ghost => {
                        self.ghost_size -= new_item_size;
                    }
                    _ => {}
                }
            }
            (FifoName::Ghost, false) => {
                self.ghost_size += new_item_size;
                if let Some(head) = self.ghost_head {
                    self.put_into_queue(node_idx, head);
                } else {
                    self.ghost_head = Some(node_idx);
                }
                self.small_size -= new_item_size;
            }
        }
        self.nodes_fifo[node_idx] = queue_name;
    }

    fn node_evict(&mut self, queue_name: FifoName) {
        match queue_name {
            FifoName::Small => {
                let mut ind = self
                    .small_head
                    .expect("small_head to exist when evicting from small queue");
                while self.small_size > self.small_threshold {
                    // Make previous node the new head
                    let prev_node = self.nodes[ind]
                        .as_ref()
                        .expect("node_idx pointing to existing node in nodes")
                        .prev;
                    self.small_head = Some(prev_node);

                    // Advance old head (move to main or to ghost) and update frequency
                    let (freq, data_ind) = {
                        let node = self.nodes[ind]
                            .as_mut()
                            .expect("ind pointing to existing node in nodes");
                        if node.frequency > 0 {
                            node.frequency -= 1;
                        }
                        (node.frequency, node.data_ind)
                    };

                    if freq > 0 {
                        self.node_advance(FifoName::Main, ind);
                    } else {
                        self.node_advance(FifoName::Ghost, ind);
                    }

                    self.small_size -= self.data[data_ind].len();
                    ind = prev_node;
                }
            }
            FifoName::Main => {
                let mut ind = self
                    .main_head
                    .expect("main_head to exist when evicting from main queue");
                while self.main_size > self.main_threshold {
                    // Make previous node the new head
                    let prev_node = self.nodes[ind]
                        .expect("node_idx pointing to existing node in nodes")
                        .prev;
                    self.main_head = Some(prev_node);

                    // Advance old head (move to the beginning of the main or remove)
                    self.remove_from_queue(ind);
                    let (freq, data_ind) = {
                        let node = self.nodes[ind]
                            .as_mut()
                            .expect("ind pointing to existing node in nodes");
                        if node.frequency > 0 {
                            node.frequency -= 1;
                        }
                        (node.frequency, node.data_ind)
                    };
                    if freq > 0 {
                        self.put_into_queue(ind, prev_node);
                    } else {
                        self.free_node(ind);
                        self.main_size -= self.data[data_ind].len();
                    }

                    ind = prev_node;
                }
            }
            FifoName::Ghost => {
                let mut ind = self
                    .ghost_head
                    .expect("ghost_head to exist when evicting from ghost queue");
                while self.ghost_size > self.ghost_threshold {
                    // Make previous node the new head
                    let prev_node = self.nodes[ind]
                        .expect("node_idx pointing to existing node in nodes")
                        .prev;
                    self.ghost_head = Some(prev_node);

                    // Advance old head (move to main or remove)
                    self.remove_from_queue(ind);
                    let (freq, data_ind) = {
                        let node = self.nodes[ind]
                            .as_mut()
                            .expect("ind pointing to existing node in nodes");
                        if node.frequency > 0 {
                            node.frequency -= 1;
                        }
                        (node.frequency, node.data_ind)
                    };
                    if freq > 0 {
                        self.node_advance(FifoName::Main, ind);
                    } else {
                        self.free_node(ind);
                        self.ghost_size -= self.data[data_ind].len();
                    }

                    ind = prev_node;
                }
            }
        }
    }

    // TODO: this sucks
    fn insert_node(&mut self, new_node: Node) -> (usize, &mut Node) {
        if let Some(i) = self.nodes.iter().position(|n| n.is_none()) {
            self.nodes[i] = Some(new_node);
            return (i, self.nodes[i].as_mut().unwrap());
        }
        self.nodes.push(Some(new_node));
        let i = self.nodes.len() - 1;
        (i, self.nodes[i].as_mut().unwrap())
    }

    fn free_node(&mut self, node_idx: usize) {
        let node = &mut self.nodes[node_idx].expect("node_idx pointing to existing node in nodes");
        self.data[node.data_ind].clear();
        self.key_to_node.remove(&self.nodes_keys[node_idx]);
        self.nodes[node_idx] = None;
        self.nodes_fifo[node_idx] = FifoName::Small;
    }

    fn remove_from_queue(&mut self, node_idx: usize) {
        let node = &mut self.nodes[node_idx].expect("node_idx pointing to existing node in nodes");
        let prev_idx = node.prev;
        let next_idx = node.next;
        self.nodes[prev_idx]
            .expect("prev_idx pointing to existing node in nodes")
            .next = next_idx;
        self.nodes[next_idx]
            .expect("next_idx pointing to existing node in nodes")
            .prev = prev_idx;
    }

    // TODO: this sucks
    fn put_into_queue(&mut self, node_idx: usize, head_idx: usize) {
        let tail_idx = self.nodes[head_idx]
            .expect("node_idx pointing to existing node in nodes")
            .next;
        self.nodes[tail_idx]
            .expect("node_idx pointing to existing node in nodes")
            .prev = node_idx;
        self.nodes[node_idx]
            .expect("node_idx pointing to existing node in nodes")
            .prev = head_idx;
        self.nodes[node_idx]
            .expect("node_idx pointing to existing node in nodes")
            .next = tail_idx;
        self.nodes[head_idx]
            .expect("head_idx pointing to existing node in nodes")
            .next = node_idx;
    }

    // fn node_advance(&mut self, queue_name: QueueName, node_ind: usize) {
    //     let (queue_size, queue_capacity, queue_head) = match queue_name {
    //         QueueName::Small => (
    //             &mut self.small_size,
    //             self.small_capacity,
    //             &mut self.small_head,
    //         ),
    //         QueueName::Main => (&mut self.main_size, self.main_capacity, &mut self.main_head),
    //         QueueName::Ghost => (
    //             &mut self.ghost_size,
    //             self.ghost_capacity,
    //             &mut self.ghost_head,
    //         ),
    //     };

    //     let new_item_size = self
    //         .data
    //         .get(self.nodes[node_ind].data_ind)
    //         .map_or(0, |v| v.len());

    //     if *queue_size >= queue_capacity {
    //         return self.node_evict_until(queue_name, node_ind, new_item_size);
    //     }

    //     *queue_size += new_item_size;
    //     if let Some(head_idx) = *queue_head {
    //         self.put_into_queue(node_ind, head_idx);
    //     }
    //     *queue_head = Some(node_ind);
    // }

    // fn small_insert(&mut self, key: String, val: Vec<u8>) {
    //     if self.small_num >= self.small_capacity {
    //         if let Some(tail) = self
    //             .small_head
    //             .as_mut()
    //             .and_then(|h| h.prev)
    //             .and_then(|tail_key| self.key_to_node.get_mut(&tail_key))
    //         {
    //             if tail.frequency > 0 {
    //                 if let Some(main_head) = &mut self.main_head {
    //                     self.put_into_queue(&e, &mut hot_head);
    //                 } else {
    //                     self.small_head = Some(e);
    //                 }
    //             } else {
    //                 if let Some(hot_head) = &mut self.small_head {
    //                     self.put_into_queue(&e, &mut hot_head);
    //                 } else {
    //                     self.small_head = Some(e);
    //                 }
    //             }
    //         }
    //     }

    //     let entry = Node {
    //         next: None,
    //         prev: None,
    //         queue_name: QueueName::Main,
    //         val,
    //         frequency: 1,
    //     };

    //     self.key_to_node.insert(key, entry);
    // }

    // fn main_insert(&mut self, key: String, val: Vec<u8>) {
    //     todo!()
    // }

    // pub fn insert_bytes(&mut self, key: String, val: Vec<u8>) {
    //     if self.key_to_node.contains_key(&key) {
    //         if let Some(e) = self.key_to_node.get_mut(&key) {
    //             e.val = val;
    //         }
    //         return;
    //     }
    // }
}
