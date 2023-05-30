use std::{cell::UnsafeCell, fmt, io::IoSlice, mem, mem::MaybeUninit, ops::Range};

use crate::{
    buffer::{Buffer, BufferValue, Drain},
    loom::{AtomicUsize, Ordering},
    utils::ArrayWithHeaderAndTrailer,
    write_vectored::{VectoredSlice, EMPTY_SLICE},
};

/// A buffer of [`IoSlice`] of size `N`
///
/// The total size of the buffer is `N * mem::size_of::<T>() + (N + 2) * mem::size_of::<IoSlice>()`.
pub struct WriteVectoredArrayBuffer<T, const N: usize> {
    owned: [MaybeUninit<T>; N],
    slices: ArrayWithHeaderAndTrailer<IoSlice<'static>, 1, N, 1>,
    total_size: AtomicUsize,
}

impl<T, const N: usize> Default for WriteVectoredArrayBuffer<T, N> {
    fn default() -> Self {
        Self {
            owned: unsafe { MaybeUninit::uninit().assume_init() },
            slices: ArrayWithHeaderAndTrailer::new(IoSlice::new(EMPTY_SLICE)),
            total_size: Default::default(),
        }
    }
}

unsafe impl<T, const N: usize> Buffer for WriteVectoredArrayBuffer<T, N>
where
    T: AsRef<[u8]>,
{
    type Slice<'a> = VectoredSlice<'a>
    where
        T: 'a;

    fn capacity(&self) -> usize {
        N
    }

    fn debug(&self, debug_struct: &mut fmt::DebugStruct) {
        debug_struct.field("total_size", &self.total_size);
    }

    unsafe fn slice(&mut self, range: Range<usize>) -> Self::Slice<'_> {
        VectoredSlice::new(
            unsafe { mem::transmute(&mut self.slices[range.start..range.end + 2]) },
            self.total_size.load(Ordering::Acquire),
        )
    }

    unsafe fn clear(&mut self, range: Range<usize>) {
        self.total_size.store(0, Ordering::Release);
        for value in &mut self.owned[range] {
            unsafe { value.assume_init_drop() }
        }
    }
}

unsafe impl<T, const N: usize> BufferValue<WriteVectoredArrayBuffer<T, N>> for T
where
    T: AsRef<[u8]>,
{
    fn size(&self) -> usize {
        1
    }

    unsafe fn insert_into(self, buffer: &UnsafeCell<WriteVectoredArrayBuffer<T, N>>, index: usize) {
        let buffer = &mut *buffer.get();
        let owned_bytes = buffer.owned[index].write(self);
        let slice = IoSlice::new(owned_bytes.as_ref());
        buffer.slices[index + 1] = unsafe { mem::transmute(slice) };
        buffer.total_size.fetch_add(slice.len(), Ordering::AcqRel);
    }
}

unsafe impl<T, const N: usize> Drain for WriteVectoredArrayBuffer<T, N>
where
    T: AsRef<[u8]>,
{
    type Value = T;

    unsafe fn remove(&mut self, index: usize) -> (Self::Value, usize) {
        let value = self.owned[index].assume_init_read();
        self.total_size
            .fetch_sub(self.owned.as_ref().len(), Ordering::Release);
        (value, 1)
    }
}
