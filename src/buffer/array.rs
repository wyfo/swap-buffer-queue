use std::{cell::UnsafeCell, fmt, mem, mem::MaybeUninit, ops::Range};

use crate::buffer::{Buffer, BufferValue, Drain};

/// A simple array buffer.
pub struct ArrayBuffer<T, const N: usize>([MaybeUninit<T>; N]);

impl<T, const N: usize> Default for ArrayBuffer<T, N> {
    fn default() -> Self {
        Self(unsafe { MaybeUninit::uninit().assume_init() })
    }
}

unsafe impl<T, const N: usize> Buffer for ArrayBuffer<T, N> {
    type Slice<'a> = &'a mut [T]
    where
        T: 'a;

    fn capacity(&self) -> usize {
        self.0.len()
    }

    fn debug(&self, _debug_struct: &mut fmt::DebugStruct) {}

    unsafe fn slice(&mut self, range: Range<usize>) -> Self::Slice<'_> {
        unsafe { mem::transmute(&mut self.0[range]) }
    }

    unsafe fn clear(&mut self, range: Range<usize>) {
        for value in &mut self.0[range] {
            unsafe { value.assume_init_drop() };
        }
    }
}

unsafe impl<T, const N: usize> BufferValue<ArrayBuffer<T, N>> for T {
    fn size(&self) -> usize {
        1
    }

    unsafe fn insert_into(self, buffer: &UnsafeCell<ArrayBuffer<T, N>>, index: usize) {
        let buffer = &mut *buffer.get();
        buffer.0[index].write(self);
    }
}

unsafe impl<T, const N: usize> Drain for ArrayBuffer<T, N> {
    type Value = T;
    unsafe fn remove(&mut self, index: usize) -> (Self::Value, usize) {
        (self.0[index].assume_init_read(), 1)
    }
}
