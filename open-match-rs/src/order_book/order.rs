use dashmap::DashMap;
use quanta::{Clock, IntoNanoseconds};
use std::collections::VecDeque;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Side {
    BUY = 0,
    SELL = 1,
}

#[repr(u8)]
enum OrderStatus {
    New = 0,
    PartiallyFilled = 1,
    Filled = 2,
    Cancelled = 3,
}

pub struct Order {
    pub id: u64,
    pub price: u64, // Price in smallest units (e.g., cents)
    pub quantity: u64,
    pub side: Side,
    pub timestamp_ns: u64,
}

impl Order {
    pub fn create(id: u64, price: u64, quantity: u64, side: Side, clock: &Clock) -> Self {
        Self {
            id,
            price,
            quantity,
            side,
            // Use the Clock at the exact point of Order creation for accuracy
            timestamp_ns: clock.raw().into_nanos(),
        }
    }
}

pub struct OrderBook {
    bids: Arc<DashMap<u64, VecDeque<Order>>>, // Buy orders grouped at price level (in the smallest units)
    asks: Arc<DashMap<u64, VecDeque<Order>>>, // Sell orders grouped at price level (in the smallest units)
    price_level_by_id: DashMap<u64, (u64, Side)>, // order_id → (price, side)
}

impl OrderBook {
    pub fn new() -> Self {
        Self {
            bids: Arc::new(DashMap::new()),
            asks: Arc::new(DashMap::new()),
            price_level_by_id: DashMap::new(),
        }
    }

    fn add_order(&mut self, order: Order) {
        match order.side {
            Side::BUY => self
                .bids
                .entry(order.price)
                .or_insert_with(VecDeque::new)
                .push_back(order),
            Side::SELL => self
                .asks
                .entry(order.price)
                .or_insert_with(VecDeque::new)
                .push_back(order),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_order() {
        let clock = Clock::new();

        let order = Order::create(1, 100, 1, Side::BUY, &clock);
        assert_eq!(
            (order.id, order.price, order.quantity, order.side),
            (1, 100, 1, Side::BUY)
        );
    }

    #[test]
    fn test_create_order_book() {
        let order_book = OrderBook::new();
        assert_eq!(order_book.bids.len(), 0);
        assert_eq!(order_book.asks.len(), 0);
    }

    #[test]
    fn test_add_order() {
        let clock = Clock::new();
        let order_id = 1;
        let order = Order::create(order_id, 100, 1, Side::BUY, &clock);

        let mut order_book = OrderBook::new();
        order_book.add_order(order);

        assert_eq!(order_book.bids.len(), 1);
        assert_eq!(order_book.asks.len(), 0);

        // assert_eq!(order_book.bids.get(&100).unwrap().pop_front().unwrap().id, order_id);
    }
}

/*todo
Placing an order in an exchange involves:

checking the target market is open to take orders
checking the order is valid for that market
choosing the right matching policy for the type of order
sequencing the order so that each order is matched at the best possible price and matched with the right liquidity
creating and publicizing the trades made as a consequence of the match
updating prices based on the new trades
 */
