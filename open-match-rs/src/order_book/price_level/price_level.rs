use crate::domain::commands::{PartialFillRequest, UpdateQtyRequest};
use crate::domain::errors::OrderBookError;
use crate::domain::errors::OrderBookError::OrderNotInArena;
use crate::domain::responses::CommandResult::{Acknowledged, Rejected, Removed};
use crate::domain::responses::{
    CommandResult, FillInfo, FilledInfo, PartialFillInfo, QtyUpdatedInfo, RemoveInfo, TimedResult,
};
use crate::order_book::price_level::resting_order_node::{OrderNode, RestingOrder};
use crate::order_book::price_level::requests::EnqueueOrder;
use crate::time::TimeSource;
use quanta::IntoNanoseconds;
use std::collections::HashMap;
use std::ptr::NonNull;
use crate::order_book::order::Side;

// NotNull is used for references, Box used for heap storage solution
pub struct PriceLevelQueue<C: TimeSource> {
    price: u64,
    side: Side,
    clock: C,
    head: Option<NonNull<OrderNode>>,
    tail: Option<NonNull<OrderNode>>,
    index: HashMap<u64, NonNull<OrderNode>>, // instantly find any order by ID (for cancel/modify)
    arena: Vec<Box<OrderNode>>, // Keeps memory alive, stores all OrderNodes so raw pointers stay valid
    freelist: Vec<Box<OrderNode>>, // Reallocate/recycle cancelled or matched nodes
}

impl<C: TimeSource> PriceLevelQueue<C> {
    pub fn new(price: u64, side: Side, clock: C) -> Self {
        Self {
            price,
            side,
            clock,
            head: None,
            tail: None,
            index: HashMap::new(),
            arena: Vec::new(),
            freelist: Vec::new(),
        }
    }

    #[inline]
    pub fn price(&self) -> u64 {
        self.price
    }

    /// Insert a new order at the tail
    pub fn insert(&mut self, new_order_request: EnqueueOrder) -> TimedResult {
        let order_id = new_order_request.order_id;

        if self.index.contains_key(&order_id) {
            return TimedResult {
                timestamp_ns: self.clock.now(),
                result: Rejected(OrderBookError::DuplicateOrderId),
            };
        }

        let now = self.clock.now();

        let ro = RestingOrder {
            id: order_id,
            qty: new_order_request.qty,
            timestamp_ns: now,
        };

        // Create the node
        let mut boxed_order_node = if let Some(recycled_node) = self.freelist.pop() {
            let mut node = recycled_node;
            node.ro = ro;
            node.prev = None; // Reset its prev and next pointers so it doesn't point to old neighbours.
            node.next = None;
            node
        } else {
            // If no recycled node is available, allocate a new one on the heap using Box::new
            Box::new(OrderNode {
                ro,
                prev: None, // Reset its prev and next pointers so it doesn't point to old neighbours.
                next: None,
            })
        };

        /*
            Convert the Box<OrderNode> into a raw pointer (NonNull<OrderNode>) that:
            Doesn't allow null
            Can be stored in the index or linked list.
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

        TimedResult {
            timestamp_ns: now,
            result: Acknowledged,
        }
    }

    /// Remove and return order from head (used for matching)
    pub fn pop_head(&mut self) -> CommandResult {
        let Some(head_ptr) = self.head else {
            return Rejected(OrderBookError::EmptyPriceLevel);
        };

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

        let order_id = head_node_ref.ro.id;
        let filled_qty = head_node_ref.ro.qty;

        self.index.remove(&head_node_ref.ro.id);
        self.remove_from_arena(head_ptr);

        CommandResult::Filled(FillInfo::Full(FilledInfo {
            order_id,
            filled_qty,
        }))
    }

    /// Remove an order by ID
    pub fn remove(&mut self, order_id: u64) -> TimedResult {
        let Some(mut ptr) = self.index.remove(&order_id) else {
            return TimedResult {
                timestamp_ns: self.clock.now(),
                result: Rejected(OrderBookError::OrderNotFound),
            };
        };

        // Safety: ptr comes from arena, guaranteed valid while in arena
        let node = unsafe { ptr.as_mut() };

        // Update linked list
        // Update previous node or update head if node is head node
        if let Some(mut prev) = node.prev {
            unsafe {
                prev.as_mut().next = node.next;
            }
        } else {
            self.head = node.next;
        }

        // Update next node or update tail if node is tail node
        if let Some(mut next) = node.next {
            unsafe {
                next.as_mut().prev = node.prev;
            }
        } else {
            self.tail = node.prev;
        }

        let canceled_quantity = node.ro.qty;

        // Remove from arena (dealloc/recycle)
        self.remove_from_arena(ptr);

        TimedResult {
            timestamp_ns: self.clock.now(),
            result: Removed(RemoveInfo {
                order_id,
                canceled_quantity,
            }),
        }
    }

    pub fn update_quantity(&mut self, req: UpdateQtyRequest) -> TimedResult {
        let Some(ptr) = self.index.get_mut(&req.order_id) else {
            return TimedResult {
                timestamp_ns: self.clock.now(),
                result: Rejected(OrderBookError::OrderNotFound),
            };
        };

        let node = unsafe { ptr.as_mut() };
        node.ro.qty = req.new_qty;

        TimedResult {
            timestamp_ns: self.clock.now(),
            result: CommandResult::QuantityUpdated(QtyUpdatedInfo {
                order_id: req.order_id,
                new_qty: req.new_qty,
            }),
        }
    }

    fn apply_partial_fill(&mut self, req: PartialFillRequest) -> TimedResult {
        let Some(mut head_ptr) = self.head else {
            return TimedResult {
                timestamp_ns: self.clock.now(),
                result: Rejected(OrderBookError::EmptyPriceLevel),
            };
        };

        let head_node_ref = unsafe { head_ptr.as_mut() };

        if req.qty_to_fill >= head_node_ref.ro.qty {
            return TimedResult {
                timestamp_ns: self.clock.now(),
                result: Rejected(OrderBookError::InsufficientQuantity),
            };
        }

        head_node_ref.ro.qty -= req.qty_to_fill;

        TimedResult {
            timestamp_ns: self.clock.now(),
            result: CommandResult::Filled(FillInfo::Partial(PartialFillInfo {
                order_id: head_node_ref.ro.id,
                filled_qty: req.qty_to_fill,
                remaining_qty: head_node_ref.ro.qty,
            })),
        }
    }

    fn link_at_tail() {
        todo!(
            "Update pointers at tail, this action is performed in insert and replace_order, similar duplication in cancel"
        )
    }

    /// Internal: remove node from arena and add to free list
    fn remove_from_arena(&mut self, target: NonNull<OrderNode>) -> CommandResult {
        if let Some(pos) = self
            .arena
            .iter()
            .position(|b| &**b as *const _ == target.as_ptr())
        {
            // Use swap_remove for O(1) performance
            let mut boxed = self.arena.swap_remove(pos);

            self.freelist.push(boxed);
            Acknowledged
        } else {
            Rejected(OrderNotInArena)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::errors::OrderBookError::EmptyPriceLevel;
    use crate::order_book::order::Side;
    use crate::order_book::price_level::requests::EnqueueOrder;
    use crate::time::mock_clock::MockClock;

    #[test]
    fn test_insert() {
        let mut price_queue =
            PriceLevelQueue::new(100, Side::BUY, MockClock { fixed: 123_456_789 });

        const DEFAULT_QTY: u64 = 1;

        let order_1 = create_new_order_request(1, DEFAULT_QTY);
        let order_2 = create_new_order_request(2, DEFAULT_QTY);
        let order_3 = create_new_order_request(3, DEFAULT_QTY);

        price_queue.insert(order_1);

        // Check arena only contains the new order
        assert_eq!(price_queue.arena.len(), 1);
        assert_eq!(price_queue.freelist.len(), 0);

        // Check node references are None
        let node_1_ptr = price_queue
            .index
            .get(&1)
            .copied()
            .expect("order 1 not found");

        let node_1 = unsafe { node_1_ptr.as_ref() };

        assert_eq!(node_1.ro.id, 1);
        assert!(node_1.next.is_none());
        assert!(node_1.prev.is_none());

        // Check head and tail are set
        assert_eq!(
            price_queue.head,
            Some(price_queue.index.get(&1).copied().expect("Head incorrect"))
        );
        assert_eq!(
            price_queue.tail,
            Some(price_queue.index.get(&1).copied().expect("Tail incorrect"))
        );

        // Insert another order and check tail is updated and node references are correct
        price_queue.insert(order_2);

        // Check arena only contains the new order
        assert_eq!(price_queue.arena.len(), 2);
        assert_eq!(price_queue.freelist.len(), 0);

        let node_2_ptr = price_queue.index.get(&2).copied().unwrap();
        let node_2 = unsafe { node_2_ptr.as_ref() };
        assert_eq!(node_2.ro.id, 2, "Order ID incorrect");

        // Check node 1 links are correct
        assert_eq!(node_1.next, Some(node_2_ptr));
        assert!(node_1.prev.is_none());

        // Check node 2 links are correct
        assert_eq!(node_2.next, None, "Order 2 next incorrect");
        assert_eq!(node_2.prev, Some(node_1_ptr), "Order 2 prev incorrect");

        assert_eq!(
            price_queue.head,
            Some(price_queue.index.get(&1).copied().expect("Head incorrect"))
        );
        assert_eq!(
            price_queue.tail,
            Some(price_queue.index.get(&2).copied().expect("Tail incorrect"))
        );

        price_queue.insert(order_3);

        // Check arena only contains the new order
        assert_eq!(price_queue.arena.len(), 3);
        assert_eq!(price_queue.freelist.len(), 0);

        let node_3_ptr = price_queue.index.get(&3).copied().unwrap();

        let node_3 = unsafe { node_3_ptr.as_ref() };
        assert_eq!(node_3.ro.id, 3, "Order ID incorrect");

        // Check node 1 links are correct
        assert_eq!(node_1.next, Some(node_2_ptr));
        assert!(node_1.prev.is_none());

        // Check node 2 links are correct
        assert_eq!(node_2.next, Some(node_3_ptr), "Order 2 next incorrect");
        assert_eq!(node_2.prev, Some(node_1_ptr), "Order 2 prev incorrect");

        // Check node 3 links are correct
        assert_eq!(node_3.next, None, "Order 3 next incorrect");
        assert_eq!(node_3.prev, Some(node_2_ptr), "Order 3 prev incorrect");

        assert_eq!(
            price_queue.head,
            Some(price_queue.index.get(&1).copied().expect("Head incorrect"))
        );
        assert_eq!(
            price_queue.tail,
            Some(price_queue.index.get(&3).copied().expect("Tail incorrect"))
        );
    }

    #[test]
    fn test_freelist_reuse() {
        let mut price_queue =
            PriceLevelQueue::new(100, Side::BUY, MockClock { fixed: 123_456_789 });

        let order_1 = create_new_order_request(1, 100);
        let order_2 = create_new_order_request(2, 100);
        let order_3 = create_new_order_request(3, 100);

        price_queue.insert(order_1);
        price_queue.insert(order_2);
        price_queue.insert(order_3);

        price_queue.remove(1);

        let order_4 = create_new_order_request(4, 100);
        price_queue.insert(order_4);

        // Check freelist node was used, avoiding a memory-leak
        assert_eq!(price_queue.arena.len(), 3);
        assert_eq!(
            price_queue.freelist.len(),
            0,
            "Freelist not empty, potential memory leak"
        );

        // Check reused node has correct order and pointers
        let node_3_ptr = price_queue
            .index
            .get(&3)
            .copied()
            .expect("order 3 not found");
        let node_4_ptr = price_queue
            .index
            .get(&4)
            .copied()
            .expect("order 4 not found");
        let node_4 = unsafe { node_4_ptr.as_ref() };
        assert_eq!(node_4.ro.id, 4, "Order incorrect");

        assert_eq!(node_4.prev, Some(node_3_ptr));
        assert!(node_4.next.is_none());
    }

    #[test]
    fn test_remove() {
        let mut book = PriceLevelQueue::new(100, Side::BUY, MockClock { fixed: 123_456_789 });

        let order_1 = create_new_order_request(1, 100);
        let order_2 = create_new_order_request(2, 100);
        let order_3 = create_new_order_request(3, 100);

        book.insert(order_1);
        book.insert(order_2);
        book.insert(order_3);

        book.remove(1);

        // Check arena only contains the non-cancelled orders
        assert_eq!(book.arena.len(), 2);
        assert_eq!(book.freelist.len(), 1);

        // Check index contains the non-cancelled orders
        assert_eq!(book.index.len(), 2);

        let node_2_ptr = book.index.get(&2).copied().expect("order 2 not found");
        let node_3_ptr = book.index.get(&3).copied().expect("order 3 not found");

        // Check correct order in list (order_2 → order_2)
        let node_2 = unsafe { node_2_ptr.as_ref() };
        let node_3 = unsafe { node_3_ptr.as_ref() };

        assert_eq!(node_2.ro.id, 2);
        assert_eq!(node_3.ro.id, 3);

        assert_eq!(node_2.next, Some(node_3_ptr));
        assert!(node_2.prev.is_none());

        assert!(node_3.next.is_none());
        assert_eq!(node_3.prev, Some(node_2_ptr));

        // Check head and tail are correctly updated
        assert_eq!(book.head, Some(node_2_ptr));
        assert_eq!(book.tail, Some(node_3_ptr));
    }

    #[test]
    fn test_pop_head() {
        let mut book = PriceLevelQueue::new(100, Side::BUY, MockClock { fixed: 123_456_789 });

        let order_1 = create_new_order_request(1, 100);
        let order_2 = create_new_order_request(2, 100);
        let order_3 = create_new_order_request(3, 100);

        assert_eq!(book.pop_head(), CommandResult::Rejected(EmptyPriceLevel));

        book.insert(order_1);
        book.insert(order_2);
        book.insert(order_3);

        book.pop_head();

        // Check arena and freelist sizes
        assert_eq!(book.arena.len(), 2);
        assert_eq!(book.freelist.len(), 1);

        let head_ptr = book.head.unwrap();
        let head_node = unsafe { head_ptr.as_ref() };
        assert_eq!(head_node.ro.id, 2);
        assert_eq!(head_node.next, Some(book.index.get(&3).copied().unwrap()));
        assert!(head_node.prev.is_none());

        let tail_ptr = book.tail.unwrap();
        let tail_node = unsafe { tail_ptr.as_ref() };
        assert_eq!(tail_node.ro.id, 3);
        assert!(tail_node.next.is_none());
        assert_eq!(tail_node.prev, Some(book.index.get(&2).copied().unwrap()));
    }

    #[test]
    fn test_update_order() {
        let mut book = PriceLevelQueue::new(100, Side::BUY, MockClock { fixed: 123_456_789 });

        let order_1 = create_new_order_request(1, 100);
        let order_2 = create_new_order_request(2, 100);
        let order_3 = create_new_order_request(3, 100);

        book.insert(order_1);
        book.insert(order_2);
        book.insert(order_3);

        let modified_order_1 = UpdateQtyRequest {
            order_id: 1,
            new_qty: 10,
        };

        book.update_quantity(modified_order_1);

        assert_eq!(book.arena.len(), 3);
        assert_eq!(book.freelist.len(), 0);

        let head_ptr = book.head.unwrap();
        let head_node = unsafe { head_ptr.as_ref() };
        assert_eq!(
            head_node.ro.id, 1,
            "Head should not change after after update_quantity"
        );

        let tail_ptr = book.tail.unwrap();
        let tail_node = unsafe { tail_ptr.as_ref() };
        assert_eq!(
            tail_node.ro.id, 3,
            "Tail should not change after after update_quantity"
        );

        let second_node_ptr = head_node.next.unwrap();
        let second_node = unsafe { second_node_ptr.as_ref() };
        assert_eq!(
            second_node.ro.id, 2,
            "FIFO should not change after after update_quantity"
        );
        assert_eq!(
            second_node.prev,
            Some(head_ptr),
            "Second node prev is incorrect after update_quantity"
        );
        assert_eq!(
            second_node.next,
            Some(tail_ptr),
            "Second node next is incorrect after update_quantity"
        );

        assert_eq!(
            tail_node.prev,
            Some(second_node_ptr),
            "Tail node prev is incorrect after update_quantity"
        );
        assert_eq!(
            tail_node.next, None,
            "Tail node next is incorrect after update_quantity"
        );
    }

    #[test]
    fn test_apply_partial_fill() {
        let mut book = PriceLevelQueue::new(100, Side::BUY, MockClock { fixed: 123_456_789 });

        assert_eq!(
            book.apply_partial_fill(PartialFillRequest { qty_to_fill: 100 })
                .result,
            Rejected(EmptyPriceLevel)
        );

        let order_1 = create_new_order_request(1, 100);

        book.insert(order_1);

        let result = book
            .apply_partial_fill(PartialFillRequest { qty_to_fill: 50 })
            .result;

        if let CommandResult::Filled(FillInfo::Partial(info)) = result {
            assert_eq!(info.order_id, 1);
            assert_eq!(info.filled_qty, 50);
            assert_eq!(info.remaining_qty, 50);
        } else {
            panic!("Expected a partial fill, got {:?}", result);
        }
    }

    fn create_new_order_request(order_id: u64, qty: u64) -> EnqueueOrder {
        EnqueueOrder { order_id, qty }
    }
}
