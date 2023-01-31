# swap-buffer-queue

A buffering MPSC queue.

This library is intended to be a (better, I hope) alternative to traditional MPSC queues in the context of a buffering consumer, by moving the buffering part directly into the queue.

It is especially well suited for IO writing workflow, see [buffer implementations](#buffer-implementations).

The crate is *no_std* (some buffer implementations may require `std`).

# Disclaimer

This library is at early state of development (and it's also my first Rust library). Waiting the crate publication, its documentation is available at https://wyfo.github.io/swap-buffer-queue/. Any feedback is welcome.

# Example

```rust
use std::ops::Deref;
use swap_buffer_queue::{buffer::VecBuffer, SBQueue};

// Initialize the queue with a capacity
let queue: SBQueue<VecBuffer<usize>, usize> = SBQueue::with_capacity(42);
// Enqueue some values
queue.try_enqueue(0).unwrap();
queue.try_enqueue(1).unwrap();
// Dequeue a slice to the enqueued values
let slice = queue.try_dequeue().unwrap();
assert_eq!(slice.deref(), &[0, 1]);
// Enqueued values can also be retrieved
assert_eq!(slice.into_iter().collect::<Vec<_>>(), vec![0, 1]);
```


# Buffer implementations

In addition to simple [`ArrayBuffer`](https://docs.rs/swap-buffer-queue/latest/swap_buffer_queue/buffer/struct.ArrayBuffer.html) and [`VecBuffer`](https://docs.rs/swap-buffer-queue/latest/swap_buffer_queue/buffer/struct.VecBuffer.html), this crate provides useful write-oriented implementations.

## [`write`](https://docs.rs/swap-buffer-queue/latest/swap_buffer_queue/write/index.html)

[`WriteArrayBuffer`](https://docs.rs/swap-buffer-queue/latest/swap_buffer_queue/write/struct.WriteVecBuffer.html) and 
[`WriteVecBuffer`](https://docs.rs/swap-buffer-queue/latest/swap_buffer_queue/write/struct.WriteVecBuffer.html) are well suited when there are objects to be serialized with a known-serialization size. Indeed, objects can then be serialized directly on the queue's buffer, avoiding allocation.

```rust
use swap_buffer_queue::SBQueue;
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
let queue: SBQueue<WriteVecBuffer<2>, Slice> = SBQueue::with_capacity((1 << 16) - 1);
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

## [`write_vectored`](https://docs.rs/swap-buffer-queue/latest/swap_buffer_queue/write/index.html)

[`WriteVectoredArrayBuffer`](https://docs.rs/swap-buffer-queue/latest/swap_buffer_queue/write_vectored/struct.WriteVectoredVecBuffer.html) and
[`WriteVectoredVecBuffer`](https://docs.rs/swap-buffer-queue/latest/swap_buffer_queue/write_vectored/struct.WriteVectoredVecBuffer.html) allows buffering a slice of [`IoSlice`](https://doc.rust-lang.org/std/io/struct.IoSlice.html), saving the cost of dequeuing io-slices one by one to collect them after.
(Internally, two buffers are used, one of the values, and one for the io-slices)

As a convenience, total size of the buffered io-slices can be retrieved.

```rust
use std::io::{IoSlice, Write};
use swap_buffer_queue::{write_vectored::WriteVectoredVecBuffer};
use swap_buffer_queue::SBQueue;

// Creates a WriteVectoredVecBuffer queue
let queue: SBQueue<WriteVectoredVecBuffer<Vec<u8>>, Vec<u8>> = SBQueue::with_capacity(100);
queue.try_enqueue(vec![0; 256]).unwrap();
queue.try_enqueue(vec![42; 42]).unwrap();
let mut slice = queue.try_dequeue().unwrap();
// Adds a header with the total size of the slices
let total_size = (slice.total_size() as u16).to_be_bytes();
let mut frame = slice.frame(.., Some(IoSlice::new(&total_size)), None);
// Let's pretend we have a writer
let mut writer: Vec<u8> = Default::default();
assert_eq!(writer.write_vectored(&mut frame).unwrap(), 300);
```

# Performance

*TODO*

# How it works 

Internally, this queue use 2 buffers: one being used for enqueuing while the other is dequeued. 

When [`SBQueue::try_enqueue`](https://docs.rs/swap-buffer-queue/latest/swap_buffer_queue/struct.SBQueue.html#method.try_enqueue) is called, it reserves atomically a slot in the current enqueuing buffer. The value is then inserted in the slot.

When [`SBQueue::try_dequeue`](https://docs.rs/swap-buffer-queue/latest/swap_buffer_queue/struct.SBQueue.html#method.try_dequeue) is called, both buffers are swapped atomically, so dequeued buffer will contain previously enqueued values, and new enqueued ones will go to the other (empty) buffer. 

As the two-phase enqueuing cannot be atomic, the queue can be in a transitory state, where slots have been reserved but have not been written yet.
This issue is mitigated using a spin loop in dequeuing method.
If the spin loop fails, the transitory state is saved and spin loop will be retried at the next dequeue.

Buffers can also be resized after being dequeued, just before the swap.