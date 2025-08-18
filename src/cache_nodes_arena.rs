use std::marker::PhantomData;

#[derive(Debug, Clone, Copy)]
struct SmallQueue;
#[derive(Debug, Clone, Copy)]
struct MainQueue;
#[derive(Debug, Clone, Copy)]
struct GhostQueue;
#[derive(Debug, Clone, Copy)]
struct NoQueue;

trait MutableQueue {}
impl MutableQueue for SmallQueue {}
impl MutableQueue for MainQueue {}
impl MutableQueue for GhostQueue {}

#[derive(Debug, Clone, Copy)]
struct Occupied;
#[derive(Debug, Clone, Copy)]
struct Free;

#[derive(Debug, Clone)]
struct Node {
    next: usize,
    prev: usize,
    data: Vec<u8>,
    data_size: usize,
    freq: u16,
}

#[derive(Debug, Clone, Copy)]
pub struct NodeRef<Q, O> {
    idx: usize,
    _occupied: PhantomData<O>,
    _queue: PhantomData<Q>,
}

#[derive(Debug, Clone, Copy)]
enum QueueHead<Q> {
    Some(NodeRef<Q, Occupied>),
    None,
}

#[derive(Debug, Clone)]
pub struct NodeArena {
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

impl<Q, O> NodeRef<Q, O> {
    // /// `Put` new node after the current node.
    // pub fn insert_after(&self, nodes: &mut Vec<Node>, new_node: &NodeRef<Q, O>) {
    //     let next_idx = nodes[self.idx].next;
    //     nodes[next_idx].prev = new_node.idx;
    //     nodes[new_node.idx].next = next_idx;
    //     nodes[new_node.idx].prev = self.idx;
    //     nodes[self.idx].next = new_node.idx;
    // }
}

impl<Q: MutableQueue> NodeRef<Q, Occupied> {
    /// `Put` new node after the current node.
    pub fn insert_after(&self, nodes: &mut Vec<Node>, new_node: &NodeRef<Q, Occupied>) {
        let next_idx = nodes[self.idx].next;
        nodes[next_idx].prev = new_node.idx;
        nodes[new_node.idx].next = next_idx;
        nodes[new_node.idx].prev = self.idx;
        nodes[self.idx].next = new_node.idx;
    }

    pub fn unlink(&self, nodes: &mut Vec<Node>) -> NodeRef<NoQueue, Occupied> {
        // link the previous and next nodes to each other
        let next_idx = nodes[self.idx].next;
        let prev_idx = nodes[self.idx].prev;
        nodes[prev_idx].next = next_idx;
        nodes[next_idx].prev = prev_idx;

        // link to itself
        nodes[self.idx].next = self.idx;
        nodes[self.idx].prev = self.idx;

        NodeRef {
            idx: self.idx,
            _occupied: PhantomData,
            _queue: PhantomData::<NoQueue>,
        }
    }

    pub fn next(&self, nodes: &Vec<Node>) -> Option<NodeRef<Q, Occupied>> {
        if nodes[self.idx].next == self.idx {
            // if the next node is itself, it means it's the only node in the queue
            return None;
        }
        // otherwise, return the previous node
        let prev_idx = nodes[self.idx].next;
        Some(NodeRef {
            idx: prev_idx,
            _occupied: PhantomData,
            _queue: PhantomData::<Q>,
        })
    }

    pub fn prev(&self, nodes: &Vec<Node>) -> Option<NodeRef<Q, Occupied>> {
        if nodes[self.idx].prev == self.idx {
            // if the previous node is itself, it means it's the only node in the queue
            return None;
        }
        // otherwise, return the previous node
        let prev_idx = nodes[self.idx].prev;
        Some(NodeRef {
            idx: prev_idx,
            _occupied: PhantomData,
            _queue: PhantomData::<Q>,
        })
    }
}

impl NodeRef<NoQueue, Occupied> {
    pub fn move_to_queue<Q: MutableQueue>(
        self,
        nodes: &mut Vec<Node>,
        head: &mut QueueHead<Q>,
    ) -> NodeRef<Q, Occupied> {
        let new_queue_ref = NodeRef {
            idx: self.idx,
            _occupied: PhantomData,
            _queue: PhantomData::<Q>,
        };

        match head {
            QueueHead::Some(head_ref) => {
                // link to the head of the new (Q2) queue
                head_ref.insert_after(nodes, &new_queue_ref);
            }
            QueueHead::None => {
                // if the head of Q2 is None, we set new queue as the head
                *head = QueueHead::Some(new_queue_ref);
            }
        }

        NodeRef {
            idx: self.idx,
            _occupied: PhantomData,
            _queue: PhantomData::<Q>,
        }
    }

    pub fn evict(self, nodes: &mut Vec<Node>) -> NodeRef<NoQueue, Free> {
        nodes[self.idx].data = Vec::new();
        nodes[self.idx].data_size = 0;
        nodes[self.idx].freq = 0;
        nodes[self.idx].next = usize::MAX; // set to usize::MAX so any use as an index will panic
        nodes[self.idx].prev = usize::MAX;

        NodeRef {
            idx: self.idx,
            _occupied: PhantomData,
            _queue: PhantomData::<NoQueue>,
        }
    }
}

impl NodeRef<NoQueue, Free> {
    pub fn occupy(
        self,
        nodes: &mut Vec<Node>,
        data: Vec<u8>,
        data_size: usize,
    ) -> NodeRef<NoQueue, Occupied> {
        nodes[self.idx].data = data;
        nodes[self.idx].data_size = data_size;
        NodeRef {
            idx: self.idx,
            _occupied: PhantomData,
            _queue: PhantomData::<NoQueue>,
        }
    }
}

impl NodeArena {
    pub fn new(small_threshold: usize, main_threshold: usize, ghost_threshold: usize) -> Self {
        Self {
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

    pub fn allocate_small(
        &mut self,
        data: Vec<u8>,
        data_size: usize,
    ) -> NodeResult<SmallQueue, Occupied> {
        let new_node = self.create_node::<SmallQueue>(data, data_size);
        match &mut self.small_head {
            QueueHead::None => {
                self.small_head = QueueHead::Some(new_node);
            }
            QueueHead::Some(head) => {
                head.insert_after(&mut self.nodes, &new_node);
            }
        }
        self.small_size += data_size;
        NodeResult::new(new_node, self)
    }

    pub fn reinsert_main<'a>(
        &'a mut self,
        node_ref: NodeRef<MainQueue, Occupied>,
    ) -> NodeResult<'a, MainQueue, Occupied> {
        self.nodes[node_ref.idx].freq -= 1;
        let unlinked_ref = node_ref.unlink(&mut self.nodes);
        let main_ref =
            unlinked_ref.move_to_queue::<MainQueue>(&mut self.nodes, &mut self.main_head);
        NodeResult::new(main_ref, self)
    }

    // TODO: make so evict_small could be called only when small_size > small_threshold?
    // TODO: make so if it's impossible to call evict without existing head
    pub fn evict_small(&mut self) {
        if let QueueHead::Some(head_ref) = &self.small_head {
            let head_ref = head_ref.clone();
            match head_ref.prev(&self.nodes) {
                Some(prev_ref) => {
                    // if there is a previous node, set it as the new head
                    self.small_head = QueueHead::Some(prev_ref);
                }
                None => {
                    // if there is no next node, set the head to None
                    self.main_head = QueueHead::None;
                }
            }
            if self.nodes[head_ref.idx].freq > 0 {
                self.promote_small_to_main(head_ref);
            } else {
                self.demote_small_to_ghost(head_ref);
            }
        }
    }

    // TODO: remove code duplication with `evict_small` and `evict_main`
    pub fn evict_ghost(&mut self) {
        if let QueueHead::Some(head_ref) = &self.ghost_head {
            let head_ref = head_ref.clone();
            match head_ref.prev(&self.nodes) {
                Some(prev_ref) => {
                    // if there is a previous node, set it as the new head
                    self.ghost_head = QueueHead::Some(prev_ref);
                }
                None => {
                    // if there is no next node, set the head to None
                    self.main_head = QueueHead::None;
                }
            }
            self.ghost_size -= self.nodes[head_ref.idx].data_size; // TODO: size changes are thrown here and there, make it more consistent
            let unlinked_ref = head_ref.unlink(&mut self.nodes);
            let freed_node = unlinked_ref.evict(&mut self.nodes);
            self.freelist.push(freed_node);
        }
    }

    pub fn evict_main(&mut self) {
        if let QueueHead::Some(head_ref) = &self.main_head {
            let head_ref = head_ref.clone();
            match head_ref.prev(&self.nodes) {
                Some(prev_ref) => {
                    // if there is a previous node, set it as the new head
                    self.main_head = QueueHead::Some(prev_ref);
                }
                None => {
                    // if there is no next node, set the head to None
                    self.main_head = QueueHead::None;
                }
            }
            if self.nodes[head_ref.idx].freq > 0 {
                let _ = self.reinsert_main(head_ref);
            } else {
                self.main_size -= self.nodes[head_ref.idx].data_size;
                let unlinked_ref = head_ref.unlink(&mut self.nodes);
                let freed_node = unlinked_ref.evict(&mut self.nodes);
                self.freelist.push(freed_node);
            }
        }
    }

    pub fn get_idx_ref<Q: MutableQueue>(&self, idx: usize) -> Option<NodeRef<Q, Occupied>> {
        // continue here: how to know in which queue the node is?
        self.nodes.get(idx).map(|_| NodeRef {
            idx,
            _occupied: PhantomData,
            _queue: PhantomData::<Q>,
        })
    }

    fn promote_small_to_main<'a>(
        &'a mut self,
        node_ref: NodeRef<SmallQueue, Occupied>,
    ) -> NodeResult<'a, MainQueue, Occupied> {
        self.nodes[node_ref.idx].freq = 0;

        self.small_size -= self.nodes[node_ref.idx].data_size;
        self.main_size += self.nodes[node_ref.idx].data_size;

        let unlinked_ref = node_ref.unlink(&mut self.nodes);
        let main_ref =
            unlinked_ref.move_to_queue::<MainQueue>(&mut self.nodes, &mut self.main_head);
        NodeResult::new(main_ref, self)
    }

    fn demote_small_to_ghost<'a>(
        &'a mut self,
        node_ref: NodeRef<SmallQueue, Occupied>,
    ) -> NodeResult<'a, GhostQueue, Occupied> {
        self.small_size -= self.nodes[node_ref.idx].data_size;
        self.ghost_size += self.nodes[node_ref.idx].data_size;

        let unlinked_ref = node_ref.unlink(&mut self.nodes);
        let ghost_ref =
            unlinked_ref.move_to_queue::<GhostQueue>(&mut self.nodes, &mut self.ghost_head);

        self.nodes[ghost_ref.idx].data = Vec::new(); // Drop data for ghost nodes
        self.nodes[ghost_ref.idx].data_size = 0;

        NodeResult::new(ghost_ref, self)
    }

    fn promote_ghost_to_main<'a>(
        &'a mut self,
        ghost_head: NodeRef<GhostQueue, Occupied>,
        data: Vec<u8>,
        data_size: usize,
    ) -> NodeResult<'a, MainQueue, Occupied> {
        self.nodes[ghost_head.idx].freq = 0;

        self.ghost_size -= self.nodes[ghost_head.idx].data_size;
        self.main_size += self.nodes[ghost_head.idx].data_size;

        let unlinked_ref = ghost_head.unlink(&mut self.nodes);
        let main_ref =
            unlinked_ref.move_to_queue::<MainQueue>(&mut self.nodes, &mut self.main_head);

        self.nodes[main_ref.idx].data = data;
        self.nodes[main_ref.idx].data_size = data_size;

        NodeResult::new(main_ref, self)
    }

    fn create_node<Q>(&mut self, data: Vec<u8>, data_size: usize) -> NodeRef<Q, Occupied> {
        let idx = if let Some(freed_ref) = self.freelist.pop() {
            // reuse a freed node
            let occupied_ref = freed_ref.occupy(&mut self.nodes, data, data_size);
            occupied_ref.idx
        } else {
            // if no free index, we need to allocate a new node
            let new_idx = self.nodes.len();
            self.nodes.push(Node {
                next: new_idx,
                prev: new_idx,
                data,
                data_size,
                freq: 0,
            });
            new_idx
        };

        NodeRef {
            idx,
            _occupied: PhantomData,
            _queue: PhantomData,
        }
    }

    pub fn print_queues(&self, truncate_count: usize) {
        self.print_queue("Small", &self.small_head, truncate_count);
        self.print_queue("Main", &self.main_head, truncate_count);
        self.print_queue("Ghost", &self.ghost_head, truncate_count);
    }

    pub fn print_queue(
        &self,
        queue_name: &str,
        head: &QueueHead<impl MutableQueue + Copy>,
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
                let mut current_node = start.clone();
                let mut out = Vec::new();
                let mut count = 0;

                loop {
                    if count <= truncate_count {
                        out.push(format!("{}", current_node.idx));
                    }

                    match current_node.next(&self.nodes) {
                        Some(next_node) => {
                            current_node = next_node;
                            count += 1;
                            if count > 10_000 {
                                panic!("Too many elements in queue, something is wrong");
                            }
                        }
                        None => break,
                    }

                    // If we've looped back to the start, we're done
                    if current_node.idx == start.idx {
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

// fn unlink_from_queue<Queue>(nodes: &mut Vec<Node>, node_idx: NodeRef<Queue>) {
//     // link the previous and next to each other
//     let node = &mut self.nodes[node_idx];
//     let prev_idx = node.prev;
//     let next_idx = node.next;
//     self.nodes[prev_idx].next = next_idx;
//     self.nodes[next_idx].prev = prev_idx;

//     // link to itself
//     self.nodes[node_idx].next = node_idx;
//     self.nodes[node_idx].prev = node_idx;
// }

struct NodeResult<'a, Q, O> {
    node_ref: NodeRef<Q, O>,
    arena: &'a mut NodeArena,
}

impl<'a, Q, O> NodeResult<'a, Q, O> {
    fn new(node_ref: NodeRef<Q, O>, arena: &'a mut NodeArena) -> Self {
        Self { node_ref, arena }
    }

    fn and_then<R, F>(self, f: F) -> NodeResult<'a, R, O>
    where
        F: FnOnce(NodeRef<Q, O>, &mut NodeArena) -> NodeResult<'a, R, O>,
    {
        f(self.node_ref, self.arena)
    }

    // Helper to access the inner NodeRef
    fn get_ref(&self) -> &NodeRef<Q, O> {
        &self.node_ref
    }
}
