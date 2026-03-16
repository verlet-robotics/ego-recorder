/// Annex-B H.264 NAL unit parser and AVCC converter.

pub struct NalUnit {
    pub nal_type: u8,
    pub data: Vec<u8>,
}

fn find_start_code(data: &[u8], pos: usize) -> Option<(usize, usize)> {
    if data.len() < pos + 3 {
        return None;
    }
    let mut i = pos;
    while i + 2 < data.len() {
        if data[i] == 0x00 && data[i + 1] == 0x00 {
            if data[i + 2] == 0x01 {
                if i > 0 && data[i - 1] == 0x00 {
                    return Some((i - 1, 4));
                }
                return Some((i, 3));
            }
            if data[i + 2] == 0x00 {
                i += 1;
                continue;
            }
        }
        i += 1;
    }
    None
}

pub fn parse_annex_b(data: &[u8]) -> Vec<NalUnit> {
    let mut nals = Vec::new();
    if data.is_empty() {
        return nals;
    }

    let Some((first_sc_pos, first_sc_len)) = find_start_code(data, 0) else {
        return nals;
    };

    let mut nal_data_start = first_sc_pos + first_sc_len;

    loop {
        let nal_data_end;
        let next_start;

        match find_start_code(data, nal_data_start) {
            Some((sc_pos, sc_len)) => {
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

pub fn nals_to_avcc(nals: &[NalUnit]) -> Vec<u8> {
    let total_size: usize = nals.iter().map(|n| 4 + n.data.len()).sum();
    let mut out = Vec::with_capacity(total_size);
    for nal in nals {
        out.extend_from_slice(&(nal.data.len() as u32).to_be_bytes());
        out.extend_from_slice(&nal.data);
    }
    out
}

pub fn extract_sps_pps(nals: &[NalUnit]) -> Option<(Vec<u8>, Vec<u8>)> {
    let sps = nals.iter().find(|n| n.nal_type == 7)?;
    let pps = nals.iter().find(|n| n.nal_type == 8)?;
    Some((sps.data.clone(), pps.data.clone()))
}

pub fn is_keyframe(nals: &[NalUnit]) -> bool {
    nals.iter().any(|n| n.nal_type == 5 || n.nal_type == 7)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_multiple_nals_keyframe() {
        let mut data = Vec::new();
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        data.extend_from_slice(&[0x67, 0x42, 0x00, 0x1E]);
        data.extend_from_slice(&[0x00, 0x00, 0x01]);
        data.extend_from_slice(&[0x68, 0xCE, 0x38, 0x80]);
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        data.extend_from_slice(&[0x65, 0x88, 0x80]);

        let nals = parse_annex_b(&data);
        assert_eq!(nals.len(), 3);
        assert_eq!(nals[0].nal_type, 7);
        assert_eq!(nals[1].nal_type, 8);
        assert_eq!(nals[2].nal_type, 5);
        assert!(is_keyframe(&nals));
    }
}
