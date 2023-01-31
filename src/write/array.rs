use std::fmt;

use crate::{
    buffer::Buffer,
    utils::ArrayWithHeaderAndTrailer,
    write::{BytesSlice, WriteBytesSlice},
};

/// A `N`-bytes buffer with a `HEADER_SIZE`-bytes header and a `TRAILER_SIZE`-bytes trailer.
///
/// The total size of the buffer is `N + HEADER_SIZE + TRAILER_SIZE`. This buffer is *no_std*.
pub struct WriteArrayBuffer<
    const N: usize,
    const HEADER_SIZE: usize = 0,
    const TRAILER_SIZE: usize = 0,
>(ArrayWithHeaderAndTrailer<u8, HEADER_SIZE, N, TRAILER_SIZE>);

impl<const N: usize, const HEADER_SIZE: usize, const TRAILER_SIZE: usize> Default
    for WriteArrayBuffer<N, HEADER_SIZE, TRAILER_SIZE>
{
    fn default() -> Self {
        Self(ArrayWithHeaderAndTrailer::new(0))
    }
}

unsafe impl<T, const N: usize, const HEADER_SIZE: usize, const TRAILER_SIZE: usize> Buffer<T>
    for WriteArrayBuffer<N, HEADER_SIZE, TRAILER_SIZE>
where
    T: WriteBytesSlice,
{
    type Slice<'a> = BytesSlice<'a, HEADER_SIZE, TRAILER_SIZE>;

    fn value_size(value: &T) -> usize {
        value.size()
    }

    fn capacity(&self) -> usize {
        N
    }

    fn debug(&self, _debug_struct: &mut fmt::DebugStruct) {}

    unsafe fn insert(&mut self, index: usize, mut value: T) {
        value.write(&mut self.0[HEADER_SIZE + index..HEADER_SIZE + index + value.size()])
    }

    unsafe fn slice(&mut self, len: usize) -> Self::Slice<'_> {
        BytesSlice::new(&mut self.0[..HEADER_SIZE + len + TRAILER_SIZE])
    }

    unsafe fn clear(&mut self, _len: usize) {}
}
