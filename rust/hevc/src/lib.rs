//! A decoder for the HEVC a High Performance Mac sends: 4:4:4 at 8 bits, one
//! slice per picture with its coding tree block rows coded as a wavefront, I
//! and P pictures with short-term references. It refuses every other shape of
//! stream by name.
//!
//! Each access unit decodes to its picture, which comes back as three planes
//! of bytes at the coded size, with the window to show and the colour the
//! stream states. A picture's rows decode in parallel on rayon's pool when the
//! decoder is made with threads.

mod bits;
mod cabac;
mod ctu;
mod deblock;
mod decoder;
mod error;
mod intra;
mod itx;
mod kernels;
mod nal;
mod pic;
mod ps;
mod sao;
mod shared;
mod slice;
mod tables;
mod wavefront;

pub use decoder::{Decoded, Decoder};
pub use nal::access_units;
pub use error::{Error, Result};
pub use pic::{Picture, Plane};
pub use ps::Colour;
