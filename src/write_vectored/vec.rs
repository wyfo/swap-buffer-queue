use std::{cell::Cell, io::IoSlice, mem, mem::MaybeUninit, ops::Range};

use crate::{
    buffer::{Buffer, CellBuffer, Drain, Resize},
    loom::sync::atomic::{AtomicUsize, Ordering},
    write_vectored::{VectoredSlice, EMPTY_SLICE},
};

/// A buffer of [`IoSlice`]
pub struct WriteVectoredVecBuffer<T> {
    owned: Box<[Cell<MaybeUninit<T>>]>,
    slices: Box<[Cell<IoSlice<'static>>]>,
    total_size: AtomicUsize,
}

impl<T> Default for WriteVectoredVecBuffer<T> {
    fn default() -> Self {
        Self {
            owned: Default::default(),
            slices: Default::default(),
            total_size: Default::default(),
        }
    }
}

// SAFETY: `WriteVectoredVecBuffer::clear` does clear the inserted range from the buffer
unsafe impl<T> Buffer for WriteVectoredVecBuffer<T>
where
    T: AsRef<[u8]>,
{
    type Slice<'a> = VectoredSlice<'a>
    where
        T: 'a;

    #[inline]
    fn capacity(&self) -> usize {
        self.owned.len()
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
unsafe impl<T> CellBuffer<T> for WriteVectoredVecBuffer<T>
where
    T: AsRef<[u8]>,
{
    unsafe fn insert(&self, index: usize, value: T) {
        // SAFETY: slice is never read with static lifetime, it will only be used as a reference
        // with the same lifetime than the slice owner
        let slice = unsafe { mem::transmute(IoSlice::new(value.as_ref())) };
        self.slices[index + 1].set(slice);
        self.owned[index].set(MaybeUninit::new(value));
        self.total_size.fetch_add(slice.len(), Ordering::AcqRel);
    }
}

impl<T> Resize for WriteVectoredVecBuffer<T>
where
    T: AsRef<[u8]>,
{
    fn resize(&mut self, capacity: usize) {
        self.owned = (0..capacity)
            .map(|_| Cell::new(MaybeUninit::uninit()))
            .collect();
        self.slices = (0..capacity + 2)
            .map(|_| Cell::new(IoSlice::new(EMPTY_SLICE)))
            .collect();
    }
}

// SAFETY: `WriteVectoredVecBuffer::remove` does remove the index from the buffer
unsafe impl<T> Drain for WriteVectoredVecBuffer<T>
where
    T: AsRef<[u8]>,
{
    type Value = T;

    #[inline]
    unsafe fn remove(&mut self, index: usize) -> (Self::Value, usize) {
        // SAFETY: function contract guarantees that the index has been inserted and is then initialized
        let value = unsafe {
            self.owned[index]
                .replace(MaybeUninit::uninit())
                .assume_init()
        };
        self.total_size
            .fetch_sub(value.as_ref().len(), Ordering::Release);
        (value, 1)
    }
}
