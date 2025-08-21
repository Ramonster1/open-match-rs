#[repr(C)]
#[derive(Debug)]
pub struct RestingOrder {
    pub id: u64,
    pub qty: u64,
    pub timestamp_ns: u64,
}

#[derive(Debug)]
pub struct OrderNode {
    pub ro: RestingOrder,
    pub prev: Option<std::ptr::NonNull<OrderNode>>,
    pub next: Option<std::ptr::NonNull<OrderNode>>,
}