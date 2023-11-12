use std::{cell::Cell, mem::MaybeUninit, ops::Range};

use crate::buffer::{Buffer, CellBuffer, Drain, Resize};

/// A simple vector buffer.
pub struct VecBuffer<T>(Box<[Cell<MaybeUninit<T>>]>);

impl<T> Default for VecBuffer<T> {
    fn default() -> Self {
        Self(Default::default())
    }
}

// SAFETY: `VecBuffer::clear` does clear the inserted range from the buffer
unsafe impl<T> Buffer for VecBuffer<T> {
    type Slice<'a> = &'a mut [T]
    where
        T: 'a;

    #[inline]
    fn capacity(&self) -> usize {
        self.0.len()
    }

    #[inline]
    unsafe fn slice(&mut self, range: Range<usize>) -> Self::Slice<'_> {
        // SAFETY: [Cell<MaybeUninit<T>>] has the same layout as [T]
        // and function contract guarantees that the range is initialized
        unsafe { &mut *(&mut self.0[range] as *mut _ as *mut [T]) }
    }

    #[inline]
    unsafe fn clear(&mut self, range: Range<usize>) {
        for index in range {
            // SAFETY: function contract guarantees that the range is initialized
            unsafe { self.remove(index) };
        }
    }
}

// SAFETY: `insert` does initialize the index in the buffer
unsafe impl<T> CellBuffer<T> for VecBuffer<T> {
    unsafe fn insert(&self, index: usize, value: T) {
        self.0[index].set(MaybeUninit::new(value));
    }
}

impl<T> Resize for VecBuffer<T> {
    fn resize(&mut self, capacity: usize) {
        self.0 = (0..capacity)
            .map(|_| Cell::new(MaybeUninit::uninit()))
            .collect();
    }
}

// SAFETY: `VecBuffer::remove` does remove the index from the buffer
unsafe impl<T> Drain for VecBuffer<T> {
    type Value = T;
    #[inline]
    unsafe fn remove(&mut self, index: usize) -> (Self::Value, usize) {
        // SAFETY: function contract guarantees that the index has been inserted and is then initialized
        let value = unsafe { self.0[index].replace(MaybeUninit::uninit()).assume_init() };
        (value, 1)
    }
}
