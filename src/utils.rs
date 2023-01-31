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
    let mut array: [MaybeUninit<T>; N] = unsafe { MaybeUninit::uninit().assume_init() };
    for elem in &mut array {
        elem.write(default);
    }
    unsafe { mem::transmute_copy(&array) }
}

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
        unsafe {
            slice::from_raw_parts(self as *const _ as *const T, HEADER_SIZE + N + TRAILER_SIZE)
        }
    }
}

impl<T, const HEADER_SIZE: usize, const N: usize, const TRAILER_SIZE: usize> DerefMut
    for ArrayWithHeaderAndTrailer<T, HEADER_SIZE, N, TRAILER_SIZE>
{
    fn deref_mut(&mut self) -> &mut Self::Target {
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
