//! A display the Mac sends in strips: four, each the display's whole width
//! and a quarter of its height rounded up to whole coding blocks, lying top to
//! bottom a strip's height apart, so the last runs past the display's last
//! row. The strips are pictures of one stream, in one decoding order, and
//! which strip a picture is comes with its unit, not in it; a frame carries
//! the strips that changed and no others.

use crate::decoder::Decoded;
use crate::error::{Error, Result};
use crate::pic::Plane;
use crate::ps::{Colour, MAX_LUMA_PS};

/// The strips of a display.
pub const STRIPS: usize = 4;

/// Every strip's bit.
const ALL: u8 = (1 << STRIPS) - 1;

/// The display, put together from its strips as they decode.
pub struct Display {
    pub planes: [Plane; 3],
    pub colour: Colour,
    /// A strip's rows.
    pitch: usize,
    /// The strips placed since the keyframe, a bit each.
    placed: u8,
    /// Every strip placed is the keyframe's own.
    keyframe: bool,
}

impl Display {
    /// Puts the decoded `part` in as strip `strip` of a display of `rows`
    /// rows, starting `display` over where it is laid out for another. A
    /// keyframe starts it over too: what the other strips hold is from before
    /// whatever the keyframe mends, and is not shown beside it.
    pub fn place(display: &mut Option<Self>, strip: usize, rows: usize, part: &Decoded) -> Result<()> {
        let [x, y, width, pitch] = part.window.map(|v| v as usize);
        if strip >= STRIPS || pitch * STRIPS < rows || pitch * (STRIPS - 1) > rows || (width * rows) as u64 > MAX_LUMA_PS {
            return Err(Error::invalid(format!("a {width}\u{d7}{pitch} picture is not strip {strip} of a display of {rows} rows")));
        }
        let display = match display {
            Some(d) if d.planes[0].width == width && d.planes[0].height == rows && d.pitch == pitch => d,
            _ => display.insert(Display {
                planes: [Plane::new(width, rows), Plane::new(width, rows), Plane::new(width, rows)],
                colour: part.colour,
                pitch,
                placed: 0,
                keyframe: false,
            }),
        };
        if part.keyframe {
            display.placed = 0;
            display.keyframe = true;
        } else if display.placed & 1 << strip != 0 {
            display.keyframe = false;
        }
        display.colour = part.colour;
        let from = strip * pitch;
        for (into, plane) in display.planes.iter_mut().zip(&part.picture.planes) {
            for row in 0..pitch.min(rows - from) {
                let at = (from + row) * into.stride;
                let line = (y + row) * plane.stride + x;
                into.data[at..at + width].copy_from_slice(&plane.data[line..line + width]);
            }
        }
        display.placed |= 1 << strip;
        Ok(())
    }

    /// Whether every strip has come since the keyframe, so that the display
    /// has a picture to show.
    pub fn whole(&self) -> bool {
        self.placed == ALL
    }

    /// Whether the display is the keyframe's strips and no later one.
    pub fn keyframe(&self) -> bool {
        self.keyframe
    }
}
