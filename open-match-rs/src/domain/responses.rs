use crate::domain::errors::OrderBookError;

/// Response structs describe outcomes.

/// Captures the exact moment when a command’s effect (insert, cancel, fill, etc.) was executed on
/// the matching engine and records the outcome. This precisely timestamps all state-changing events.
/// Timestamp should be the precise point where the command is committed to the engine’s internal state.
pub struct TimedResult {
    pub timestamp_ns: u64,
    pub result: CommandResult,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CommandResult {
    Acknowledged,
    Filled(FillInfo),
    QuantityUpdated(QtyUpdatedInfo),
    Rejected(OrderBookError),
    Removed(RemoveInfo),
}

#[derive(Debug, PartialEq, Eq)]
pub enum FillInfo {
    Partial(PartialFillInfo),
    Full(FilledInfo),
}

#[derive(Debug, PartialEq, Eq)]
pub struct PartialFillInfo {
    pub order_id: u64,
    pub filled_qty: u64,
    pub remaining_qty: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct FilledInfo {
    pub order_id: u64,
    pub filled_qty: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct RemoveInfo {
    pub order_id: u64,
    pub canceled_quantity: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct QtyUpdatedInfo {
    pub order_id: u64,
    pub new_qty: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct QtyFilledInfo {
    pub order_id: u64,
    pub filled_qty: u64,
}