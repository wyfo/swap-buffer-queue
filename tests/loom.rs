// #![cfg(loom)]
//
// use std::{io::Write, iter};
//
// use futures::StreamExt;
// use loom::{future::block_on, sync::Arc, thread};
// use swap_buffer_queue::{buffer::VecBuffer, AsyncSBQueue};
// use tokio::sync::oneshot;
//
// // #[rustfmt::skip]
// // use std::{sync::Arc, thread};
// // use futures::executor::block_on;
//
// const THREADS: usize = 1;
// const ITERATIONS: usize = 2;
// const QUEUE_SIZE: usize = 1;
// const CONCURRENCY: usize = 1024;
//
// async fn task(queue: Arc<AsyncSBQueue<VecBuffer<oneshot::Sender<()>>>>) {
//     futures::stream::repeat_with(|| {
//         tokio::task::unconstrained(async {
//             let (tx, rx) = oneshot::channel::<()>();
//             queue.enqueue(tx).await.unwrap();
//             rx.await.unwrap_err();
//         })
//     })
//     .take(ITERATIONS)
//     .buffer_unordered(CONCURRENCY)
//     .for_each(|_| async {})
//     .await;
// }
//
// #[test]
// fn contention() {
//     let count = std::sync::atomic::AtomicUsize::new(0);
//     loom::model(move || {
//         // dbg!(count.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
//         if (count.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1) % 1000 == 0 {
//             print!(".");
//             std::io::stdout().flush().unwrap();
//         }
//         // println!("==================================================");
//         let queue = Arc::new(AsyncSBQueue::with_capacity(QUEUE_SIZE));
//         let threads: Vec<_> = iter::repeat_with(|| queue.clone())
//             .take(THREADS)
//             .map(|queue| thread::spawn(|| block_on(task(queue))))
//             .collect();
//         block_on(async {
//             let mut total = 0;
//             while total < THREADS * ITERATIONS {
//                 total += queue.dequeue().await.unwrap().len();
//             }
//         });
//         for thread in threads {
//             thread.join().unwrap();
//         }
//     })
// }
