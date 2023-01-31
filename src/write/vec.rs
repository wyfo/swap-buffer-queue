use std::fmt;

use crate::{
    buffer::{Buffer, Resizable},
    write::{BytesSlice, WriteBytesSlice},
};

/// A bytes buffer with a `HEADER_SIZE`-bytes header and a `TRAILER_SIZE`-bytes trailer.
#[derive(Default)]
pub struct WriteVecBuffer<const HEADER_SIZE: usize = 0, const TRAILER_SIZE: usize = 0>(Box<[u8]>);

unsafe impl<T, const HEADER_SIZE: usize, const TRAILER_SIZE: usize> Buffer<T>
    for WriteVecBuffer<HEADER_SIZE, TRAILER_SIZE>
where
    T: WriteBytesSlice,
{
    type Slice<'a> = BytesSlice<'a, HEADER_SIZE, TRAILER_SIZE>;

    fn value_size(value: &T) -> usize {
        value.size()
    }

    fn capacity(&self) -> usize {
        self.0.len().saturating_sub(HEADER_SIZE + TRAILER_SIZE)
    }

    fn debug(&self, debug_struct: &mut fmt::DebugStruct) {
        let capacity = <WriteVecBuffer<HEADER_SIZE, TRAILER_SIZE> as Buffer<T>>::capacity(self);
        debug_struct.field("capacity", &capacity);
    }

    unsafe fn insert(&mut self, index: usize, mut value: T) {
        value.write(&mut self.0[HEADER_SIZE + index..HEADER_SIZE + index + value.size()])
    }

    unsafe fn slice(&mut self, len: usize) -> Self::Slice<'_> {
        BytesSlice::new(&mut self.0[..HEADER_SIZE + len + TRAILER_SIZE])
    }

    unsafe fn clear(&mut self, _len: usize) {}
}

unsafe impl<T, const HEADER_SIZE: usize, const TRAILER_SIZE: usize> Resizable<T>
    for WriteVecBuffer<HEADER_SIZE, TRAILER_SIZE>
where
    T: WriteBytesSlice,
{
    unsafe fn resize(&mut self, capacity: usize) {
        self.0 = vec![0; HEADER_SIZE + capacity + TRAILER_SIZE].into();
    }
}
