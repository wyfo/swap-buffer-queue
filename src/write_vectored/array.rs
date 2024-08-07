use std::{io::IoSlice, mem, mem::MaybeUninit, ops::Range};

use crate::{
    buffer::{Buffer, CellBuffer, Drain},
    loom::{
        cell::Cell,
        sync::atomic::{AtomicUsize, Ordering},
    },
    utils::{init_array, ArrayWithHeaderAndTrailer},
    write_vectored::{VectoredSlice, EMPTY_SLICE},
};

/// A buffer of [`IoSlice`] of size `N`
///
/// The total size of the buffer is `N * mem::size_of::<T>() + (N + 2) * mem::size_of::<IoSlice>()`.
pub struct WriteVectoredArrayBuffer<T, const N: usize> {
    owned: [Cell<MaybeUninit<T>>; N],
    slices: ArrayWithHeaderAndTrailer<Cell<IoSlice<'static>>, 1, N, 1>,
    total_size: AtomicUsize,
}

impl<T, const N: usize> Default for WriteVectoredArrayBuffer<T, N> {
    fn default() -> Self {
        Self {
            owned: init_array(|| Cell::new(MaybeUninit::uninit())),
            slices: ArrayWithHeaderAndTrailer::new(|| Cell::new(IoSlice::new(EMPTY_SLICE))),
            total_size: Default::default(),
        }
    }
}

// SAFETY: `WriteVectoredArrayBuffer::clear` does clear the inserted range from the buffer
unsafe impl<T, const N: usize> Buffer for WriteVectoredArrayBuffer<T, N>
where
    T: AsRef<[u8]>,
{
    type Slice<'a> = VectoredSlice<'a>
    where
        T: 'a;

    #[inline]
    fn capacity(&self) -> usize {
        N
    }

    #[inline]
    unsafe fn slice(&mut self, range: Range<usize>) -> Self::Slice<'_> {
        // SAFETY: [Cell<IoSlice>] has the same layout as [IoSlice]
        // and function contract guarantees that the range is initialized
        let slices = unsafe {
            &mut *(&mut self.slices[range.start..range.end + 2] as *mut _
                as *mut [IoSlice<'static>])
        };
        // SAFETY: slices are never read and live along their owner in the buffer, as they are
        // inserted and removed together
        unsafe { VectoredSlice::new(slices, self.total_size.load(Ordering::Acquire)) }
    }

    #[inline]
    unsafe fn clear(&mut self, range: Range<usize>) {
        *self.total_size.get_mut() = 0;
        for index in range {
            // SAFETY: function contract guarantees that the range is initialized
            unsafe { self.remove(index) };
        }
    }
}

// SAFETY: `insert` does initialize the index in the buffer
unsafe impl<T, const N: usize> CellBuffer<T> for WriteVectoredArrayBuffer<T, N>
where
    T: AsRef<[u8]>,
{
    unsafe fn insert(&self, index: usize, value: T) {
        // SAFETY: slice is never read with static lifetime, it will only be used as a reference
        // with the same lifetime than the slice owner
        let slice = unsafe { mem::transmute::<IoSlice, IoSlice>(IoSlice::new(value.as_ref())) };
        self.slices[index + 1].set(slice);
        self.owned[index].set(MaybeUninit::new(value));
        self.total_size.fetch_add(slice.len(), Ordering::AcqRel);
    }
}

// SAFETY: `WriteVectoredArrayBuffer::remove` does remove the index from the buffer
unsafe impl<T, const N: usize> Drain for WriteVectoredArrayBuffer<T, N>
where
    T: AsRef<[u8]>,
{
    type Value = T;

    #[inline]
    unsafe fn remove(&mut self, index: usize) -> Self::Value {
        // SAFETY: function contract guarantees that the index has been inserted and is then initialized
        let value = unsafe {
            self.owned[index]
                .replace(MaybeUninit::uninit())
                .assume_init()
        };
        self.total_size
            .fetch_sub(value.as_ref().len(), Ordering::Release);
        value
    }
}
