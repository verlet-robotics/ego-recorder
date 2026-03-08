/// Hand-rolled protobuf serialization for tf.train.Example.
/// Only the subset needed for RLDS TFRecord output.
///
/// Proto wire format reference:
///   - varint: field_number << 3 | wire_type
///   - wire_type 0 = varint, 2 = length-delimited
///
/// Message layout:
///   Example { Features features = 1; }
///   Features { map<string, Feature> feature = 1; }
///     maps encode as: message entry { string key = 1; Feature value = 2; }
///   Feature { oneof kind { BytesList=1, FloatList=2, Int64List=3 } }
///   BytesList { repeated bytes value = 1; }
///   FloatList { repeated float value = 1; }  // packed
///   Int64List { repeated int64 value = 1; }  // packed
use std::collections::HashMap;

/// A TensorFlow Feature value.
pub enum Feature {
    Bytes(Vec<Vec<u8>>),
    Float(Vec<f32>),
    Int64(Vec<i64>),
}

/// Encode a tf.train.Example with the given features map.
pub fn encode_example(features: &HashMap<String, Feature>) -> Vec<u8> {
    // Encode Features message (map entries)
    let features_bytes = encode_features(features);
    // Wrap in Example: field 1, wire type 2 (length-delimited)
    let mut out = Vec::new();
    write_tag(&mut out, 1, 2);
    write_len_delimited(&mut out, &features_bytes);
    out
}

fn encode_features(features: &HashMap<String, Feature>) -> Vec<u8> {
    let mut out = Vec::new();
    for (key, value) in features {
        // Map entry: field 1, wire type 2
        let entry_bytes = encode_map_entry(key, value);
        write_tag(&mut out, 1, 2);
        write_len_delimited(&mut out, &entry_bytes);
    }
    out
}

fn encode_map_entry(key: &str, value: &Feature) -> Vec<u8> {
    let mut out = Vec::new();
    // key: field 1, wire type 2 (string)
    write_tag(&mut out, 1, 2);
    write_len_delimited(&mut out, key.as_bytes());
    // value: field 2, wire type 2 (Feature message)
    let feature_bytes = encode_feature(value);
    write_tag(&mut out, 2, 2);
    write_len_delimited(&mut out, &feature_bytes);
    out
}

fn encode_feature(feature: &Feature) -> Vec<u8> {
    let mut out = Vec::new();
    match feature {
        Feature::Bytes(values) => {
            // BytesList: field 1, wire type 2
            let bytes_list = encode_bytes_list(values);
            write_tag(&mut out, 1, 2);
            write_len_delimited(&mut out, &bytes_list);
        }
        Feature::Float(values) => {
            // FloatList: field 2, wire type 2
            let float_list = encode_float_list(values);
            write_tag(&mut out, 2, 2);
            write_len_delimited(&mut out, &float_list);
        }
        Feature::Int64(values) => {
            // Int64List: field 3, wire type 2
            let int64_list = encode_int64_list(values);
            write_tag(&mut out, 3, 2);
            write_len_delimited(&mut out, &int64_list);
        }
    }
    out
}

fn encode_bytes_list(values: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for v in values {
        // repeated bytes value = 1; wire type 2
        write_tag(&mut out, 1, 2);
        write_len_delimited(&mut out, v);
    }
    out
}

fn encode_float_list(values: &[f32]) -> Vec<u8> {
    let mut out = Vec::new();
    if values.is_empty() {
        return out;
    }
    // packed: field 1, wire type 2
    write_tag(&mut out, 1, 2);
    let data_len = values.len() * 4;
    write_varint(&mut out, data_len as u64);
    for &v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

fn encode_int64_list(values: &[i64]) -> Vec<u8> {
    let mut out = Vec::new();
    if values.is_empty() {
        return out;
    }
    // packed: field 1, wire type 2
    write_tag(&mut out, 1, 2);
    // Compute total varint-encoded size
    let mut data = Vec::new();
    for &v in values {
        write_varint(&mut data, v as u64);
    }
    write_varint(&mut out, data.len() as u64);
    out.extend_from_slice(&data);
    out
}

fn write_tag(out: &mut Vec<u8>, field_number: u32, wire_type: u32) {
    write_varint(out, ((field_number << 3) | wire_type) as u64);
}

fn write_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn write_len_delimited(out: &mut Vec<u8>, data: &[u8]) {
    write_varint(out, data.len() as u64);
    out.extend_from_slice(data);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_empty_example() {
        let features = HashMap::new();
        let encoded = encode_example(&features);
        // Should produce a valid (small) protobuf
        assert!(!encoded.is_empty());
    }

    #[test]
    fn test_encode_bytes_feature() {
        let mut features = HashMap::new();
        features.insert("test".to_string(), Feature::Bytes(vec![b"hello".to_vec()]));
        let encoded = encode_example(&features);
        assert!(!encoded.is_empty());
    }

    #[test]
    fn test_encode_float_feature() {
        let mut features = HashMap::new();
        features.insert("vals".to_string(), Feature::Float(vec![1.0, 2.0, 3.0]));
        let encoded = encode_example(&features);
        assert!(!encoded.is_empty());
    }

    #[test]
    fn test_encode_int64_feature() {
        let mut features = HashMap::new();
        features.insert("ids".to_string(), Feature::Int64(vec![1, 0, 1]));
        let encoded = encode_example(&features);
        assert!(!encoded.is_empty());
    }
}
