use std::{fmt, mem, mem::MaybeUninit};

use crate::buffer::{Buffer, BufferValue, Drainable, Resizable};

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

    unsafe fn slice(&mut self, len: usize) -> Self::Slice<'_> {
        unsafe { mem::transmute(&mut self.0[..len]) }
    }

    unsafe fn clear(&mut self, len: usize) {
        for value in &mut self.0[..len] {
            unsafe { value.assume_init_drop() };
        }
    }
}

unsafe impl<T> BufferValue<VecBuffer<T>> for T {
    fn size(&self) -> usize {
        1
    }

    unsafe fn insert_into(self, buffer: &mut VecBuffer<T>, index: usize) {
        buffer.0[index].write(self);
    }
}

unsafe impl<T> Resizable for VecBuffer<T> {
    unsafe fn resize(&mut self, capacity: usize) {
        self.0 = (0..capacity).map(|_| MaybeUninit::uninit()).collect();
    }
}

unsafe impl<T> Drainable for VecBuffer<T> {
    type Item = T;
    type Drain<'a> =
        std::iter::Map<std::slice::IterMut<'a, MaybeUninit<T>>, fn(&mut MaybeUninit<T>) -> T>
    where
        T: 'a;

    unsafe fn drain(&mut self, len: usize) -> Self::Drain<'_> {
        self.0[..len]
            .iter_mut()
            .map(|value| unsafe { value.assume_init_read() })
    }
}
