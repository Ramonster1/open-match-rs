use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use open_match_rs::order_book::order::Side;
use open_match_rs::order_book::price_level::price_level::PriceLevelQueue;
use open_match_rs::order_book::price_level::requests::EnqueueOrder;
use open_match_rs::time::quanta_clock::QuantaClock;
use std::hint::black_box;

// test inserts
fn insert_orders_benchmark(c: &mut Criterion) {
    c.bench_function("insert 1000 orders", |b| {
        b.iter(|| {
            let mut price_queue =
                PriceLevelQueue::new(100, Side::BUY, QuantaClock::new());

            for i in 0..1000 {
                let order_req = EnqueueOrder {
                    order_id: black_box(i),
                    qty: black_box(100),
                };

                price_queue.insert(order_req);
            }
        });
    });
}

fn insert_single_order(c: &mut Criterion) {
    c.bench_function("insert single", |b| {
        b.iter_batched(
            // setup closure: returns the data you want to benchmark with
            || {
                let mut price_queue = PriceLevelQueue::new(100, Side::BUY, QuantaClock::new());
                let order_req = EnqueueOrder {
                    order_id: 1,
                    qty: 100,
                };
                (price_queue, order_req) // return a tuple
            },
            // benchmark closure: takes the tuple and runs the code to measure
            |(mut price_queue, order_req)| {
                price_queue.insert(order_req);
            },
            BatchSize::SmallInput,
        );
    });
}

// test cancels
fn cancel_1000_orders_benchmark(c: &mut Criterion) {
    c.bench_function("cancel 1000 orders", |b| {
        b.iter_batched(
            || {
                // Setup: insert 1000 orders
                let mut price_queue = PriceLevelQueue::new(100, Side::BUY, QuantaClock::new());
                for i in 0..1000 {
                    let order_req = EnqueueOrder {
                        order_id: 1,
                        qty: 100,
                    };

                    price_queue.insert(order_req);
                }
                price_queue
            },
            |mut price_queue| {
                // Measured: cancel all 1000 orders
                for id in 0..1000 {
                    price_queue.remove(black_box(id));
                }
            },
            BatchSize::SmallInput,
        );
    });
}

fn remove_order_benchmark(c: &mut Criterion) {
    c.bench_function("cancel 1 order from 1000", |b| {
        b.iter_batched(
            || {
                // Pre-fill the book with 1000 orders
                let mut price_queue = PriceLevelQueue::new(100, Side::BUY, QuantaClock::new());
                for i in 0..1000 {
                    let order_req = EnqueueOrder {
                        order_id: i,
                        qty: 100,
                    };

                    price_queue.insert(order_req);
                }
                // Cancel middle order to avoid head/tail fast paths
                (price_queue, 500_u64)
            },
            |(mut price_queue, id_to_cancel)| {
                // Only this cancel is measured
                price_queue.remove(id_to_cancel);
            },
            BatchSize::SmallInput,
        );
    });
}

fn create_new_order_request(order_id: u64, qty: u64) -> EnqueueOrder {
    EnqueueOrder { order_id, qty }
}

criterion_group!(benches, insert_orders_benchmark, insert_single_order, cancel_1000_orders_benchmark, remove_order_benchmark);
criterion_main!(benches);
