use std::{fmt, io::IoSlice, mem, mem::MaybeUninit};

use crate::{
    buffer::{Buffer, BufferValue, Drainable, Resizable},
    loom::{AtomicUsize, Ordering},
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

unsafe impl<T> Buffer for WriteVectoredVecBuffer<T>
where
    T: AsRef<[u8]>,
{
    type Slice<'a> = VectoredSlice<'a>
    where
        T: 'a;

    fn capacity(&self) -> usize {
        self.owned.len()
    }

    fn debug(&self, debug_struct: &mut fmt::DebugStruct) {
        debug_struct.field("capacity", &self.capacity());
        debug_struct.field("total_size", &self.total_size);
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

unsafe impl<T> BufferValue<WriteVectoredVecBuffer<T>> for T
where
    T: AsRef<[u8]>,
{
    fn size(&self) -> usize {
        1
    }

    unsafe fn insert_into(self, buffer: &mut WriteVectoredVecBuffer<T>, index: usize) {
        let owned_bytes = buffer.owned[index].write(self);
        let slice = IoSlice::new(owned_bytes.as_ref());
        buffer.slices[index + 1] = unsafe { mem::transmute(slice) };
        buffer.total_size.fetch_add(slice.len(), Ordering::AcqRel);
    }
}

unsafe impl<T> Resizable for WriteVectoredVecBuffer<T>
where
    T: AsRef<[u8]>,
{
    unsafe fn resize(&mut self, capacity: usize) {
        self.owned = (0..capacity).map(|_| MaybeUninit::uninit()).collect();
        self.slices = vec![IoSlice::new(EMPTY_SLICE); capacity + 2].into();
    }
}

unsafe impl<T> Drainable for WriteVectoredVecBuffer<T>
where
    T: AsRef<[u8]>,
{
    type Item = T;
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
