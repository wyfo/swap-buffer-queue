use std::{
    cell::{Cell, UnsafeCell},
    mem,
    ops::Range,
};

use crate::{
    buffer::{Buffer, BufferValue, Resize},
    write::{BytesSlice, WriteBytesSlice},
};

/// A bytes buffer with a `HEADER_SIZE`-bytes header and a `TRAILER_SIZE`-bytes trailer.
#[derive(Default)]
pub struct WriteVecBuffer<const HEADER_SIZE: usize = 0, const TRAILER_SIZE: usize = 0>(
    Box<[Cell<u8>]>,
);

unsafe impl<const HEADER_SIZE: usize, const TRAILER_SIZE: usize> Buffer
    for WriteVecBuffer<HEADER_SIZE, TRAILER_SIZE>
{
    type Slice<'a> = BytesSlice<'a, HEADER_SIZE, TRAILER_SIZE>;

    fn capacity(&self) -> usize {
        self.0.len().saturating_sub(HEADER_SIZE + TRAILER_SIZE)
    }

    unsafe fn slice(&mut self, range: Range<usize>) -> Self::Slice<'_> {
        BytesSlice::new(
            &mut *(&mut self.0[range.start..HEADER_SIZE + range.end + TRAILER_SIZE]
                as *mut [Cell<u8>] as *mut [u8]),
        )
    }

    unsafe fn clear(&mut self, _range: Range<usize>) {}
}

unsafe impl<T, const HEADER_SIZE: usize, const TRAILER_SIZE: usize>
    BufferValue<WriteVecBuffer<HEADER_SIZE, TRAILER_SIZE>> for T
where
    T: WriteBytesSlice,
{
    fn size(&self) -> usize {
        WriteBytesSlice::size(self)
    }

    unsafe fn insert_into(self, buffer: &WriteVecBuffer<HEADER_SIZE, TRAILER_SIZE>, index: usize) {
        let size = self.size();
        self.write(&mut *UnsafeCell::raw_get(
            &buffer.0[HEADER_SIZE + index..HEADER_SIZE + index + size] as *const [Cell<u8>]
                as *const UnsafeCell<[u8]>,
        ));
    }
}

impl<const HEADER_SIZE: usize, const TRAILER_SIZE: usize> Resize
    for WriteVecBuffer<HEADER_SIZE, TRAILER_SIZE>
{
    fn resize(&mut self, capacity: usize) {
        let full_capacity = HEADER_SIZE + capacity + TRAILER_SIZE;
        self.0 = unsafe { mem::transmute(vec![0u8; full_capacity].into_boxed_slice()) };
    }
}
