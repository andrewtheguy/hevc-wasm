//! The picture and its per-block maps as the wavefront's rows share them.
//!
//! A picture's coding tree block rows decode side by side, each on its own
//! thread, into one set of sample planes and one set of per-4×4 maps. A row
//! writes only its own coding tree block row, and reads the row above only
//! where that row has finished (`wavefront` orders every such read after the
//! write with the row's progress, which is sequentially consistent). The
//! in-loop filters run on the same threads a block or two behind the
//! decoding (`ctu::decode_row`): a row's thread deblocks the row above, which
//! touches a few lines of the row above that, and applies SAO to the row
//! above that, from blocks the rows above are at least two blocks past, so
//! no two threads ever touch the same byte at once, and no reference made
//! here outlives the access it serves. That is the contract every `unsafe`
//! call below relies on, and the one place in this crate that holds it.

use std::marker::PhantomData;

use crate::pic::Plane;

/// One sample plane, by raw pointer.
#[derive(Clone, Copy)]
pub struct PlanePtr {
    ptr: *mut u8,
    pub stride: usize,
    pub width: usize,
    pub height: usize,
}

unsafe impl Send for PlanePtr {}
unsafe impl Sync for PlanePtr {}

impl PlanePtr {
    pub fn of(plane: &mut Plane) -> Self {
        PlanePtr { ptr: plane.data.as_mut_ptr(), stride: plane.stride, width: plane.width, height: plane.height }
    }

    /// The sample at (`x`, `y`).
    ///
    /// # Safety
    /// Nothing may be writing it: see the module.
    #[inline]
    pub unsafe fn get(&self, x: usize, y: usize) -> u8 {
        debug_assert!(x < self.width && y < self.height);
        unsafe { *self.ptr.add(y * self.stride + x) }
    }

    /// `len` samples of row `y` from `x`.
    ///
    /// # Safety
    /// Nothing may be writing them: see the module.
    #[inline]
    pub unsafe fn row(&self, x: usize, y: usize, len: usize) -> &[u8] {
        debug_assert!(x + len <= self.width && y < self.height);
        unsafe { std::slice::from_raw_parts(self.ptr.add(y * self.stride + x), len) }
    }

    /// The `w`×`h` block at (`x`, `y`) as the kernels take one: a slice from
    /// its first sample through its last, `stride` apart.
    ///
    /// # Safety
    /// Nothing else may be touching those rows' samples: see the module.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn block_mut(&self, x: usize, y: usize, w: usize, h: usize) -> &mut [u8] {
        debug_assert!(x + w <= self.width && y + h <= self.height && w > 0 && h > 0);
        unsafe { std::slice::from_raw_parts_mut(self.ptr.add(y * self.stride + x), (h - 1) * self.stride + w) }
    }
}

/// A per-block map, by raw pointer.
pub struct MapPtr<T> {
    ptr: *mut T,
    len: usize,
    _t: PhantomData<T>,
}

impl<T> Clone for MapPtr<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for MapPtr<T> {}
unsafe impl<T: Send> Send for MapPtr<T> {}
unsafe impl<T: Sync> Sync for MapPtr<T> {}

impl<T: Copy> MapPtr<T> {
    pub fn of(map: &mut [T]) -> Self {
        MapPtr { ptr: map.as_mut_ptr(), len: map.len(), _t: PhantomData }
    }

    /// # Safety
    /// Nothing may be writing entry `i`: see the module.
    #[inline]
    pub unsafe fn get(&self, i: usize) -> T {
        debug_assert!(i < self.len);
        unsafe { self.ptr.add(i).read() }
    }

    /// Entry `i`, in place.
    ///
    /// # Safety
    /// Nothing may be writing it: see the module.
    #[inline]
    pub unsafe fn at(&self, i: usize) -> &T {
        debug_assert!(i < self.len);
        unsafe { &*self.ptr.add(i) }
    }

    /// Entries `start..start + len`.
    ///
    /// # Safety
    /// Nothing may be writing them: see the module.
    #[inline]
    pub unsafe fn slice(&self, start: usize, len: usize) -> &[T] {
        debug_assert!(start + len <= self.len);
        unsafe { std::slice::from_raw_parts(self.ptr.add(start), len) }
    }

    /// # Safety
    /// Nothing else may be touching those entries: see the module.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn slice_mut(&self, start: usize, len: usize) -> &mut [T] {
        debug_assert!(start + len <= self.len);
        unsafe { std::slice::from_raw_parts_mut(self.ptr.add(start), len) }
    }

    /// # Safety
    /// Nothing else may be touching entry `i`: see the module.
    #[inline]
    pub unsafe fn set(&self, i: usize, v: T) {
        debug_assert!(i < self.len);
        unsafe { self.ptr.add(i).write(v) }
    }

    /// Sets a rectangle of a map `w4` entries wide, the rectangle given in
    /// luma samples.
    ///
    /// # Safety
    /// Nothing else may be touching those entries: see the module.
    #[inline]
    pub unsafe fn fill_rect(&self, w4: usize, x: usize, y: usize, w: usize, h: usize, v: T) {
        let (x0, x1) = (x >> 2, (x + w).div_ceil(4));
        let n = x1 - x0;
        for yy in y >> 2..(y + h).div_ceil(4) {
            let a = yy * w4 + x0;
            debug_assert!(a + n <= self.len);
            let p = unsafe { self.ptr.add(a) };
            if std::mem::size_of::<T>() == 1 {
                // SAFETY: `T` is one byte, so its bits are a `u8`'s.
                unsafe { fill_bytes(p as *mut u8, n, std::mem::transmute_copy(&v)) };
            } else {
                for i in 0..n {
                    unsafe { p.add(i).write(v) };
                }
            }
        }
    }
}

impl MapPtr<u8> {
    /// Entries `i..i + 8` as one little-endian word.
    ///
    /// # Safety
    /// Nothing may be writing them: see the module.
    #[inline]
    pub unsafe fn word(&self, i: usize) -> u64 {
        debug_assert!(i + 8 <= self.len);
        u64::from_le_bytes(unsafe { (self.ptr.add(i) as *const [u8; 8]).read_unaligned() })
    }
}

/// `n` copies of `b` at `p`, `n` a block's width in map entries (a power of
/// two up to 16), as one or two whole words: a `memory.fill` is a call into
/// the runtime, which costs more than the bytes it writes.
///
/// # Safety
/// `p..p + n` must be writable.
#[inline(always)]
unsafe fn fill_bytes(p: *mut u8, n: usize, b: u8) {
    let w = u64::from_ne_bytes([b; 8]);
    unsafe {
        match n {
            16 => {
                (p as *mut u64).write_unaligned(w);
                (p.add(8) as *mut u64).write_unaligned(w);
            }
            8 => (p as *mut u64).write_unaligned(w),
            4 => (p as *mut u32).write_unaligned(w as u32),
            2 => (p as *mut u16).write_unaligned(w as u16),
            1 => *p = b,
            _ => {
                for i in 0..n {
                    *p.add(i) = b;
                }
            }
        }
    }
}
