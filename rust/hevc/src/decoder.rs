//! The decoder: parameter sets, the reference pictures a picture decodes
//! against (§8.3), and one picture per access unit.

use std::sync::{Arc, Mutex};

use crate::bits::BitReader;
use crate::ctu::{self, Maps, PictureCtx, RefPic, Scratch};
use crate::deblock::DeblockCtx;
use crate::error::{Error, Result};
use crate::nal::{self, NalHeader, Rbsp};
use crate::pic::{PicState, Picture};
use crate::ps::{parse_pps, parse_sps, Colour, Pps, Sps};
use crate::sao::SaoCtx;
use crate::shared::PlanePtr;
use crate::slice::parse_slice_header;
use crate::wavefront::{self, Progress};

/// A decoded picture, and how to show it.
pub struct Decoded {
    /// At the coded size, which the window crops.
    pub picture: Arc<Picture>,
    /// `x, y, width, height` of the part to show.
    pub window: [u32; 4],
    pub colour: Colour,
    /// An IDR picture: the stream can be joined here.
    pub keyframe: bool,
}

/// A picture the next ones may refer to.
struct Reference {
    pic: Arc<Picture>,
    poc: i32,
}

pub struct Decoder {
    threads: usize,
    sps: Vec<Option<Sps>>,
    pps: Vec<Option<Pps>>,
    /// The references, every one marked short-term.
    dpb: Vec<Reference>,
    /// An IDR has been decoded and nothing has gone wrong since.
    started: bool,
    /// POC of the previous picture with TemporalId 0 that is not a
    /// sub-layer non-reference picture (§8.3.1).
    prev_tid0_poc: i32,
    /// The per-block maps, reused across pictures.
    state: Option<PicState>,
    /// Where a picture is reconstructed and deblocked, reused.
    recon: Option<Picture>,
    /// Picture buffers no reference or output holds any more.
    pool: Vec<Picture>,
    progress: Vec<Progress>,
    wpp_ctx: Vec<Mutex<Option<crate::cabac::Contexts>>>,
    scratch: Vec<Mutex<Scratch>>,
    rbsp: Rbsp,
}

impl Decoder {
    /// A decoder that decodes a picture's rows on `threads` threads: rayon's
    /// pool, which the caller has made, when more than one.
    pub fn new(threads: usize) -> Self {
        let threads = threads.max(1);
        Decoder {
            threads,
            sps: (0..16).map(|_| None).collect(),
            pps: (0..64).map(|_| None).collect(),
            dpb: Vec::new(),
            started: false,
            prev_tid0_poc: 0,
            state: None,
            recon: None,
            pool: Vec::new(),
            progress: Vec::new(),
            wpp_ctx: Vec::new(),
            scratch: (0..threads).map(|_| Mutex::new(Scratch::default())).collect(),
            rbsp: Rbsp::default(),
        }
    }

    /// Decodes one access unit in Annex B form: its parameter sets, and its
    /// picture if it has one. A unit that fails leaves the stream waiting for
    /// an IDR.
    pub fn decode(&mut self, unit: &[u8]) -> Result<Option<Decoded>> {
        let mut out = None;
        for nal in nal::split_annex_b(unit) {
            let hdr = NalHeader::parse(nal).ok_or_else(|| Error::invalid("a NAL unit header"))?;
            if hdr.layer_id != 0 {
                continue;
            }
            match hdr.nal_type {
                nal::SPS => {
                    self.rbsp.unescape(&nal[2..]);
                    let s = parse_sps(&self.rbsp.data)?;
                    let id = s.id as usize;
                    self.sps[id] = Some(s);
                }
                nal::PPS => {
                    self.rbsp.unescape(&nal[2..]);
                    let p = parse_pps(&self.rbsp.data)?;
                    let id = p.id as usize;
                    self.pps[id] = Some(p);
                }
                nal::TRAIL_N | nal::TRAIL_R | nal::IDR_W_RADL | nal::IDR_N_LP => {
                    if out.is_some() {
                        return Err(Error::invalid("two pictures in one unit"));
                    }
                    let mut rbsp = std::mem::take(&mut self.rbsp);
                    rbsp.unescape(&nal[2..]);
                    let r = self.picture(&hdr, &rbsp);
                    self.rbsp = rbsp;
                    match r {
                        Ok(decoded) => out = decoded,
                        Err(e) => {
                            self.started = false;
                            return Err(e);
                        }
                    }
                }
                t if t < 32 => return Err(Error::unsupported(format!("a picture of NAL unit type {t}"))),
                _ => {}
            }
        }
        Ok(out)
    }

    fn picture(&mut self, hdr: &NalHeader, rbsp: &Rbsp) -> Result<Option<Decoded>> {
        let idr = matches!(hdr.nal_type, nal::IDR_W_RADL | nal::IDR_N_LP);
        if !idr && !self.started {
            // Joining mid-stream: nothing to decode against until an IDR.
            return Ok(None);
        }
        let mut r = BitReader::new(&rbsp.data);
        let lookup = |id: u8| -> Option<(&Sps, &Pps)> {
            let pps = self.pps.get(id as usize)?.as_ref()?;
            let sps = self.sps.get(pps.sps_id as usize)?.as_ref()?;
            Some((sps, pps))
        };
        let sh = parse_slice_header(&mut r, hdr, &lookup)?;
        let (sps, pps) = lookup(sh.pps_id).ok_or_else(|| Error::invalid("the slice's parameter sets"))?;
        let (sps, pps) = (sps.clone(), pps.clone());
        if sh.entry_point_offsets.len() + 1 != sps.pic_height_in_ctbs as usize {
            return Err(Error::unsupported("a slice whose entry points are not one per coding tree block row"));
        }

        // POC (§8.3.1).
        let max_poc_lsb = 1i32 << sps.log2_max_poc_lsb;
        let poc = if idr {
            0
        } else {
            let prev_lsb = self.prev_tid0_poc & (max_poc_lsb - 1);
            let prev_msb = self.prev_tid0_poc - prev_lsb;
            let lsb = sh.poc_lsb as i32;
            let msb = if lsb < prev_lsb && prev_lsb - lsb >= max_poc_lsb / 2 {
                prev_msb + max_poc_lsb
            } else if lsb > prev_lsb && lsb - prev_lsb > max_poc_lsb / 2 {
                prev_msb - max_poc_lsb
            } else {
                prev_msb
            };
            msb + lsb
        };
        if hdr.temporal_id == 0 && hdr.nal_type != nal::TRAIL_N {
            self.prev_tid0_poc = poc;
        }

        // The reference picture set (§8.3.2): what stays, what is missing.
        if idr {
            self.release_all();
        }
        let wanted: Vec<(i32, bool)> = sh.st_rps.neg.iter().chain(sh.st_rps.pos.iter()).map(|&(d, used)| (poc + d, used)).collect();
        let mut i = 0;
        while i < self.dpb.len() {
            if wanted.iter().any(|&(p, _)| p == self.dpb[i].poc) {
                i += 1;
            } else {
                let gone = self.dpb.swap_remove(i);
                self.release(gone.pic);
            }
        }
        // A missing reference is a grey picture (§8.3.3.2), as FFmpeg makes one.
        for &(p, used) in &wanted {
            if used && !self.dpb.iter().any(|e| e.poc == p) {
                let mut pic = self.take_buffer(&sps);
                for plane in &mut pic.planes {
                    plane.data.fill(128);
                }
                pic.poc = p;
                self.dpb.push(Reference { pic: Arc::new(pic), poc: p });
            }
        }
        if self.dpb.len() > sps.max_dec_pic_buffering as usize {
            return Err(Error::invalid("more reference pictures than the stream allows"));
        }
        // RefPicList0 (§8.3.4): the earlier pictures, then the later, repeated.
        let mut refs: Vec<RefPic> = Vec::new();
        if !sh.intra {
            let order = sh.st_rps.neg.iter().chain(sh.st_rps.pos.iter()).filter(|&&(_, used)| used).map(|&(d, _)| poc + d);
            let pool: Vec<RefPic> = order.map(|p| self.dpb.iter().find(|e| e.poc == p).map(|e| RefPic { pic: e.pic.clone(), poc: e.poc }).expect("every used reference is in the DPB")).collect();
            for i in 0..sh.num_ref_idx_l0_active as usize {
                let e = &pool[i % pool.len()];
                refs.push(RefPic { pic: e.pic.clone(), poc: e.poc });
            }
        }

        // The picture's buffers and maps.
        let mut recon = match self.recon.take() {
            Some(p) if p.width() == sps.width as usize && p.height() == sps.height as usize => p,
            _ => Picture::new(sps.width as usize, sps.height as usize),
        };
        let mut state = match self.state.take() {
            Some(s) if s.fits(&sps) => s,
            _ => PicState::new(&sps),
        };
        state.reset();
        let rows = sps.pic_height_in_ctbs as usize;
        self.progress.resize_with(rows, Progress::default);
        self.wpp_ctx.resize_with(rows, || Mutex::new(None));
        for slot in &self.wpp_ctx {
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
        // Each row's substream, from the entry points, which count escaped bytes.
        let mut substreams = vec![sh.data_offset];
        let mut esc = rbsp.rbsp_to_escaped(sh.data_offset);
        for &off in &sh.entry_point_offsets {
            esc += off as usize;
            substreams.push(rbsp.escaped_to_rbsp(esc));
        }

        let maps = Maps::of(&mut state);
        let planes = [PlanePtr::of(&mut recon.planes[0]), PlanePtr::of(&mut recon.planes[1]), PlanePtr::of(&mut recon.planes[2])];
        let decoded = {
            let ctx = PictureCtx {
                sps: &sps,
                pps: &pps,
                sh: &sh,
                poc,
                planes,
                maps,
                zs: &state.zs,
                refs: &refs,
                data: &rbsp.data,
                substreams: &substreams,
                progress: &self.progress,
                wpp_ctx: &self.wpp_ctx,
            };
            let scratch = &self.scratch;
            wavefront::run_rows(self.threads, rows, &self.progress, |row| {
                // Each thread keeps one scratch; any free one will do.
                let mut s = scratch.iter().find_map(|m| m.try_lock().ok()).unwrap_or_else(|| scratch[0].lock().unwrap_or_else(|e| e.into_inner()));
                ctu::decode_row(&ctx, &mut s, row)
            })
        };
        if let Err(e) = decoded {
            self.recon = Some(recon);
            self.state = Some(state);
            return Err(e);
        }

        // The in-loop filters, into the output picture.
        if !sh.deblocking_filter_disabled {
            let ref_pocs: Vec<i32> = refs.iter().map(|r| r.poc).collect();
            let ctx = DeblockCtx { planes, maps, sh: &sh, pps: &pps, ref_pocs: &ref_pocs };
            wavefront::parallel_for(self.threads, rows, |row| ctx.vertical(row));
            wavefront::parallel_for(self.threads, rows, |row| ctx.horizontal(row));
        }
        let mut out = self.take_buffer(&sps);
        if sh.sao_luma || sh.sao_chroma {
            let dst = [PlanePtr::of(&mut out.planes[0]), PlanePtr::of(&mut out.planes[1]), PlanePtr::of(&mut out.planes[2])];
            let ctx = SaoCtx { src: [&recon.planes[0], &recon.planes[1], &recon.planes[2]], dst, maps };
            wavefront::parallel_for(self.threads, rows, |row| ctx.row(row));
        } else {
            std::mem::swap(&mut out, &mut recon);
        }
        out.poc = poc;
        self.recon = Some(recon);
        self.state = Some(state);
        self.started = true;

        let pic = Arc::new(out);
        self.dpb.push(Reference { pic: pic.clone(), poc });
        let (w, h) = sps.output_size();
        Ok(Some(Decoded { picture: pic, window: [sps.conf_win[0], sps.conf_win[2], w, h], colour: sps.colour, keyframe: idr }))
    }

    /// A picture buffer of the sequence's size: a pooled one, or a new one.
    fn take_buffer(&mut self, sps: &Sps) -> Picture {
        let (w, h) = (sps.width as usize, sps.height as usize);
        self.pool.retain(|p| p.width() == w && p.height() == h);
        self.pool.pop().unwrap_or_else(|| Picture::new(w, h))
    }

    /// Returns a picture's buffer to the pool once nothing else holds it.
    fn release(&mut self, pic: Arc<Picture>) {
        if let Ok(p) = Arc::try_unwrap(pic) {
            if self.pool.len() < 4 {
                self.pool.push(p);
            }
        }
    }

    fn release_all(&mut self) {
        for e in std::mem::take(&mut self.dpb) {
            self.release(e.pic);
        }
    }
}
