/// Annex-B H.264 NAL unit parser and AVCC converter.
///
/// .egorec files store H.264 data in Annex-B format (start-code delimited).
/// MP4 containers require AVCC format (length-prefixed). This module handles
/// the conversion and extracts SPS/PPS for the avcC decoder configuration box.

/// A single NAL unit extracted from an Annex-B bitstream.
pub struct NalUnit {
    /// NAL unit type (first_byte & 0x1F). Key values:
    /// 1 = non-IDR slice, 5 = IDR slice, 6 = SEI, 7 = SPS, 8 = PPS
    pub nal_type: u8,
    /// Raw NAL bytes (excluding the start code, including the type byte).
    pub data: Vec<u8>,
}

/// Find the next Annex-B start code (00 00 01 or 00 00 00 01) at or after `pos`.
/// Returns (start_code_position, start_code_length) or None.
fn find_start_code(data: &[u8], pos: usize) -> Option<(usize, usize)> {
    if data.len() < pos + 3 {
        return None;
    }
    let mut i = pos;
    while i + 2 < data.len() {
        if data[i] == 0x00 && data[i + 1] == 0x00 {
            if data[i + 2] == 0x01 {
                // Check for 4-byte start code (00 00 00 01)
                if i > 0 && data[i - 1] == 0x00 {
                    return Some((i - 1, 4));
                }
                return Some((i, 3));
            }
            // 00 00 00 prefix — advance past the first 00
            if data[i + 2] == 0x00 {
                i += 1;
                continue;
            }
        }
        i += 1;
    }
    None
}

/// Split an Annex-B H.264 bitstream into individual NAL units.
pub fn parse_annex_b(data: &[u8]) -> Vec<NalUnit> {
    let mut nals = Vec::new();
    if data.is_empty() {
        return nals;
    }

    // Find the first start code
    let Some((first_sc_pos, first_sc_len)) = find_start_code(data, 0) else {
        return nals;
    };

    let mut nal_data_start = first_sc_pos + first_sc_len;

    loop {
        // Find the next start code (marks the end of the current NAL)
        let nal_data_end;
        let next_start;

        match find_start_code(data, nal_data_start) {
            Some((sc_pos, sc_len)) => {
                // Strip trailing zero bytes that are part of the next start code
                nal_data_end = sc_pos;
                next_start = Some(sc_pos + sc_len);
            }
            None => {
                nal_data_end = data.len();
                next_start = None;
            }
        }

        if nal_data_start < nal_data_end {
            let nal_bytes = &data[nal_data_start..nal_data_end];
            let nal_type = nal_bytes[0] & 0x1F;
            nals.push(NalUnit {
                nal_type,
                data: nal_bytes.to_vec(),
            });
        }

        match next_start {
            Some(ns) => nal_data_start = ns,
            None => break,
        }
    }

    nals
}

/// Convert parsed NAL units to AVCC format (4-byte big-endian length prefix per NAL).
pub fn nals_to_avcc(nals: &[NalUnit]) -> Vec<u8> {
    let total_size: usize = nals.iter().map(|n| 4 + n.data.len()).sum();
    let mut out = Vec::with_capacity(total_size);
    for nal in nals {
        out.extend_from_slice(&(nal.data.len() as u32).to_be_bytes());
        out.extend_from_slice(&nal.data);
    }
    out
}

/// Extract SPS and PPS NAL unit data from a set of parsed NALs.
/// Returns (sps_data, pps_data) where each includes the NAL type byte.
pub fn extract_sps_pps(nals: &[NalUnit]) -> Option<(Vec<u8>, Vec<u8>)> {
    let sps = nals.iter().find(|n| n.nal_type == 7)?;
    let pps = nals.iter().find(|n| n.nal_type == 8)?;
    Some((sps.data.clone(), pps.data.clone()))
}

/// Check if a set of NALs represents a keyframe (contains IDR slice or SPS).
pub fn is_keyframe(nals: &[NalUnit]) -> bool {
    nals.iter().any(|n| n.nal_type == 5 || n.nal_type == 7)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_nal_4byte_start_code() {
        // 00 00 00 01 [type=0x65 (IDR, type 5)] [payload]
        let data = [0x00, 0x00, 0x00, 0x01, 0x65, 0xAA, 0xBB];
        let nals = parse_annex_b(&data);
        assert_eq!(nals.len(), 1);
        assert_eq!(nals[0].nal_type, 5);
        assert_eq!(nals[0].data, &[0x65, 0xAA, 0xBB]);
    }

    #[test]
    fn parse_single_nal_3byte_start_code() {
        let data = [0x00, 0x00, 0x01, 0x41, 0xCC];
        let nals = parse_annex_b(&data);
        assert_eq!(nals.len(), 1);
        assert_eq!(nals[0].nal_type, 1); // non-IDR
        assert_eq!(nals[0].data, &[0x41, 0xCC]);
    }

    #[test]
    fn parse_multiple_nals_keyframe() {
        // SPS + PPS + IDR with mixed start code lengths
        let mut data = Vec::new();
        // SPS (4-byte start code)
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        data.extend_from_slice(&[0x67, 0x42, 0x00, 0x1E]); // type 7 = SPS
        // PPS (3-byte start code)
        data.extend_from_slice(&[0x00, 0x00, 0x01]);
        data.extend_from_slice(&[0x68, 0xCE, 0x38, 0x80]); // type 8 = PPS
        // IDR (4-byte start code)
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        data.extend_from_slice(&[0x65, 0x88, 0x80]); // type 5 = IDR

        let nals = parse_annex_b(&data);
        assert_eq!(nals.len(), 3);
        assert_eq!(nals[0].nal_type, 7); // SPS
        assert_eq!(nals[1].nal_type, 8); // PPS
        assert_eq!(nals[2].nal_type, 5); // IDR
        assert!(is_keyframe(&nals));
    }

    #[test]
    fn parse_p_frame_single_nal() {
        let data = [0x00, 0x00, 0x00, 0x01, 0x41, 0x9A, 0x24, 0xFF];
        let nals = parse_annex_b(&data);
        assert_eq!(nals.len(), 1);
        assert_eq!(nals[0].nal_type, 1);
        assert!(!is_keyframe(&nals));
    }

    #[test]
    fn avcc_conversion_round_trip() {
        let nals = vec![
            NalUnit { nal_type: 7, data: vec![0x67, 0x42, 0x00] },
            NalUnit { nal_type: 8, data: vec![0x68, 0xCE] },
            NalUnit { nal_type: 5, data: vec![0x65, 0x88, 0x80, 0x40] },
        ];
        let avcc = nals_to_avcc(&nals);
        // 4 + 3 + 4 + 2 + 4 + 4 = 21 bytes
        assert_eq!(avcc.len(), 21);
        // First NAL: length=3 in BE
        assert_eq!(&avcc[0..4], &[0x00, 0x00, 0x00, 0x03]);
        assert_eq!(&avcc[4..7], &[0x67, 0x42, 0x00]);
        // Second NAL: length=2 in BE
        assert_eq!(&avcc[7..11], &[0x00, 0x00, 0x00, 0x02]);
        assert_eq!(&avcc[11..13], &[0x68, 0xCE]);
    }

    #[test]
    fn extract_sps_pps_found() {
        let nals = vec![
            NalUnit { nal_type: 7, data: vec![0x67, 0x42] },
            NalUnit { nal_type: 8, data: vec![0x68, 0xCE] },
            NalUnit { nal_type: 5, data: vec![0x65] },
        ];
        let (sps, pps) = extract_sps_pps(&nals).unwrap();
        assert_eq!(sps, &[0x67, 0x42]);
        assert_eq!(pps, &[0x68, 0xCE]);
    }

    #[test]
    fn extract_sps_pps_missing() {
        let nals = vec![NalUnit { nal_type: 1, data: vec![0x41] }];
        assert!(extract_sps_pps(&nals).is_none());
    }

    #[test]
    fn empty_input() {
        assert!(parse_annex_b(&[]).is_empty());
        assert!(parse_annex_b(&[0x00, 0x00]).is_empty());
    }
}
