// Splits an Annex B HEVC stream into access units, the units remotex hands the
// decoder one at a time (H.265 7.4.2.4.4).

export interface Nal {
  type: number;
  // Offset of the start code, including a leading zero byte.
  start: number;
  // Offset of the NAL unit header, just past the start code.
  header: number;
  end: number;
}

export interface AccessUnit {
  data: Uint8Array;
  // Holds an IRAP picture (IDR, CRA or BLA).
  keyframe: boolean;
  nals: number[];
}

export const NAL = { VPS: 32, SPS: 33, PPS: 34, AUD: 35, EOS: 36, EOB: 37, PREFIX_SEI: 39, SUFFIX_SEI: 40 } as const;

export const isVcl = (type: number) => type < 32;
export const isIrap = (type: number) => type >= 16 && type <= 23;

// Non-VCL units that, after a picture's slices, begin the next access unit.
const beginsUnit = (type: number) =>
  (type >= NAL.VPS && type <= NAL.AUD) ||
  type === NAL.PREFIX_SEI ||
  (type >= 41 && type <= 44) ||
  (type >= 48 && type <= 55);

export function splitNals(stream: Uint8Array): Nal[] {
  const found: { start: number; header: number }[] = [];
  for (let i = 0; i + 2 < stream.length; i++) {
    if (stream[i] === 0 && stream[i + 1] === 0 && stream[i + 2] === 1) {
      found.push({ start: i > 0 && stream[i - 1] === 0 ? i - 1 : i, header: i + 3 });
      i += 2;
    }
  }
  return found.map(({ start, header }, k) => ({
    type: ((stream[header] ?? 0) >> 1) & 0x3f,
    start,
    header,
    end: found[k + 1]?.start ?? stream.length,
  }));
}

export function accessUnits(stream: Uint8Array): AccessUnit[] {
  const units: AccessUnit[] = [];
  let current: { start: number; end: number; keyframe: boolean; vcl: boolean; nals: number[] } | null = null;
  const flush = () => {
    if (current) {
      units.push({ data: stream.subarray(current.start, current.end), keyframe: current.keyframe, nals: current.nals });
    }
    current = null;
  };
  for (const nal of splitNals(stream)) {
    // first_slice_segment_in_pic_flag: the first bit after the two-byte header.
    const firstSlice = isVcl(nal.type) && ((stream[nal.header + 2] ?? 0) & 0x80) !== 0;
    if (current?.vcl && (beginsUnit(nal.type) || firstSlice)) flush();
    current ??= { start: nal.start, end: nal.end, keyframe: false, vcl: false, nals: [] };
    current.end = nal.end;
    current.nals.push(nal.type);
    if (isVcl(nal.type)) current.vcl = true;
    if (isIrap(nal.type)) current.keyframe = true;
  }
  flush();
  return units;
}
