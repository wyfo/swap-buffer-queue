# swap-buffer-queue

[![License](https://img.shields.io/badge/license-MIT-blue.svg)](
https://github.com/wyfo/swap-buffer-queue/blob/main/LICENSE)
[![Cargo](https://img.shields.io/crates/v/swap-buffer-queue.svg)](
https://crates.io/crates/swap-buffer-queue)
[![Documentation](https://docs.rs/swap-buffer-queue/badge.svg)](
https://docs.rs/swap-buffer-queue)

A buffering MPSC queue.

This library is intended to be a (better, I hope) alternative to traditional MPSC queues in the context of a buffering consumer, by moving the buffering part directly into the queue.

It is especially well suited for IO writing workflow, see [buffer implementations](#buffer-implementations).

The crate is *no_std* (some buffer implementations may require `std`).


## Example

```rust
use std::ops::Deref;
use swap_buffer_queue::{buffer::VecBuffer, Queue};

// Initialize the queue with a capacity
let queue: Queue<VecBuffer<usize>, usize> = Queue::with_capacity(42);
// Enqueue some values
queue.try_enqueue(0).unwrap();
queue.try_enqueue(1).unwrap();
// Dequeue a slice to the enqueued values
let slice = queue.try_dequeue().unwrap();
assert_eq!(slice.deref(), &[0, 1]);
// Enqueued values can also be retrieved
assert_eq!(slice.into_iter().collect::<Vec<_>>(), vec![0, 1]);
```


## Buffer implementations

In addition to simple [`ArrayBuffer`](https://docs.rs/swap-buffer-queue/latest/swap_buffer_queue/buffer/struct.ArrayBuffer.html) and [`VecBuffer`](https://docs.rs/swap-buffer-queue/latest/swap_buffer_queue/buffer/struct.VecBuffer.html), this crate provides useful write-oriented implementations.

### [`write`](https://docs.rs/swap-buffer-queue/latest/swap_buffer_queue/write/index.html)

[`WriteArrayBuffer`](https://docs.rs/swap-buffer-queue/latest/swap_buffer_queue/write/struct.WriteVecBuffer.html) and 
[`WriteVecBuffer`](https://docs.rs/swap-buffer-queue/latest/swap_buffer_queue/write/struct.WriteVecBuffer.html) are well suited when there are objects to be serialized with a known-serialization size. Indeed, objects can then be serialized directly on the queue's buffer, avoiding allocation.

```rust
use swap_buffer_queue::Queue;
use swap_buffer_queue::write::{WriteBytesSlice, WriteVecBuffer};

// the slice to be written in the queue's buffer (not too complex for the example)
#[derive(Debug)]
struct Slice(Vec<u8>);
impl WriteBytesSlice for Slice {
    fn size(&self) -> usize {
        self.0.len()
    }
    fn write(&mut self, slice: &mut [u8]) {
        slice.copy_from_slice(&self.0);
    }
}
//!
// Creates a WriteVecBuffer queue with a 2-bytes header
let queue: Queue<WriteVecBuffer<2>, Slice> = Queue::with_capacity((1 << 16) - 1);
queue.try_enqueue(Slice(vec![0; 256])).unwrap();
queue.try_enqueue(Slice(vec![42; 42])).unwrap();
let mut slice = queue.try_dequeue().unwrap();
// Adds a header with the len of the buffer
let len = (slice.len() as u16).to_be_bytes();
slice.header().copy_from_slice(&len);
// Let's pretend we have a writer
let mut writer: Vec<u8> = Default::default();
assert_eq!(
    std::io::Write::write(&mut writer, slice.frame()).unwrap(),
    300
);
```

### [`write_vectored`](https://docs.rs/swap-buffer-queue/latest/swap_buffer_queue/write/index.html)

[`WriteVectoredArrayBuffer`](https://docs.rs/swap-buffer-queue/latest/swap_buffer_queue/write_vectored/struct.WriteVectoredVecBuffer.html) and
[`WriteVectoredVecBuffer`](https://docs.rs/swap-buffer-queue/latest/swap_buffer_queue/write_vectored/struct.WriteVectoredVecBuffer.html) allows buffering a slice of [`IoSlice`](https://doc.rust-lang.org/std/io/struct.IoSlice.html), saving the cost of dequeuing io-slices one by one to collect them after.
(Internally, two buffers are used, one of the values, and one for the io-slices)

As a convenience, total size of the buffered io-slices can be retrieved.

```rust
use std::io::{Write};
use swap_buffer_queue::{write_vectored::WriteVectoredVecBuffer};
use swap_buffer_queue::Queue;

// Creates a WriteVectoredVecBuffer queue
let queue: Queue<WriteVectoredVecBuffer<Vec<u8>>, Vec<u8>> = Queue::with_capacity(100);
queue.try_enqueue(vec![0; 256]).unwrap();
queue.try_enqueue(vec![42; 42]).unwrap();
let mut slice = queue.try_dequeue().unwrap();
// Adds a header with the total size of the slices
let total_size = (slice.total_size() as u16).to_be_bytes();
let mut frame = slice.frame(.., &total_size, None);
// Let's pretend we have a writer
let mut writer: Vec<u8> = Default::default();
assert_eq!(writer.write_vectored(&mut frame).unwrap(), 300);
```

## How it works 

Internally, this queue use 2 buffers: one being used for enqueuing while the other is dequeued. 

When [`Queue::try_enqueue`](https://docs.rs/swap-buffer-queue/latest/swap_buffer_queue/struct.Queue.html#method.try_enqueue) is called, it reserves atomically a slot in the current enqueuing buffer. The value is then inserted in the slot.

When [`Queue::try_dequeue`](https://docs.rs/swap-buffer-queue/latest/swap_buffer_queue/struct.Queue.html#method.try_dequeue) is called, both buffers are swapped atomically, so dequeued buffer will contain previously enqueued values, and new enqueued ones will go to the other (empty) buffer. 

As the two-phase enqueuing cannot be atomic, the queue can be in a transitory state, where slots have been reserved but have not been written yet. In this rare case, dequeuing will fail and have to be retried.

Also, `SynchronizedQueue` is a higher level interface that provides blocking and asynchronous methods.

## Unsafe

This library uses unsafe code, for three reasons:
- buffers are wrapped in `UnsafeCell` to allow mutable for the dequeued buffer;
- buffers implementation may use unsafe to allow insertion with shared reference;
- `Buffer` trait require unsafe interface for its invariant, because it's public.

To ensure the safety of the algorithm, it uses:
- tests (mostly doctests for now, but it needs to be completed)
- benchmarks
- MIRI (with tests)

Loom is partially integrated for now, but loom tests are on the TODO list.

## Performance

swap-buffer-queue is very performant – it's actually the fastest MPSC queue I know.

Here is the crossbeam benchmark [forked](https://github.com/wyfo/crossbeam/tree/bench_sbq/crossbeam-channel/benchmarks)

| benchmark     | crossbeam | swap-buffer-queue |
|---------------|-----------|-------------------|
| bounded1_mpsc | 1.545s    | 1.341s            |
| bounded1_spsc | 1.652s    | 1.138s            |
| bounded_mpsc  | 0.362s    | 0.178s            |
| bounded_seq   | 0.190s    | 0.156s            |
| bounded_spsc  | 0.115s    | 0.139s            |

However, a large enough capacity is required to reach maximum performance; otherwise, high contention scenario may be penalized.
This is because the algorithm put all the contention on a single atomic integer (instead of two for crossbeam).