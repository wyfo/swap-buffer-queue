use std::{
    cell::{Cell, UnsafeCell},
    num::NonZeroUsize,
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

// SAFETY: Buffer values are `Copy` and already initialized
unsafe impl<const N: usize, const HEADER_SIZE: usize, const TRAILER_SIZE: usize> Buffer
    for WriteArrayBuffer<N, HEADER_SIZE, TRAILER_SIZE>
{
    type Slice<'a> = BytesSlice<'a, HEADER_SIZE, TRAILER_SIZE>;

    #[inline]
    fn capacity(&self) -> usize {
        N
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
unsafe impl<T, const N: usize, const HEADER_SIZE: usize, const TRAILER_SIZE: usize>
    BufferValue<WriteArrayBuffer<N, HEADER_SIZE, TRAILER_SIZE>> for T
where
    T: WriteBytesSlice,
{
    fn size(&self) -> NonZeroUsize {
        WriteBytesSlice::size(self)
    }

    unsafe fn insert_into(
        self,
        buffer: &WriteArrayBuffer<N, HEADER_SIZE, TRAILER_SIZE>,
        index: usize,
        size: NonZeroUsize,
    ) {
        let slice = &buffer.0[HEADER_SIZE + index..HEADER_SIZE + index + size.get()];
        // SAFETY: [Cell<u8>] has the same layout as UnsafeCell<[u8]>
        self.write(unsafe {
            &mut *UnsafeCell::raw_get(slice as *const _ as *const UnsafeCell<[u8]>)
        });
    }
}
