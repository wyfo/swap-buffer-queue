// use std::{io::Write, sync::Arc, time::Duration};
//
// use futures::StreamExt;
// use swap_buffer_queue::{buffer::VecBuffer, AsyncSBQueue};
// use tokio::sync::oneshot;
//
// async fn task(queue: Arc<AsyncSBQueue<VecBuffer<oneshot::Sender<()>>>>) {
//     futures::stream::repeat_with(|| {
//         tokio::task::unconstrained(async {
//             let (tx, rx) = oneshot::channel::<()>();
//             queue.enqueue(tx).await.unwrap();
//             rx.await.unwrap_err();
//         })
//     })
//     .take(1000000)
//     .buffer_unordered(1024)
//     .for_each(|_| async {})
//     .await;
// }
//
// #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
// async fn test_contention() {
//     // console_subscriber::init();
//     let queue = Arc::new(AsyncSBQueue::with_capacity(100));
//     // for _ in 0..100000 {
//     //     let (tx, rx) = oneshot::channel::<()>();
//     //     drop(tx);
//     //     rx.await.unwrap_err();
//     // }
//     let tasks: Vec<_> = (0..4).map(|_| tokio::spawn(task(queue.clone()))).collect();
//     let queue2 = queue.clone();
//     let dequeue = tokio::spawn(async move {
//         // let mut total = 0;
//         loop {
//             // tokio::select! {
//             //     biased;
//             //     _ = tokio::time::sleep(Duration::from_secs(1)) => panic!("plop"),
//             //     slice = queue.dequeue() => total += slice.unwrap().len(),
//             // }
//             // total += queue.dequeue().await.unwrap().len();
//             // dbg!(total);
//             // if total > 10000 {
//             //     total -= 10000;
//             //     print!(".");
//             //     std::io::stdout().flush().unwrap();
//             // }
//             // tokio::task::yield_now().await;
//             // tokio::task::yield_now().await;
//             // tokio::task::yield_now().await;
//             // queue.dequeue().await.unwrap();
//             // dbg!(&queue);
//             dbg!(queue.dequeue().await.unwrap().len());
//         }
//     });
//     for task in tasks {
//         tokio::select! {
//             res = task => res.unwrap(),
//             _ = tokio::time::sleep(Duration::from_secs(20)) => {dbg!(&queue2);},
//         }
//         // task.await.unwrap();
//     }
//     dequeue.abort();
//     dequeue.await.unwrap_err();
// }
