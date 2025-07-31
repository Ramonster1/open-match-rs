use crate::order_book::Order;
use std::collections::HashMap;
use std::mem;
use std::ptr::NonNull;

pub struct OrderNode {
    order: Order,
    prev: Option<NonNull<OrderNode>>,
    next: Option<NonNull<OrderNode>>,
}

// NotNull is used for references, Box used for heap storage solution
pub struct OrderNodeLinkedList {
    head: Option<NonNull<OrderNode>>,
    tail: Option<NonNull<OrderNode>>,
    index: HashMap<u64, NonNull<OrderNode>>, // instantly find any order by ID (for cancel/modify)
    arena: Vec<Box<OrderNode>>, // Keeps memory alive, stores all OrderNodes so raw pointers stay valid
    freelist: Vec<Box<OrderNode>>, // Reallocate/recycle cancelled or matched nodes
}

impl OrderNodeLinkedList {
    pub fn new() -> Self {
        Self {
            head: None,
            tail: None,
            index: HashMap::new(),
            arena: Vec::new(),
            freelist: Vec::new(),
        }
    }

    /// Insert a new order at the tail
    pub fn insert(&mut self, order: Order) {
        let order_id = order.id;
        // Create the node
        let mut boxed_order_node = if let Some(recycled_node) = self.freelist.pop() {
            let mut node = recycled_node;
            node.order = order;
            node.prev = None; // Reset its prev and next pointers so it doesn't point to old neighbors.
            node.next = None;
            node
        } else {
            // If no recycled node is available, allocate a new one on the heap using Box::new
            Box::new(OrderNode {
                order,
                prev: None, // Reset its prev and next pointers so it doesn't point to old neighbours.
                next: None,
            })
        };

        /*
            Convert the Box<OrderNode> into a raw pointer (NonNull<OrderNode>) that:
            Doesn't allow null
            Can be stored in the index or linked list
            This is unsafe because:
            We're telling Rust: “Trust me, this pointer is valid.”
            It must be kept alive (which we handle via arena).
        */
        let non_null_node_ptr =
            unsafe { NonNull::new_unchecked(boxed_order_node.as_mut() as *mut _) };

        // Link it
        match self.tail {
            None => self.head = Some(non_null_node_ptr),
            Some(mut old_tail) => unsafe {
                old_tail.as_mut().next = Some(non_null_node_ptr);
                boxed_order_node.prev = Some(old_tail);
            },
        }

        self.tail = Some(non_null_node_ptr);

        // Insert into order ID : order node map
        self.index.insert(order_id, non_null_node_ptr);

        self.arena.push(boxed_order_node); // Ownership stored here
    }

    /// Cancel an order by ID
    pub fn cancel(&mut self, order_id: u64) -> Option<Order> {
        let ptr = self.index.remove(&order_id)?;
        let node_ref = unsafe { ptr.as_ref() };

        // Update linked list
        unsafe {
            // Update previous node or update head if node is head node
            if let Some(mut prev) = node_ref.prev {
                prev.as_mut().next = node_ref.next;
            } else {
                self.head = node_ref.next;
            }

            // Update next node or update tail if node is tail node
            if let Some(mut next) = node_ref.next {
                next.as_mut().prev = node_ref.prev;
            } else {
                self.tail = node_ref.prev;
            }
        }

        self.remove_from_arena(ptr)
    }

    /// Internal: remove node from arena and add to free list
    fn remove_from_arena(&mut self, target: NonNull<OrderNode>) -> Option<Order> {
        if let Some(pos) = self
            .arena
            .iter()
            .position(|b| &**b as *const _ == target.as_ptr())
        {
            // Use swap_remove for O(1) performance
            let mut boxed = self.arena.swap_remove(pos);

            let order = mem::take(&mut boxed.order);
            self.freelist.push(boxed);
            Some(order)
        } else {
            None
        }
    }

    /// Remove and return order from head (used for matching)
    pub fn pop_head(&mut self) -> Option<Order> {
        let head_ptr = self.head?;
        let head_node_ref = unsafe { head_ptr.as_ref() };
        let next_node = head_node_ref.next;

        // Update head pointer
        self.head = next_node;
        if let Some(mut next_ptr) = next_node {
            unsafe {
                next_ptr.as_mut().prev = None;
            }
        } else {
            // List is now empty
            self.tail = None;
        }

        self.index.remove(&head_node_ref.order.id);
        self.remove_from_arena(head_ptr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::order_book::Side;
    use std::sync::atomic::AtomicU64;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn test_insert() {
        let mut book = OrderNodeLinkedList::new();

        let order_1 = Order {
            id: 1,
            price: 100,
            quantity: 1,
            side: Side::BUY,
            timestamp: AtomicU64::new(now_as_nanos()),
        };

        let order_2 = Order {
            id: 2,
            price: 100,
            quantity: 1,
            side: Side::BUY,
            timestamp: AtomicU64::new(now_as_nanos()),
        };

        book.insert(order_1);
        book.insert(order_2);

        // Check arena contains both orders
        assert_eq!(book.arena.len(), 2);
        assert_eq!(book.freelist.len(), 0);

        // Check index contains both IDs
        let node_1_ptr = book.index.get(&1).copied().expect("order 1 not found");
        let node_2_ptr = book.index.get(&2).copied().expect("order 2 not found");

        // Check correct order in list (order_1 → order_2)
        unsafe {
            let node_1 = node_1_ptr.as_ref();
            let node_2 = node_2_ptr.as_ref();

            assert_eq!(node_1.order.id, 1);
            assert_eq!(node_2.order.id, 2);

            assert_eq!(node_1.next, Some(node_2_ptr));
            assert_eq!(node_2.prev, Some(node_1_ptr));
            assert!(node_2.next.is_none());
            assert!(node_1.prev.is_none());
        }

        // Check head and tail are correctly updated
        assert_eq!(book.head, Some(node_1_ptr));
        assert_eq!(book.tail, Some(node_2_ptr));
    }

    #[test]
    fn test_cancel() {
        let mut book = OrderNodeLinkedList::new();

        let order_1 = Order {
            id: 1,
            price: 100,
            quantity: 1,
            side: Side::BUY,
            timestamp: AtomicU64::new(now_as_nanos()),
        };

        let order_2 = Order {
            id: 2,
            price: 100,
            quantity: 1,
            side: Side::BUY,
            timestamp: AtomicU64::new(now_as_nanos()),
        };

        let order_3 = Order {
            id: 3,
            price: 100,
            quantity: 1,
            side: Side::BUY,
            timestamp: AtomicU64::new(now_as_nanos()),
        };

        book.insert(order_1);
        book.insert(order_2);
        book.insert(order_3);

        book.cancel(1);

        // Check arena only contains the non-cancelled orders
        assert_eq!(book.arena.len(), 2);
        assert_eq!(book.freelist.len(), 1);

        // Check index contains the non-cancelled orders
        let node_2_ptr = book.index.get(&2).copied().expect("order 2 not found");
        let node_3_ptr = book.index.get(&3).copied().expect("order 3 not found");

        // Check correct order in list (order_2 → order_2)
        unsafe {
            let node_2 = node_2_ptr.as_ref();
            let node_3 = node_3_ptr.as_ref();

            assert_eq!(node_2.order.id, 2);
            assert_eq!(node_3.order.id, 3);

            assert_eq!(node_2.next, Some(node_3_ptr));
            assert!(node_2.prev.is_none());

            assert!(node_3.next.is_none());
            assert_eq!(node_3.prev, Some(node_2_ptr));
        }

        // Check head and tail are correctly updated
        assert_eq!(book.head, Some(node_2_ptr));
        assert_eq!(book.tail, Some(node_3_ptr));
    }

    #[test]
    fn test_pop_head() {
        let mut book = OrderNodeLinkedList::new();

        let order_1 = Order {
            id: 1,
            price: 100,
            quantity: 1,
            side: Side::BUY,
            timestamp: AtomicU64::new(now_as_nanos()),
        };

        let order_2 = Order {
            id: 2,
            price: 100,
            quantity: 1,
            side: Side::BUY,
            timestamp: AtomicU64::new(now_as_nanos()),
        };

        let order_3 = Order {
            id: 3,
            price: 100,
            quantity: 1,
            side: Side::BUY,
            timestamp: AtomicU64::new(now_as_nanos()),
        };

        book.insert(order_1);
        book.insert(order_2);
        book.insert(order_3);

        assert_eq!(book.freelist.len(), 0);
        book.pop_head();

        // Check arena and freelist sizes
        assert_eq!(book.arena.len(), 2);
        assert_eq!(book.freelist.len(), 1);
    }

    fn now_as_nanos() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_nanos() as u64
    }
}
