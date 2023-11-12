//! [`Buffer`](`crate::buffer::Buffer`) implementations to be used with [`Write::write_vectored`](std::io::Write::write_vectored).
//!
//! [`WriteVectoredArrayBuffer`] and [`WriteVectoredVecBuffer`] allows buffering a slice of
//! [`IoSlice`], saving the cost of dequeuing io-slices one by one to collect them after.
//! (Internally, two buffers are used: one of the values, and one for the io-slices)
//!
//! # Examples
//! ```rust
//! # use std::io::{IoSlice, Write};
//! # use swap_buffer_queue::{Queue, write_vectored::WriteVectoredVecBuffer};
//! // Creates a WriteVectoredVecBuffer queue
//! let queue: Queue<WriteVectoredVecBuffer<Vec<u8>>> = Queue::with_capacity(100);
//! queue.try_enqueue([vec![0; 256]]).unwrap();
//! queue.try_enqueue([vec![42; 42]]).unwrap();
//! // Adds a header with the total size of the slices
//! let mut slice = queue.try_dequeue().unwrap();
//! let total_size = (slice.total_size() as u16).to_be_bytes();
//! let mut frame = slice.frame(.., Some(&total_size), None);
//! // Let's pretend we have a writer
//! let mut writer: Vec<u8> = Default::default();
//! assert_eq!(writer.write_vectored(&mut frame).unwrap(), 300);
//! // In this example, because `total_size` header has a shorter lifetime than `slice`,
//! // `slice` must be dropped before `total_size`.
//! drop(slice);
//! ```

use std::{
    fmt,
    io::IoSlice,
    mem,
    ops::{Bound, Deref, DerefMut, RangeBounds},
};

mod array;
mod vec;

pub use array::WriteVectoredArrayBuffer;
pub use vec::WriteVectoredVecBuffer;

pub(crate) static EMPTY_SLICE: &[u8] = &[];

/// A *vectored* slice, i.e. a slice of [`IoSlice`].
///
/// The total size of all the buffered io-slices can be retrieved with [`total_size`](VectoredSlice::total_size) method.
/// An header and a trailer can also be added to the slice using [`frame`](VectoredSlice::frame)
/// method.
///
/// # Examples
///
/// ```rust
/// # use std::io::IoSlice;
/// # use std::ops::Deref;
/// # use swap_buffer_queue::buffer::BufferSlice;
/// # use swap_buffer_queue::Queue;
/// # use swap_buffer_queue::write_vectored::{VectoredSlice, WriteVectoredVecBuffer};
/// # let queue: Queue<WriteVectoredVecBuffer<_>> = Queue::with_capacity(42);
/// # queue.try_enqueue([vec![2, 3, 4, 5]]).unwrap();
/// let header = vec![0, 1];
/// let trailer = vec![6, 7, 8, 9];
/// let mut slice: BufferSlice<WriteVectoredVecBuffer<Vec<u8>>, _> /* = ... */;
/// # slice = queue.try_dequeue().unwrap();
/// fn to_vec<'a, 'b: 'a>(slices: &'a [IoSlice<'b>]) -> Vec<&'a [u8]> {
///     slices.iter().map(Deref::deref).collect()
/// }
/// assert_eq!(to_vec(slice.deref().deref()), vec![&[2u8, 3, 4, 5]]);
/// assert_eq!(slice.total_size(), 4);
/// let frame = slice.frame(.., Some(&header), Some(&trailer));
/// assert_eq!(
///     to_vec(frame.deref()),
///     vec![&[0u8, 1] as &[u8], &[2, 3, 4, 5], &[6, 7, 8, 9]]
/// );
/// ```
pub struct VectoredSlice<'a> {
    slices: &'a mut [IoSlice<'static>],
    total_size: usize,
}

impl<'a> fmt::Debug for VectoredSlice<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VectoredSlice")
            .field("slices", &self.deref())
            .field("total_size", &self.total_size)
            .finish()
    }
}

impl<'a> Deref for VectoredSlice<'a> {
    type Target = [IoSlice<'a>];
    fn deref(&self) -> &Self::Target {
        &self.slices[1..self.slices.len() - 1]
    }
}

impl<'a> DerefMut for VectoredSlice<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let slices_len = self.slices.len();
        // SAFETY: slices in `self.slices[1..self.slices.len() - 1]` are never read
        // with their static lifetime (see `VectoredSlice::new`), only with `'a`,
        //  so it's fine to mutate them with `'a` lifetime
        unsafe { mem::transmute(&mut self.slices[1..slices_len - 1]) }
    }
}

impl<'a> VectoredSlice<'a> {
    /// # Safety
    /// `slices` must not be read by the caller and have a lifetime greater than `'a`
    pub(crate) unsafe fn new(slices: &'a mut [IoSlice<'static>], total_size: usize) -> Self {
        Self { slices, total_size }
    }

    /// Returns the total size of all the buffered io-slices
    /// (see [examples](VectoredSlice#examples)).
    pub fn total_size(&self) -> usize {
        self.total_size
    }

    /// Returns the *framed* part of the vectored slice within the given range, with an optional
    /// header io-slice and an optional trailer io-slice
    /// (see [examples](VectoredSlice#examples)).
    pub fn frame(
        &mut self,
        range: impl RangeBounds<usize>,
        header: Option<&'a [u8]>,
        trailer: Option<&'a [u8]>,
    ) -> VectoredFrame<'a> {
        let mut start = match range.start_bound() {
            Bound::Included(&n) => n,
            Bound::Excluded(&n) => n + 1,
            Bound::Unbounded => 0,
        };
        let mut end = match range.end_bound() {
            Bound::Included(&n) => n + 2,
            Bound::Excluded(&n) => n + 1,
            Bound::Unbounded => self.slices.len(),
        };
        let header = if let Some(header) = header {
            // SAFETY: `self.slices[start..end]` will be transmuted right after to `[IoSlice<'a>]
            Some(mem::replace(&mut self.slices[start], unsafe {
                mem::transmute(IoSlice::new(header))
            }))
        } else {
            start += 1;
            None
        };
        let trailer = if let Some(trailer) = trailer {
            // SAFETY: `self.slices[start..end]` will be transmuted right after to `[IoSlice<'a>]
            Some(mem::replace(&mut self.slices[end - 1], unsafe {
                mem::transmute(IoSlice::new(trailer))
            }))
        } else {
            end -= 1;
            None
        };
        VectoredFrame {
            // SAFETY: `[self.slices[1..self.slices.len() - 1]` is safe to transmute to
            // `[IoSlice<'a>]` (see `VectoredSlice::new`), and `start == 0`
            // (respectively `end == self.slices.len()`) means that `self.slices[start]`
            // (respectively `self.slices[end]`) has `'a` lifetime because it's set from `header`
            // (respectively `trailer`) parameter above
            slices: unsafe { mem::transmute(&mut self.slices[start..end]) },
            header,
            trailer,
        }
    }
}

/// A *framed* part of a [`VectoredSlice`], with an [`IoSlice`] header and an [`IoSlice`] trailer
/// (see [`VectoredSlice::frame`]).
pub struct VectoredFrame<'a> {
    slices: &'a mut [IoSlice<'a>],
    header: Option<IoSlice<'static>>,
    trailer: Option<IoSlice<'static>>,
}

impl fmt::Debug for VectoredFrame<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("VectoredFrame").field(&self.slices).finish()
    }
}

impl<'a> Deref for VectoredFrame<'a> {
    type Target = [IoSlice<'a>];
    fn deref(&self) -> &Self::Target {
        self.slices
    }
}

impl<'a> DerefMut for VectoredFrame<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.slices
    }
}

impl<'a> Drop for VectoredFrame<'a> {
    fn drop(&mut self) {
        if let Some(header) = self.header {
            self.slices[0] = header;
        }
        if let Some(trailer) = self.trailer {
            self.slices[self.slices.len() - 1] = trailer;
        }
    }
}
