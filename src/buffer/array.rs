use std::{fmt, mem, mem::MaybeUninit};

use crate::buffer::{Buffer, Drainable};

/// A simple array buffer.
pub struct ArrayBuffer<T, const N: usize>([MaybeUninit<T>; N]);

impl<T, const N: usize> Default for ArrayBuffer<T, N> {
    fn default() -> Self {
        Self(unsafe { MaybeUninit::uninit().assume_init() })
    }
}

unsafe impl<T, const N: usize> Buffer<T> for ArrayBuffer<T, N> {
    type Slice<'a> = &'a mut [T]
    where
        T: 'a;

    fn value_size(_value: &T) -> usize {
        1
    }

    fn capacity(&self) -> usize {
        self.0.len()
    }

    fn debug(&self, _debug_struct: &mut fmt::DebugStruct) {}

    unsafe fn insert(&mut self, index: usize, value: T) {
        self.0[index].write(value);
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

unsafe impl<T, const N: usize> Drainable<T> for ArrayBuffer<T, N> {
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
