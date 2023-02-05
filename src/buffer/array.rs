use std::{fmt, mem, mem::MaybeUninit};

use crate::buffer::{Buffer, BufferValue, Drainable};

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

    unsafe fn slice(&mut self, len: usize) -> Self::Slice<'_> {
        unsafe { mem::transmute(&mut self.0[..len]) }
    }

    unsafe fn clear(&mut self, len: usize) {
        for value in &mut self.0[..len] {
            unsafe { value.assume_init_drop() };
        }
    }
}

unsafe impl<T, const N: usize> BufferValue<ArrayBuffer<T, N>> for T {
    fn size(&self) -> usize {
        1
    }

    unsafe fn insert_into(self, buffer: &mut ArrayBuffer<T, N>, index: usize) {
        buffer.0[index].write(self);
    }
}

unsafe impl<T, const N: usize> Drainable for ArrayBuffer<T, N> {
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
