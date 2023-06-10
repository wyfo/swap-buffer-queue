use std::{
    cell::{Cell, UnsafeCell},
    ops::Range,
};

use crate::{
    buffer::{Buffer, BufferValue},
    utils::ArrayWithHeaderAndTrailer,
    write::{BytesSlice, WriteBytesSlice},
};

/// A `N`-bytes buffer with a `HEADER_SIZE`-bytes header and a `TRAILER_SIZE`-bytes trailer.
///
/// The total size of the buffer is `N + HEADER_SIZE + TRAILER_SIZE`. This buffer is *no_std*.
#[derive(Default)]
pub struct WriteArrayBuffer<
    const N: usize,
    const HEADER_SIZE: usize = 0,
    const TRAILER_SIZE: usize = 0,
>(ArrayWithHeaderAndTrailer<Cell<u8>, HEADER_SIZE, N, TRAILER_SIZE>);

unsafe impl<const N: usize, const HEADER_SIZE: usize, const TRAILER_SIZE: usize> Buffer
    for WriteArrayBuffer<N, HEADER_SIZE, TRAILER_SIZE>
{
    type Slice<'a> = BytesSlice<'a, HEADER_SIZE, TRAILER_SIZE>;

    fn capacity(&self) -> usize {
        N
    }

    unsafe fn slice(&mut self, range: Range<usize>) -> Self::Slice<'_> {
        BytesSlice::new(
            &mut *(&mut self.0[range.start..HEADER_SIZE + range.end + TRAILER_SIZE]
                as *mut [Cell<u8>] as *mut [u8]),
        )
    }

    unsafe fn clear(&mut self, _range: Range<usize>) {}
}

unsafe impl<T, const N: usize, const HEADER_SIZE: usize, const TRAILER_SIZE: usize>
    BufferValue<WriteArrayBuffer<N, HEADER_SIZE, TRAILER_SIZE>> for T
where
    T: WriteBytesSlice,
{
    fn size(&self) -> usize {
        WriteBytesSlice::size(self)
    }

    unsafe fn insert_into(
        self,
        buffer: &WriteArrayBuffer<N, HEADER_SIZE, TRAILER_SIZE>,
        index: usize,
    ) {
        let size = self.size();
        self.write(&mut *UnsafeCell::raw_get(
            &buffer.0[HEADER_SIZE + index..HEADER_SIZE + index + size] as *const [Cell<u8>]
                as *const UnsafeCell<[u8]>,
        ));
    }
}
