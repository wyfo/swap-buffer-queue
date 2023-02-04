use std::{
    mem,
    mem::MaybeUninit,
    ops::{Deref, DerefMut},
    slice,
};

fn init_array<T, const N: usize>(default: T) -> [T; N]
where
    T: Copy,
{
    // SAFETY: common MaybeUninit pattern, used in unstable `MaybeUninit::uninit_array`
    let mut array: [MaybeUninit<T>; N] = unsafe { MaybeUninit::uninit().assume_init() };
    for elem in &mut array {
        elem.write(default);
    }
    // SAFETY: all elements have been initialized
    // I used `std::mem::transmute_copy` because `transmute` doesn't work here
    // see https://users.rust-lang.org/t/transmuting-a-generic-array/45645
    unsafe { mem::transmute_copy(&array) }
}

/// A hack for const-expression-sized array, as discussed here:
/// https://users.rust-lang.org/t/is-slice-from-raw-parts-unsound-in-case-of-a-repr-c-struct-with-consecutive-arrays/88368
#[allow(dead_code)]
pub(crate) struct ArrayWithHeaderAndTrailer<
    T,
    const HEADER_SIZE: usize,
    const N: usize,
    const TRAILER_SIZE: usize,
> {
    header: [T; HEADER_SIZE],
    array: [T; N],
    trailer: [T; TRAILER_SIZE],
}

impl<T, const HEADER_SIZE: usize, const N: usize, const TRAILER_SIZE: usize> Deref
    for ArrayWithHeaderAndTrailer<T, HEADER_SIZE, N, TRAILER_SIZE>
{
    type Target = [T];
    fn deref(&self) -> &Self::Target {
        // SAFETY: see struct documentation
        unsafe {
            slice::from_raw_parts(self as *const _ as *const T, HEADER_SIZE + N + TRAILER_SIZE)
        }
    }
}

impl<T, const HEADER_SIZE: usize, const N: usize, const TRAILER_SIZE: usize> DerefMut
    for ArrayWithHeaderAndTrailer<T, HEADER_SIZE, N, TRAILER_SIZE>
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: see struct documentation
        unsafe {
            slice::from_raw_parts_mut(self as *mut _ as *mut T, HEADER_SIZE + N + TRAILER_SIZE)
        }
    }
}

impl<T, const HEADER_SIZE: usize, const N: usize, const TRAILER_SIZE: usize>
    ArrayWithHeaderAndTrailer<T, HEADER_SIZE, N, TRAILER_SIZE>
where
    T: Copy,
{
    pub(crate) fn new(default: T) -> Self {
        Self {
            header: init_array(default),
            array: init_array(default),
            trailer: init_array(default),
        }
    }
}
