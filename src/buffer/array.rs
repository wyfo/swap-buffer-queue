use std::{cell::UnsafeCell, mem, mem::MaybeUninit, ops::Range};

use crate::{
    buffer::{Buffer, BufferValue, Drain},
    utils::init_array,
};

/// A simple array buffer.
pub struct ArrayBuffer<T, const N: usize>([UnsafeCell<MaybeUninit<T>>; N]);

impl<T, const N: usize> Default for ArrayBuffer<T, N> {
    fn default() -> Self {
        Self(init_array(|| UnsafeCell::new(MaybeUninit::uninit())))
    }
}

unsafe impl<T, const N: usize> Buffer for ArrayBuffer<T, N> {
    type Slice<'a> = &'a mut [T]
    where
        T: 'a;

    fn capacity(&self) -> usize {
        self.0.len()
    }

    unsafe fn slice(&mut self, range: Range<usize>) -> Self::Slice<'_> {
        mem::transmute(&mut self.0[range])
    }

    unsafe fn clear(&mut self, range: Range<usize>) {
        for value in &mut self.0[range] {
            value.get_mut().assume_init_drop();
        }
    }
}

unsafe impl<T, const N: usize> BufferValue<ArrayBuffer<T, N>> for T {
    fn size(&self) -> usize {
        1
    }

    unsafe fn insert_into(self, buffer: &ArrayBuffer<T, N>, index: usize) {
        (*buffer.0[index].get()).write(self);
    }
}

unsafe impl<T, const N: usize> Drain for ArrayBuffer<T, N> {
    type Value = T;
    unsafe fn remove(&mut self, index: usize) -> (Self::Value, usize) {
        ((*self.0[index].get()).assume_init_read(), 1)
    }
}
