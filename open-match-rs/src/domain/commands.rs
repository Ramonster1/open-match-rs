use crate::domain::responses::TimedResult;
use crate::order_book::price_level::price_level::PriceLevelQueue;
use crate::time::TimeSource;

/// Structs to act on the order book
/// API exposed via public Request structs, which can be easily optimised with SBE. This allows
/// internal state to be hidden (e.g. Order struct)
/// OrderCommand's can be stored and replayed to ensure that the order book is always in a
/// consistent state in the case of Disaster Recovery or using a replicated order book.
pub trait OrderCommand {
    fn apply<C: TimeSource>(self, price_level_queue: &mut PriceLevelQueue<C>) -> TimedResult;
}

#[derive(Debug)]
pub struct NewOrderRequest {
    pub order_id: u64,
    pub price: u64,
    pub qty: u64,
}

#[derive(Debug)]
pub struct CancelRequest {
    pub order_id: u64,
}

#[derive(Debug)]
pub struct UpdateQtyRequest {
    pub order_id: u64,
    pub new_qty: u64,
}

#[derive(Debug)]
pub struct PartialFillRequest {
    pub qty_to_fill: u64,
}


/*
Big Picture Design
External API (domain-facing)
User/client submits a NewOrderRequest { price, qty }
order_id is assigned inside the engine, not by the client.
Other requests (CancelRequest, ReplaceRequest, etc.) keep their order_id because they need to reference an existing order.
Internal Commands (engine-facing)
These are the same as the external ones, but fully populated, i.e. they always have order_id.
They’re what the OrderBook executes on its data structures (e.g. PriceLevelQueue).
OrderBook Layer
Owns the ID generator
Owns the map: order_id → (price_level, node_ptr)
Exposes a public API (place_order, cancel_order, etc.) that takes external commands and transforms them into internal commands.
 */