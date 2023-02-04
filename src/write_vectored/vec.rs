use std::{
    fmt,
    io::IoSlice,
    mem,
    mem::MaybeUninit,
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::{
    buffer::{Buffer, Drainable, Resizable},
    write_vectored::{VectoredSlice, EMPTY_SLICE},
};

/// A buffer of [`IoSlice`]
pub struct WriteVectoredVecBuffer<T> {
    owned: Box<[MaybeUninit<T>]>,
    slices: Box<[IoSlice<'static>]>,
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

unsafe impl<T> Buffer<T> for WriteVectoredVecBuffer<T>
where
    T: AsRef<[u8]>,
{
    type Slice<'a> = VectoredSlice<'a>
    where
        T: 'a;

    fn value_size(_value: &T) -> usize {
        1
    }

    fn capacity(&self) -> usize {
        self.owned.len()
    }

    fn debug(&self, debug_struct: &mut fmt::DebugStruct) {
        debug_struct.field("capacity", &self.capacity());
        debug_struct.field("total_size", &self.total_size);
    }

    unsafe fn insert(&mut self, index: usize, value: T) {
        let owned_bytes = self.owned[index].write(value);
        let slice = IoSlice::new(owned_bytes.as_ref());
        self.slices[index + 1] = unsafe { mem::transmute(slice) };
        self.total_size.fetch_add(slice.len(), Ordering::AcqRel);
    }

    unsafe fn slice(&mut self, len: usize) -> Self::Slice<'_> {
        VectoredSlice::new(
            unsafe { mem::transmute(&mut self.slices[..len + 2]) },
            self.total_size.load(Ordering::Acquire),
        )
    }

    unsafe fn clear(&mut self, len: usize) {
        self.total_size.store(0, Ordering::Release);
        for value in &mut self.owned[..len] {
            unsafe { value.assume_init_drop() }
        }
    }
}

unsafe impl<T> Resizable<T> for WriteVectoredVecBuffer<T>
where
    T: AsRef<[u8]>,
{
    unsafe fn resize(&mut self, capacity: usize) {
        self.owned = (0..capacity).map(|_| MaybeUninit::uninit()).collect();
        self.slices = vec![IoSlice::new(EMPTY_SLICE); capacity + 2].into();
    }
}

unsafe impl<T> Drainable<T> for WriteVectoredVecBuffer<T>
where
    T: AsRef<[u8]>,
{
    type Drain<'a> =
        std::iter::Map<std::slice::IterMut<'a, MaybeUninit<T>>, fn(&mut MaybeUninit<T>) -> T>
    where
        T: 'a;

    unsafe fn drain(&mut self, len: usize) -> Self::Drain<'_> {
        self.total_size.store(0, Ordering::Release);
        self.owned[..len]
            .iter_mut()
            .map(|value| unsafe { value.assume_init_read() })
    }
}
