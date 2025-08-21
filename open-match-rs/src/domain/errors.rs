#[derive(Debug, PartialEq, Eq)]
pub enum OrderBookError {
    OrderNotInArena,
    OrderNotFound,
    DuplicateOrderId,
    InsufficientQuantity,
    EmptyPriceLevel,
}