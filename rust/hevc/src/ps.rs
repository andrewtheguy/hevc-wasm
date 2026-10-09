//! The sequence and picture parameter sets (§7.3.2.2, §7.3.2.3), read whole and
//! refused where they ask for a tool this decoder does not have.

use crate::bits::BitReader;
use crate::error::{Error, Result};

/// `MaxLumaPs` of level 6.2 (Table A.8), the largest of any level.
pub(crate) const MAX_LUMA_PS: u64 = 35_651_584;

fn unsupported(what: &str) -> Error {
    Error::unsupported(format!("the stream uses {what}, which is not the Mac's shape"))
}

/// `profile_tier_level()`, read past: the tools are checked one by one below,
/// which is what the profile would promise.
fn skip_ptl(r: &mut BitReader, max_sub_layers_minus1: u32) -> Result<()> {
    r.read_bits(8)?; // profile space, tier, profile idc
    r.read_bits(32)?; // compatibility flags
    r.read_bits(32)?; // source, constraint and reserved flags
    r.read_bits(16)?;
    r.read_bits(8)?; // level idc
    let n = max_sub_layers_minus1 as usize;
    let mut sub = [(false, false); 8];
    for s in sub.iter_mut().take(n) {
        *s = (r.read_flag()?, r.read_flag()?);
    }
    if n > 0 {
        for _ in n..8 {
            r.read_bits(2)?;
        }
    }
    for &(profile, level) in sub.iter().take(n) {
        if profile {
            r.read_bits(32)?;
            r.read_bits(32)?;
            r.read_bits(24)?;
        }
        if level {
            r.read_bits(8)?;
        }
    }
    Ok(())
}

fn skip_sub_layer_hrd(r: &mut BitReader, cpb_cnt: u32, sub_pic: bool) -> Result<()> {
    for _ in 0..cpb_cnt {
        r.read_ue()?;
        r.read_ue()?;
        if sub_pic {
            r.read_ue()?;
            r.read_ue()?;
        }
        r.read_flag()?;
    }
    Ok(())
}

fn skip_hrd(r: &mut BitReader, max_sub_layers_minus1: u32) -> Result<()> {
    let nal_hrd = r.read_flag()?;
    let vcl_hrd = r.read_flag()?;
    let mut sub_pic = false;
    if nal_hrd || vcl_hrd {
        sub_pic = r.read_flag()?;
        if sub_pic {
            r.read_bits(8)?;
            r.read_bits(5)?;
            r.read_flag()?;
            r.read_bits(5)?;
        }
        r.read_bits(8)?;
        if sub_pic {
            r.read_bits(4)?;
        }
        r.read_bits(15)?;
    }
    for _ in 0..=max_sub_layers_minus1 {
        let fixed_general = r.read_flag()?;
        let fixed_within_cvs = if fixed_general { true } else { r.read_flag()? };
        let mut low_delay = false;
        if fixed_within_cvs {
            r.read_ue()?;
        } else {
            low_delay = r.read_flag()?;
        }
        let cpb_cnt = if low_delay { 1 } else { r.read_ue_max(31)? + 1 };
        if nal_hrd {
            skip_sub_layer_hrd(r, cpb_cnt, sub_pic)?;
        }
        if vcl_hrd {
            skip_sub_layer_hrd(r, cpb_cnt, sub_pic)?;
        }
    }
    Ok(())
}

/// What the page needs to know of the picture's colour (Annex E), as the
/// stream states it, or `2` (unspecified) where it does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Colour {
    pub full_range: bool,
    pub primaries: u8,
    pub transfer: u8,
    pub matrix: u8,
}

impl Default for Colour {
    fn default() -> Self {
        Colour { full_range: false, primaries: 2, transfer: 2, matrix: 2 }
    }
}

fn parse_vui(r: &mut BitReader, max_sub_layers_minus1: u32) -> Result<Colour> {
    let mut c = Colour::default();
    if r.read_flag()? && r.read_bits(8)? == 255 {
        r.read_bits(32)?; // sar
    }
    if r.read_flag()? {
        r.read_flag()?; // overscan_appropriate_flag
    }
    if r.read_flag()? {
        r.read_bits(3)?; // video_format
        c.full_range = r.read_flag()?;
        if r.read_flag()? {
            c.primaries = r.read_bits(8)? as u8;
            c.transfer = r.read_bits(8)? as u8;
            c.matrix = r.read_bits(8)? as u8;
        }
    }
    if r.read_flag()? {
        r.read_ue()?;
        r.read_ue()?; // chroma sample locations
    }
    r.read_bits(3)?; // neutral chroma, field seq, frame field info
    if r.read_flag()? {
        for _ in 0..4 {
            r.read_ue()?; // default display window
        }
    }
    if r.read_flag()? {
        r.read_bits(32)?;
        r.read_bits(32)?;
        if r.read_flag()? {
            r.read_ue()?;
        }
        if r.read_flag()? {
            skip_hrd(r, max_sub_layers_minus1)?;
        }
    }
    if r.read_flag()? {
        r.read_bits(3)?;
        for _ in 0..5 {
            r.read_ue()?;
        }
    }
    Ok(c)
}

/// A short-term reference picture set: POC deltas and whether the current
/// picture uses each, negatives then positives.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShortTermRps {
    pub neg: Vec<(i32, bool)>,
    pub pos: Vec<(i32, bool)>,
}

impl ShortTermRps {
    pub fn num_delta_pocs(&self) -> usize {
        self.neg.len() + self.pos.len()
    }
}

/// `st_ref_pic_set(idx)` (§7.3.7), predicted from `sets` where it says so.
pub fn parse_st_ref_pic_set(r: &mut BitReader, idx: usize, sets: &[ShortTermRps], in_slice_header: bool) -> Result<ShortTermRps> {
    let inter_rps_pred = if idx != 0 { r.read_flag()? } else { false };
    let mut rps = ShortTermRps::default();
    if inter_rps_pred {
        let delta_idx_minus1 = if in_slice_header { r.read_ue_max(idx as u32 - 1)? as usize } else { 0 };
        let delta_rps_sign = r.read_flag()?;
        let abs_delta_rps_minus1 = r.read_ue_max(32767)?;
        let rf = sets.get(idx - (delta_idx_minus1 + 1)).ok_or_else(|| Error::invalid("st_ref_pic_set reference index"))?;
        let delta_rps = (1 - 2 * delta_rps_sign as i32) * (abs_delta_rps_minus1 as i32 + 1);
        let n = rf.num_delta_pocs();
        let mut used = vec![false; n + 1];
        let mut use_delta = vec![true; n + 1];
        for j in 0..=n {
            used[j] = r.read_flag()?;
            if !used[j] {
                use_delta[j] = r.read_flag()?;
            }
        }
        for j in (0..rf.pos.len()).rev() {
            let d = rf.pos[j].0 + delta_rps;
            let k = rf.neg.len() + j;
            if d < 0 && use_delta[k] {
                rps.neg.push((d, used[k]));
            }
        }
        if delta_rps < 0 && use_delta[n] {
            rps.neg.push((delta_rps, used[n]));
        }
        for j in 0..rf.neg.len() {
            let d = rf.neg[j].0 + delta_rps;
            if d < 0 && use_delta[j] {
                rps.neg.push((d, used[j]));
            }
        }
        for j in (0..rf.neg.len()).rev() {
            let d = rf.neg[j].0 + delta_rps;
            if d > 0 && use_delta[j] {
                rps.pos.push((d, used[j]));
            }
        }
        if delta_rps > 0 && use_delta[n] {
            rps.pos.push((delta_rps, used[n]));
        }
        for j in 0..rf.pos.len() {
            let d = rf.pos[j].0 + delta_rps;
            let k = rf.neg.len() + j;
            if d > 0 && use_delta[k] {
                rps.pos.push((d, used[k]));
            }
        }
    } else {
        let num_negative = r.read_ue_max(16)?;
        let num_positive = r.read_ue_max(16)?;
        let mut prev = 0i32;
        for _ in 0..num_negative {
            prev -= r.read_ue_max(32767)? as i32 + 1;
            rps.neg.push((prev, r.read_flag()?));
        }
        prev = 0;
        for _ in 0..num_positive {
            prev += r.read_ue_max(32767)? as i32 + 1;
            rps.pos.push((prev, r.read_flag()?));
        }
    }
    if rps.num_delta_pocs() > 16 {
        return Err(Error::invalid("more than 16 pictures in a reference picture set"));
    }
    Ok(rps)
}

#[derive(Debug, Clone)]
pub struct Sps {
    pub id: u8,
    pub width: u32,
    pub height: u32,
    /// `conf_win_*_offset` in luma samples: left, right, top, bottom.
    pub conf_win: [u32; 4],
    pub log2_max_poc_lsb: u8,
    pub max_dec_pic_buffering: u32,
    pub log2_min_cb_size: u8,
    pub log2_ctb_size: u8,
    pub log2_min_tb_size: u8,
    pub log2_max_tb_size: u8,
    pub max_transform_hierarchy_depth_inter: u8,
    pub max_transform_hierarchy_depth_intra: u8,
    pub sao_enabled: bool,
    pub st_rps: Vec<ShortTermRps>,
    pub temporal_mvp_enabled: bool,
    pub colour: Colour,
    pub pic_width_in_ctbs: u32,
    pub pic_height_in_ctbs: u32,
}

impl Sps {
    /// The cropped size.
    pub fn output_size(&self) -> (u32, u32) {
        (self.width - self.conf_win[0] - self.conf_win[1], self.height - self.conf_win[2] - self.conf_win[3])
    }
}

pub fn parse_sps(rbsp: &[u8]) -> Result<Sps> {
    let mut r = BitReader::new(rbsp);
    r.read_bits(4)?; // vps id
    let max_sub_layers_minus1 = r.read_bits(3)?;
    if max_sub_layers_minus1 > 6 {
        return Err(Error::invalid("sps_max_sub_layers_minus1"));
    }
    r.read_flag()?; // temporal id nesting
    skip_ptl(&mut r, max_sub_layers_minus1)?;
    let id = r.read_ue_max(15)? as u8;
    let chroma_format_idc = r.read_ue_max(3)?;
    if chroma_format_idc != 3 {
        return Err(unsupported("a chroma format other than 4:4:4"));
    }
    if r.read_flag()? {
        return Err(unsupported("separate colour planes"));
    }
    let width = r.read_ue_max(16888)?;
    let height = r.read_ue_max(16888)?;
    // The largest picture any level allows: what a picture's planes and
    // maps are allocated for at most.
    let luma_ps = u64::from(width) * u64::from(height);
    if luma_ps > MAX_LUMA_PS {
        return Err(Error::unsupported("a picture larger than level 6.2 allows"));
    }
    let mut conf_win = [0u32; 4];
    if r.read_flag()? {
        for o in conf_win.iter_mut() {
            *o = r.read_ue_max(16888)?;
        }
    }
    if conf_win[0] + conf_win[1] >= width || conf_win[2] + conf_win[3] >= height {
        return Err(Error::invalid("conformance window"));
    }
    if r.read_ue_max(8)? != 0 || r.read_ue_max(8)? != 0 {
        return Err(unsupported("a bit depth other than 8"));
    }
    let log2_max_poc_lsb = r.read_ue_max(12)? as u8 + 4;
    let ordering_present = r.read_flag()?;
    let mut max_dec_pic_buffering = 0;
    for _ in (if ordering_present { 0 } else { max_sub_layers_minus1 })..=max_sub_layers_minus1 {
        max_dec_pic_buffering = r.read_ue_max(16)? + 1;
        if r.read_ue_max(16)? != 0 {
            return Err(unsupported("picture reordering"));
        }
        r.read_ue()?; // max latency increase
    }
    // A.4.2: the pictures a DPB holds, fewer the larger they are, against
    // the largest any level allows. What a reference picture set may ask
    // the decoder to allocate (`decoder`) is bounded by it.
    let max_dpb_size = if luma_ps <= MAX_LUMA_PS / 4 {
        16
    } else if luma_ps <= MAX_LUMA_PS / 2 {
        12
    } else if luma_ps <= MAX_LUMA_PS * 3 / 4 {
        8
    } else {
        6
    };
    if max_dec_pic_buffering > max_dpb_size {
        return Err(Error::invalid("sps_max_dec_pic_buffering beyond the level's DPB"));
    }
    let log2_min_cb_size = r.read_ue_max(3)? as u8 + 3;
    let log2_ctb_size = log2_min_cb_size + r.read_ue_max(3)? as u8;
    if !(4..=6).contains(&log2_ctb_size) {
        return Err(Error::invalid("coding tree block size"));
    }
    let log2_min_tb_size = r.read_ue_max(3)? as u8 + 2;
    let log2_max_tb_size = log2_min_tb_size + r.read_ue_max(3)? as u8;
    if log2_max_tb_size > 5 || log2_max_tb_size > log2_ctb_size || log2_min_tb_size >= log2_min_cb_size {
        return Err(Error::invalid("transform block sizes"));
    }
    let max_transform_hierarchy_depth_inter = r.read_ue_max(4)? as u8;
    let max_transform_hierarchy_depth_intra = r.read_ue_max(4)? as u8;
    if r.read_flag()? {
        return Err(unsupported("scaling lists"));
    }
    if r.read_flag()? {
        return Err(unsupported("asymmetric motion partitions"));
    }
    let sao_enabled = r.read_flag()?;
    if r.read_flag()? {
        return Err(unsupported("PCM"));
    }
    let num_st_rps = r.read_ue_max(64)? as usize;
    let mut st_rps = Vec::with_capacity(num_st_rps);
    for i in 0..num_st_rps {
        let set = parse_st_ref_pic_set(&mut r, i, &st_rps, false)?;
        st_rps.push(set);
    }
    if r.read_flag()? {
        return Err(unsupported("long-term reference pictures"));
    }
    let temporal_mvp_enabled = r.read_flag()?;
    if r.read_flag()? {
        return Err(unsupported("strong intra smoothing"));
    }
    let colour = if r.read_flag()? { parse_vui(&mut r, max_sub_layers_minus1)? } else { Colour::default() };
    if r.read_flag()? {
        return Err(unsupported("an SPS extension"));
    }
    let min_cb = 1 << log2_min_cb_size;
    if width % min_cb != 0 || height % min_cb != 0 {
        return Err(Error::invalid("picture size not a multiple of the minimum coding block"));
    }
    let ctb = 1 << log2_ctb_size;
    Ok(Sps {
        id,
        width,
        height,
        conf_win,
        log2_max_poc_lsb,
        max_dec_pic_buffering,
        log2_min_cb_size,
        log2_ctb_size,
        log2_min_tb_size,
        log2_max_tb_size,
        max_transform_hierarchy_depth_inter,
        max_transform_hierarchy_depth_intra,
        sao_enabled,
        st_rps,
        temporal_mvp_enabled,
        colour,
        pic_width_in_ctbs: width.div_ceil(ctb),
        pic_height_in_ctbs: height.div_ceil(ctb),
    })
}

#[derive(Debug, Clone)]
pub struct Pps {
    pub id: u8,
    pub sps_id: u8,
    pub num_extra_slice_header_bits: u8,
    pub num_ref_idx_l0_default_active: u8,
    pub init_qp: i32,
    pub cu_qp_delta_enabled: bool,
    pub diff_cu_qp_delta_depth: u8,
    pub cb_qp_offset: i32,
    pub cr_qp_offset: i32,
    pub slice_chroma_qp_offsets_present: bool,
    pub loop_filter_across_slices_enabled: bool,
    pub deblocking_filter_override_enabled: bool,
    pub deblocking_filter_disabled: bool,
    pub beta_offset_div2: i32,
    pub tc_offset_div2: i32,
    pub log2_parallel_merge_level: u8,
    pub slice_segment_header_extension_present: bool,
}

pub fn parse_pps(rbsp: &[u8]) -> Result<Pps> {
    let mut r = BitReader::new(rbsp);
    let id = r.read_ue_max(63)? as u8;
    let sps_id = r.read_ue_max(15)? as u8;
    if r.read_flag()? {
        return Err(unsupported("dependent slice segments"));
    }
    if r.read_flag()? {
        return Err(unsupported("pic_output_flag"));
    }
    let num_extra_slice_header_bits = r.read_bits(3)? as u8;
    if r.read_flag()? {
        return Err(unsupported("sign data hiding"));
    }
    if r.read_flag()? {
        return Err(unsupported("cabac_init_flag"));
    }
    let num_ref_idx_l0_default_active = r.read_ue_max(14)? as u8 + 1;
    r.read_ue_max(14)?; // list 1's default, for B slices
    let init_qp = 26 + r.read_se()?;
    if !(0..=51).contains(&init_qp) {
        return Err(Error::invalid("init_qp"));
    }
    if r.read_flag()? {
        return Err(unsupported("constrained intra prediction"));
    }
    if r.read_flag()? {
        return Err(unsupported("transform skip"));
    }
    let cu_qp_delta_enabled = r.read_flag()?;
    let diff_cu_qp_delta_depth = if cu_qp_delta_enabled { r.read_ue_max(3)? as u8 } else { 0 };
    let cb_qp_offset = r.read_se()?;
    let cr_qp_offset = r.read_se()?;
    if !(-12..=12).contains(&cb_qp_offset) || !(-12..=12).contains(&cr_qp_offset) {
        return Err(Error::invalid("chroma QP offsets"));
    }
    let slice_chroma_qp_offsets_present = r.read_flag()?;
    if r.read_flag()? {
        return Err(unsupported("weighted prediction"));
    }
    r.read_flag()?; // weighted_bipred_flag, for B slices
    if r.read_flag()? {
        return Err(unsupported("transquant bypass"));
    }
    if r.read_flag()? {
        return Err(unsupported("tiles"));
    }
    if !r.read_flag()? {
        return Err(unsupported("no wavefront (entropy_coding_sync_enabled_flag is 0)"));
    }
    let loop_filter_across_slices_enabled = r.read_flag()?;
    let mut deblocking_filter_override_enabled = false;
    let mut deblocking_filter_disabled = false;
    let (mut beta_offset_div2, mut tc_offset_div2) = (0, 0);
    if r.read_flag()? {
        deblocking_filter_override_enabled = r.read_flag()?;
        deblocking_filter_disabled = r.read_flag()?;
        if !deblocking_filter_disabled {
            beta_offset_div2 = r.read_se()?;
            tc_offset_div2 = r.read_se()?;
            if !(-6..=6).contains(&beta_offset_div2) || !(-6..=6).contains(&tc_offset_div2) {
                return Err(Error::invalid("deblocking offsets"));
            }
        }
    }
    if r.read_flag()? {
        return Err(unsupported("scaling lists"));
    }
    if r.read_flag()? {
        return Err(unsupported("reference list modification"));
    }
    let log2_parallel_merge_level = r.read_ue_max(4)? as u8 + 2;
    let slice_segment_header_extension_present = r.read_flag()?;
    if r.read_flag()? {
        return Err(unsupported("a PPS extension"));
    }
    Ok(Pps {
        id,
        sps_id,
        num_extra_slice_header_bits,
        num_ref_idx_l0_default_active,
        init_qp,
        cu_qp_delta_enabled,
        diff_cu_qp_delta_depth,
        cb_qp_offset,
        cr_qp_offset,
        slice_chroma_qp_offsets_present,
        loop_filter_across_slices_enabled,
        deblocking_filter_override_enabled,
        deblocking_filter_disabled,
        beta_offset_div2,
        tc_offset_div2,
        log2_parallel_merge_level,
        slice_segment_header_extension_present,
    })
}
