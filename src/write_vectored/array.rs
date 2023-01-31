use std::{
    fmt,
    io::IoSlice,
    mem,
    mem::MaybeUninit,
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::{
    buffer::{Buffer, Drainable},
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

unsafe impl<T, const N: usize> Buffer<T> for WriteVectoredArrayBuffer<T, N>
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
        N
    }

    fn debug(&self, debug_struct: &mut fmt::DebugStruct) {
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
        for value in &mut self.owned[..len] {
            unsafe { value.assume_init_drop() }
        }
        self.total_size.store(0, Ordering::Release);
    }
}

unsafe impl<T, const N: usize> Drainable<T> for WriteVectoredArrayBuffer<T, N>
where
    T: AsRef<[u8]>,
{
    type Drain<'a> =
        std::iter::Map<std::slice::IterMut<'a, MaybeUninit<T>>, fn(&mut MaybeUninit<T>) -> T>
    where
        T: 'a;

    unsafe fn drain(&mut self, len: usize) -> Self::Drain<'_> {
        self.owned[..len]
            .iter_mut()
            .map(|value| unsafe { value.assume_init_read() })
    }
}
