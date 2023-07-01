use std::{cell::Cell, mem::MaybeUninit, ops::Range};

use crate::{
    buffer::{Buffer, BufferValue, Drain},
    utils::init_array,
};

/// A simple array buffer.
pub struct ArrayBuffer<T, const N: usize>([Cell<MaybeUninit<T>>; N]);

impl<T, const N: usize> Default for ArrayBuffer<T, N> {
    fn default() -> Self {
        Self(init_array(|| Cell::new(MaybeUninit::uninit())))
    }
}

// SAFETY: `ArrayBuffer::clear` does clear the inserted range from the buffer
unsafe impl<T, const N: usize> Buffer for ArrayBuffer<T, N> {
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

// SAFETY: `T::insert_into` does initialize the index in the buffer
unsafe impl<T, const N: usize> BufferValue<ArrayBuffer<T, N>> for T {
    #[inline]
    fn size(&self) -> usize {
        1
    }

    #[inline]
    unsafe fn insert_into(self, buffer: &ArrayBuffer<T, N>, index: usize) {
        buffer.0[index].set(MaybeUninit::new(self));
    }
}

// SAFETY: `ArrayBuffer::remove` does remove the index from the buffer
unsafe impl<T, const N: usize> Drain for ArrayBuffer<T, N> {
    type Value = T;
    #[inline]
    unsafe fn remove(&mut self, index: usize) -> (Self::Value, usize) {
        // SAFETY: function contract guarantees that the index has been inserted and is then initialized
        let value = unsafe { self.0[index].replace(MaybeUninit::uninit()).assume_init() };
        (value, 1)
    }
}
