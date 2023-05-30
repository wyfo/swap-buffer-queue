use std::{cell::UnsafeCell, fmt, mem, mem::MaybeUninit, ops::Range};

use crate::buffer::{Buffer, BufferValue, Drain, Resize};

/// A simple vector buffer.
pub struct VecBuffer<T>(Box<[MaybeUninit<T>]>);

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

    fn debug(&self, debug_struct: &mut fmt::DebugStruct) {
        debug_struct.field("capacity", &self.capacity());
    }

    unsafe fn slice(&mut self, range: Range<usize>) -> Self::Slice<'_> {
        unsafe { mem::transmute(&mut self.0[range]) }
    }

    unsafe fn clear(&mut self, range: Range<usize>) {
        for value in &mut self.0[range] {
            unsafe { value.assume_init_drop() };
        }
    }
}

unsafe impl<T> BufferValue<VecBuffer<T>> for T {
    fn size(&self) -> usize {
        1
    }

    unsafe fn insert_into(self, buffer: &UnsafeCell<VecBuffer<T>>, index: usize) {
        let buffer = &mut *buffer.get();
        buffer.0[index].write(self);
    }
}

impl<T> Resize for VecBuffer<T> {
    fn resize(&mut self, capacity: usize) {
        self.0 = (0..capacity).map(|_| MaybeUninit::uninit()).collect();
    }
}

unsafe impl<T> Drain for VecBuffer<T> {
    type Value = T;

    unsafe fn remove(&mut self, index: usize) -> (Self::Value, usize) {
        (self.0[index].assume_init_read(), 1)
    }
}
