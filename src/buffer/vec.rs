use std::{cell::UnsafeCell, mem::MaybeUninit, ops::Range};

use crate::buffer::{Buffer, BufferValue, Drain, Resize};

/// A simple vector buffer.
pub struct VecBuffer<T>(Box<[UnsafeCell<MaybeUninit<T>>]>);

impl<T> Default for VecBuffer<T> {
    fn default() -> Self {
        Self(Default::default())
    }
}

unsafe impl<T> Buffer for VecBuffer<T> {
    type Slice<'a> = &'a mut [T]
    where
        T: 'a;

    fn capacity(&self) -> usize {
        self.0.len()
    }

    unsafe fn slice(&mut self, range: Range<usize>) -> Self::Slice<'_> {
        &mut *(&mut self.0[range] as *mut [UnsafeCell<MaybeUninit<T>>] as *mut [T])
    }

    unsafe fn clear(&mut self, range: Range<usize>) {
        for value in &mut self.0[range] {
            value.get_mut().assume_init_drop();
        }
    }
}

unsafe impl<T> BufferValue<VecBuffer<T>> for T {
    fn size(&self) -> usize {
        1
    }

    unsafe fn insert_into(self, buffer: &VecBuffer<T>, index: usize) {
        (*buffer.0[index].get()).write(self);
    }
}

impl<T> Resize for VecBuffer<T> {
    fn resize(&mut self, capacity: usize) {
        self.0 = (0..capacity)
            .map(|_| UnsafeCell::new(MaybeUninit::uninit()))
            .collect();
    }
}

unsafe impl<T> Drain for VecBuffer<T> {
    type Value = T;

    unsafe fn remove(&mut self, index: usize) -> (Self::Value, usize) {
        ((*self.0[index].get()).assume_init_read(), 1)
    }
}
