//! The slice segment header (§7.3.6.1) of the one slice a picture has.

use crate::bits::BitReader;
use crate::error::{Error, Result};
use crate::nal::{NalHeader, IDR_N_LP, IDR_W_RADL};
use crate::ps::{parse_st_ref_pic_set, Pps, ShortTermRps, Sps};

#[derive(Debug, Clone)]
pub struct SliceHeader {
    pub pps_id: u8,
    pub intra: bool,
    pub poc_lsb: u32,
    pub st_rps: ShortTermRps,
    pub sao_luma: bool,
    pub sao_chroma: bool,
    pub num_ref_idx_l0_active: u8,
    pub max_num_merge_cand: u8,
    /// `SliceQpY`.
    pub slice_qp: i32,
    pub cb_qp_offset: i32,
    pub cr_qp_offset: i32,
    pub deblocking_filter_disabled: bool,
    pub beta_offset_div2: i32,
    pub tc_offset_div2: i32,
    /// `entry_point_offset_minus1[i] + 1`, in escaped bytes: where each coding
    /// tree block row after the first starts.
    pub entry_point_offsets: Vec<u32>,
    /// Where `slice_segment_data()` starts in the RBSP.
    pub data_offset: usize,
}

fn ceil_log2(v: u32) -> u32 {
    if v <= 1 {
        0
    } else {
        32 - (v - 1).leading_zeros()
    }
}

/// Reads the header, leaving `r` at the slice data. `lookup` finds the PPS and
/// its SPS by PPS id.
pub fn parse_slice_header<'a>(r: &mut BitReader, nal: &NalHeader, lookup: &dyn Fn(u8) -> Option<(&'a Sps, &'a Pps)>) -> Result<SliceHeader> {
    let idr = matches!(nal.nal_type, IDR_W_RADL | IDR_N_LP);
    if !r.read_flag()? {
        return Err(Error::unsupported("more than one slice per picture"));
    }
    if idr {
        r.read_flag()?; // no_output_of_prior_pics_flag
    }
    let pps_id = r.read_ue_max(63)? as u8;
    let (sps, pps) = lookup(pps_id).ok_or_else(|| Error::invalid(format!("slice refers to unknown PPS {pps_id}")))?;
    for _ in 0..pps.num_extra_slice_header_bits {
        r.read_flag()?;
    }
    let intra = match r.read_ue_max(2)? {
        0 => return Err(Error::unsupported("B slices")),
        1 => false,
        _ => true,
    };
    let mut sh = SliceHeader {
        pps_id,
        intra,
        poc_lsb: 0,
        st_rps: ShortTermRps::default(),
        sao_luma: false,
        sao_chroma: false,
        num_ref_idx_l0_active: 0,
        max_num_merge_cand: 5,
        slice_qp: pps.init_qp,
        cb_qp_offset: 0,
        cr_qp_offset: 0,
        deblocking_filter_disabled: pps.deblocking_filter_disabled,
        beta_offset_div2: pps.beta_offset_div2,
        tc_offset_div2: pps.tc_offset_div2,
        entry_point_offsets: Vec::new(),
        data_offset: 0,
    };
    if !idr {
        sh.poc_lsb = r.read_bits(sps.log2_max_poc_lsb as u32)?;
        if !r.read_flag()? {
            sh.st_rps = parse_st_ref_pic_set(r, sps.st_rps.len(), &sps.st_rps, true)?;
        } else {
            if sps.st_rps.is_empty() {
                return Err(Error::invalid("no reference picture sets in the SPS"));
            }
            let idx = if sps.st_rps.len() > 1 { r.read_bits(ceil_log2(sps.st_rps.len() as u32))? as usize } else { 0 };
            sh.st_rps = sps.st_rps.get(idx).ok_or_else(|| Error::invalid("short_term_ref_pic_set_idx"))?.clone();
        }
        if sps.temporal_mvp_enabled && r.read_flag()? {
            return Err(Error::unsupported("temporal motion vector prediction"));
        }
    }
    if sps.sao_enabled {
        sh.sao_luma = r.read_flag()?;
        sh.sao_chroma = r.read_flag()?;
    }
    if !intra {
        sh.num_ref_idx_l0_active = pps.num_ref_idx_l0_default_active;
        if r.read_flag()? {
            sh.num_ref_idx_l0_active = r.read_ue_max(14)? as u8 + 1;
        }
        let used = sh.st_rps.neg.iter().chain(sh.st_rps.pos.iter()).filter(|&&(_, u)| u).count();
        if used == 0 {
            return Err(Error::invalid("a P slice with no reference picture"));
        }
        sh.max_num_merge_cand = 5 - r.read_ue_max(4)? as u8;
    }
    sh.slice_qp = pps.init_qp + r.read_se()?;
    if !(0..=51).contains(&sh.slice_qp) {
        return Err(Error::invalid("SliceQpY"));
    }
    if pps.slice_chroma_qp_offsets_present {
        sh.cb_qp_offset = r.read_se()?;
        sh.cr_qp_offset = r.read_se()?;
        let within = |v: i32| (-12..=12).contains(&v);
        if !within(sh.cb_qp_offset) || !within(sh.cr_qp_offset) || !within(pps.cb_qp_offset + sh.cb_qp_offset) || !within(pps.cr_qp_offset + sh.cr_qp_offset) {
            return Err(Error::invalid("slice chroma QP offsets"));
        }
    }
    if pps.deblocking_filter_override_enabled && r.read_flag()? {
        sh.deblocking_filter_disabled = r.read_flag()?;
        if !sh.deblocking_filter_disabled {
            sh.beta_offset_div2 = r.read_se()?;
            sh.tc_offset_div2 = r.read_se()?;
            if !(-6..=6).contains(&sh.beta_offset_div2) || !(-6..=6).contains(&sh.tc_offset_div2) {
                return Err(Error::invalid("slice deblocking offsets"));
            }
        }
    }
    if pps.loop_filter_across_slices_enabled && (sh.sao_luma || sh.sao_chroma || !sh.deblocking_filter_disabled) {
        r.read_flag()?; // one slice: nothing to filter across
    }
    let num = r.read_ue_max(sps.pic_height_in_ctbs)?;
    if num > 0 {
        let len = r.read_ue_max(31)? + 1;
        for _ in 0..num {
            sh.entry_point_offsets.push(r.read_bits(len)?.wrapping_add(1));
        }
    }
    if pps.slice_segment_header_extension_present {
        for _ in 0..r.read_ue_max(256)? {
            r.read_bits(8)?;
        }
    }
    if !r.read_flag()? {
        return Err(Error::invalid("alignment_bit_equal_to_one"));
    }
    r.align_to_byte();
    sh.data_offset = r.byte_pos();
    Ok(sh)
}
