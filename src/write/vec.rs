use alloc::boxed::Box;
use core::ops::Range;

use crate::{
    buffer::{Buffer, InsertIntoBuffer, Resize},
    loom::{cell::Cell, LoomUnsafeCell},
    write::{BytesSlice, WriteBytesSlice},
};

/// A bytes buffer with a `HEADER_SIZE`-bytes header and a `TRAILER_SIZE`-bytes trailer.
#[derive(Default)]
pub struct WriteVecBuffer<const HEADER_SIZE: usize = 0, const TRAILER_SIZE: usize = 0>(
    Box<[Cell<u8>]>,
);

// SAFETY: Buffer values are `Copy` and already initialized
unsafe impl<const HEADER_SIZE: usize, const TRAILER_SIZE: usize> Buffer
    for WriteVecBuffer<HEADER_SIZE, TRAILER_SIZE>
{
    type Slice<'a> = BytesSlice<'a, HEADER_SIZE, TRAILER_SIZE>;

    #[inline]
    fn capacity(&self) -> usize {
        self.0.len().saturating_sub(HEADER_SIZE + TRAILER_SIZE)
    }

    #[inline]
    unsafe fn slice(&mut self, range: Range<usize>) -> Self::Slice<'_> {
        // SAFETY: [Cell<u8>] has the same layout as [u8]
        BytesSlice::new(unsafe {
            &mut *(&mut self.0[range.start..HEADER_SIZE + range.end + TRAILER_SIZE] as *mut _
                as *mut [u8])
        })
    }

    #[inline]
    unsafe fn clear(&mut self, _range: Range<usize>) {}
}

// SAFETY: Buffer values are `Copy` and already initialized
unsafe impl<T, const HEADER_SIZE: usize, const TRAILER_SIZE: usize>
    InsertIntoBuffer<WriteVecBuffer<HEADER_SIZE, TRAILER_SIZE>> for T
where
    T: WriteBytesSlice,
{
    #[inline]
    fn size(&self) -> usize {
        WriteBytesSlice::size(self)
    }

    #[inline]
    unsafe fn insert_into(self, buffer: &WriteVecBuffer<HEADER_SIZE, TRAILER_SIZE>, index: usize) {
        let slice =
            &buffer.0[HEADER_SIZE + index..HEADER_SIZE + index + WriteBytesSlice::size(&self)];
        // SAFETY: [Cell<u8>] has the same layout as UnsafeCell<[u8]>
        unsafe {
            (*(slice as *const _ as *const LoomUnsafeCell<[u8]>)).with_mut(|s| self.write(&mut *s));
        };
    }
}

impl<const HEADER_SIZE: usize, const TRAILER_SIZE: usize> Resize
    for WriteVecBuffer<HEADER_SIZE, TRAILER_SIZE>
{
    fn resize(&mut self, capacity: usize) {
        let full_capacity = HEADER_SIZE + capacity + TRAILER_SIZE;
        let buffer = alloc::vec![0u8; full_capacity].into_boxed_slice();
        // SAFETY: [Cell<u8>] has the same layout as [u8]
        self.0 = unsafe { Box::from_raw(Box::into_raw(buffer) as *mut _) };
    }
}
