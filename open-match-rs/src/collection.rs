use crate::order_book::Order;
use std::collections::HashMap;
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

        // Check index contains both IDs
        let node_1_ptr = book.index.get(&1).copied().expect("order 1 not found");
        let node_2_ptr = book.index.get(&2).copied().expect("order 2 not found");

        // ✅ Check correct order in list (order_1 → order_2)
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

        // ✅ Check head and tail are correctly updated
        assert_eq!(book.head, Some(node_1_ptr));
        assert_eq!(book.tail, Some(node_2_ptr));
    }

    fn now_as_nanos() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_nanos() as u64
    }
}
