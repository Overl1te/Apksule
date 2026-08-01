#![allow(clippy::many_single_char_names, clippy::too_many_lines, dead_code)]

const HEADER_SIZE: usize = 0x70;

pub fn minimal_activity_dex() -> Vec<u8> {
    let strings = [
        "Ldev/apksule/Bridge;",
        "Ldev/apksule/m2/MainActivity;",
        "Ljava/lang/Object;",
        "V",
        "markReached",
        "onCreate",
    ];

    let string_ids_off = HEADER_SIZE;
    let type_ids_off = string_ids_off + strings.len() * 4;
    let proto_ids_off = type_ids_off + 4 * 4;
    let method_ids_off = proto_ids_off + 12;
    let class_defs_off = method_ids_off + 2 * 8;
    let data_off = class_defs_off + 32;

    let mut data = Vec::new();
    let mut string_offsets = Vec::new();
    for value in strings {
        string_offsets.push(u32_at(data_off + data.len()));
        push_uleb128(&mut data, u32::try_from(value.encode_utf16().count()).expect("string size"));
        data.extend_from_slice(value.as_bytes());
        data.push(0);
    }
    align4(&mut data);

    let code_off = u32_at(data_off + data.len());
    push_u16(&mut data, 1); // registers_size
    push_u16(&mut data, 1); // ins_size (receiver)
    push_u16(&mut data, 0); // outs_size
    push_u16(&mut data, 0); // tries_size
    push_u32(&mut data, 0); // debug_info_off
    push_u32(&mut data, 4); // insns_size in 16-bit code units
    push_u16(&mut data, 0x0071); // invoke-static {}, method@0
    push_u16(&mut data, 0);
    push_u16(&mut data, 0);
    push_u16(&mut data, 0x000e); // return-void

    let class_data_off = u32_at(data_off + data.len());
    push_uleb128(&mut data, 0); // static_fields_size
    push_uleb128(&mut data, 0); // instance_fields_size
    push_uleb128(&mut data, 0); // direct_methods_size
    push_uleb128(&mut data, 1); // virtual_methods_size
    push_uleb128(&mut data, 1); // method_idx_diff => onCreate
    push_uleb128(&mut data, 1); // public
    push_uleb128(&mut data, code_off);
    align4(&mut data);

    let map_off = u32_at(data_off + data.len());
    let map_entries = [
        (0x0000, 1, 0),
        (0x0001, u32::try_from(strings.len()).expect("string count"), u32_at(string_ids_off)),
        (0x0002, 4, u32_at(type_ids_off)),
        (0x0003, 1, u32_at(proto_ids_off)),
        (0x0005, 2, u32_at(method_ids_off)),
        (0x0006, 1, u32_at(class_defs_off)),
        (0x2002, u32::try_from(strings.len()).expect("string count"), u32_at(data_off)),
        (0x2001, 1, code_off),
        (0x2000, 1, class_data_off),
        (0x1000, 1, map_off),
    ];
    push_u32(&mut data, u32::try_from(map_entries.len()).expect("map count"));
    for (item_type, size, offset) in map_entries {
        push_u16(&mut data, item_type);
        push_u16(&mut data, 0);
        push_u32(&mut data, size);
        push_u32(&mut data, offset);
    }

    let file_size = data_off + data.len();
    let mut bytes = vec![0; data_off];
    bytes.extend_from_slice(&data);

    bytes[0..8].copy_from_slice(b"dex\n035\0");
    write_u32(&mut bytes, 32, u32_at(file_size));
    write_u32(&mut bytes, 36, u32_at(HEADER_SIZE));
    write_u32(&mut bytes, 40, 0x1234_5678);
    write_u32(&mut bytes, 52, map_off);
    write_u32(&mut bytes, 56, u32::try_from(strings.len()).expect("string count"));
    write_u32(&mut bytes, 60, u32_at(string_ids_off));
    write_u32(&mut bytes, 64, 4);
    write_u32(&mut bytes, 68, u32_at(type_ids_off));
    write_u32(&mut bytes, 72, 1);
    write_u32(&mut bytes, 76, u32_at(proto_ids_off));
    write_u32(&mut bytes, 80, 0);
    write_u32(&mut bytes, 84, 0);
    write_u32(&mut bytes, 88, 2);
    write_u32(&mut bytes, 92, u32_at(method_ids_off));
    write_u32(&mut bytes, 96, 1);
    write_u32(&mut bytes, 100, u32_at(class_defs_off));
    write_u32(&mut bytes, 104, u32_at(file_size - data_off));
    write_u32(&mut bytes, 108, u32_at(data_off));

    for (index, offset) in string_offsets.into_iter().enumerate() {
        write_u32(&mut bytes, string_ids_off + index * 4, offset);
    }
    for (index, descriptor_idx) in [0_u32, 1, 2, 3].into_iter().enumerate() {
        write_u32(&mut bytes, type_ids_off + index * 4, descriptor_idx);
    }

    write_u32(&mut bytes, proto_ids_off, 3); // shorty_idx "V"
    write_u32(&mut bytes, proto_ids_off + 4, 3); // return_type_idx V
    write_u32(&mut bytes, proto_ids_off + 8, 0); // no parameters

    write_u16(&mut bytes, method_ids_off, 0); // Bridge
    write_u16(&mut bytes, method_ids_off + 2, 0);
    write_u32(&mut bytes, method_ids_off + 4, 4); // markReached
    write_u16(&mut bytes, method_ids_off + 8, 1); // MainActivity
    write_u16(&mut bytes, method_ids_off + 10, 0);
    write_u32(&mut bytes, method_ids_off + 12, 5); // onCreate

    write_u32(&mut bytes, class_defs_off, 1); // MainActivity type
    write_u32(&mut bytes, class_defs_off + 4, 1); // public
    write_u32(&mut bytes, class_defs_off + 8, 2); // Object superclass
    write_u32(&mut bytes, class_defs_off + 12, 0);
    write_u32(&mut bytes, class_defs_off + 16, u32::MAX);
    write_u32(&mut bytes, class_defs_off + 20, 0);
    write_u32(&mut bytes, class_defs_off + 24, class_data_off);
    write_u32(&mut bytes, class_defs_off + 28, 0);

    let signature = sha1(&bytes[32..]);
    bytes[12..32].copy_from_slice(&signature);
    let checksum = adler32(&bytes[12..]);
    write_u32(&mut bytes, 8, checksum);
    bytes
}

/// M3 Activity: `onCreate` calls `Bridge.markReached` then `Bridge.setContentView`.
pub fn minimal_m3_activity_dex() -> Vec<u8> {
    let strings = [
        "Ldev/apksule/Bridge;",
        "Ldev/apksule/m3/MainActivity;",
        "Ljava/lang/Object;",
        "V",
        "markReached",
        "setContentView",
        "onCreate",
    ];

    let string_ids_off = HEADER_SIZE;
    let type_ids_off = string_ids_off + strings.len() * 4;
    let proto_ids_off = type_ids_off + 4 * 4;
    let method_ids_off = proto_ids_off + 12;
    let class_defs_off = method_ids_off + 3 * 8;
    let data_off = class_defs_off + 32;

    let mut data = Vec::new();
    let mut string_offsets = Vec::new();
    for value in strings {
        string_offsets.push(u32_at(data_off + data.len()));
        push_uleb128(&mut data, u32::try_from(value.encode_utf16().count()).expect("string size"));
        data.extend_from_slice(value.as_bytes());
        data.push(0);
    }
    align4(&mut data);

    let code_off = u32_at(data_off + data.len());
    push_u16(&mut data, 1); // registers_size
    push_u16(&mut data, 1); // ins_size
    push_u16(&mut data, 0); // outs_size
    push_u16(&mut data, 0); // tries_size
    push_u32(&mut data, 0); // debug_info_off
    push_u32(&mut data, 7); // insns_size
    push_u16(&mut data, 0x0071); // invoke-static {}, method@0 markReached
    push_u16(&mut data, 0);
    push_u16(&mut data, 0);
    push_u16(&mut data, 0x0071); // invoke-static {}, method@1 setContentView
    push_u16(&mut data, 1);
    push_u16(&mut data, 0);
    push_u16(&mut data, 0x000e); // return-void

    let class_data_off = u32_at(data_off + data.len());
    push_uleb128(&mut data, 0);
    push_uleb128(&mut data, 0);
    push_uleb128(&mut data, 0);
    push_uleb128(&mut data, 1); // virtual_methods_size
    push_uleb128(&mut data, 2); // method_idx_diff => onCreate @2
    push_uleb128(&mut data, 1); // public
    push_uleb128(&mut data, code_off);
    align4(&mut data);

    let map_off = u32_at(data_off + data.len());
    let map_entries = [
        (0x0000, 1, 0),
        (0x0001, u32::try_from(strings.len()).expect("string count"), u32_at(string_ids_off)),
        (0x0002, 4, u32_at(type_ids_off)),
        (0x0003, 1, u32_at(proto_ids_off)),
        (0x0005, 3, u32_at(method_ids_off)),
        (0x0006, 1, u32_at(class_defs_off)),
        (0x2002, u32::try_from(strings.len()).expect("string count"), u32_at(data_off)),
        (0x2001, 1, code_off),
        (0x2000, 1, class_data_off),
        (0x1000, 1, map_off),
    ];
    push_u32(&mut data, u32::try_from(map_entries.len()).expect("map count"));
    for (item_type, size, offset) in map_entries {
        push_u16(&mut data, item_type);
        push_u16(&mut data, 0);
        push_u32(&mut data, size);
        push_u32(&mut data, offset);
    }

    let file_size = data_off + data.len();
    let mut bytes = vec![0; data_off];
    bytes.extend_from_slice(&data);

    bytes[0..8].copy_from_slice(b"dex\n035\0");
    write_u32(&mut bytes, 32, u32_at(file_size));
    write_u32(&mut bytes, 36, u32_at(HEADER_SIZE));
    write_u32(&mut bytes, 40, 0x1234_5678);
    write_u32(&mut bytes, 52, map_off);
    write_u32(&mut bytes, 56, u32::try_from(strings.len()).expect("string count"));
    write_u32(&mut bytes, 60, u32_at(string_ids_off));
    write_u32(&mut bytes, 64, 4);
    write_u32(&mut bytes, 68, u32_at(type_ids_off));
    write_u32(&mut bytes, 72, 1);
    write_u32(&mut bytes, 76, u32_at(proto_ids_off));
    write_u32(&mut bytes, 80, 0);
    write_u32(&mut bytes, 84, 0);
    write_u32(&mut bytes, 88, 3);
    write_u32(&mut bytes, 92, u32_at(method_ids_off));
    write_u32(&mut bytes, 96, 1);
    write_u32(&mut bytes, 100, u32_at(class_defs_off));
    write_u32(&mut bytes, 104, u32_at(file_size - data_off));
    write_u32(&mut bytes, 108, u32_at(data_off));

    for (index, offset) in string_offsets.into_iter().enumerate() {
        write_u32(&mut bytes, string_ids_off + index * 4, offset);
    }
    for (index, descriptor_idx) in [0_u32, 1, 2, 3].into_iter().enumerate() {
        write_u32(&mut bytes, type_ids_off + index * 4, descriptor_idx);
    }

    write_u32(&mut bytes, proto_ids_off, 3);
    write_u32(&mut bytes, proto_ids_off + 4, 3);
    write_u32(&mut bytes, proto_ids_off + 8, 0);

    write_u16(&mut bytes, method_ids_off, 0); // Bridge.markReached
    write_u16(&mut bytes, method_ids_off + 2, 0);
    write_u32(&mut bytes, method_ids_off + 4, 4);
    write_u16(&mut bytes, method_ids_off + 8, 0); // Bridge.setContentView
    write_u16(&mut bytes, method_ids_off + 10, 0);
    write_u32(&mut bytes, method_ids_off + 12, 5);
    write_u16(&mut bytes, method_ids_off + 16, 1); // MainActivity.onCreate
    write_u16(&mut bytes, method_ids_off + 18, 0);
    write_u32(&mut bytes, method_ids_off + 20, 6);

    write_u32(&mut bytes, class_defs_off, 1);
    write_u32(&mut bytes, class_defs_off + 4, 1);
    write_u32(&mut bytes, class_defs_off + 8, 2);
    write_u32(&mut bytes, class_defs_off + 12, 0);
    write_u32(&mut bytes, class_defs_off + 16, u32::MAX);
    write_u32(&mut bytes, class_defs_off + 20, 0);
    write_u32(&mut bytes, class_defs_off + 24, class_data_off);
    write_u32(&mut bytes, class_defs_off + 28, 0);

    let signature = sha1(&bytes[32..]);
    bytes[12..32].copy_from_slice(&signature);
    let checksum = adler32(&bytes[12..]);
    write_u32(&mut bytes, 8, checksum);
    bytes
}

fn align4(bytes: &mut Vec<u8>) {
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
}

fn push_uleb128(bytes: &mut Vec<u8>, mut value: u32) {
    loop {
        let mut byte = u8::try_from(value & 0x7f).expect("seven bits fit");
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        bytes.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn u32_at(value: usize) -> u32 {
    u32::try_from(value).expect("fixture offset fits u32")
}

fn adler32(bytes: &[u8]) -> u32 {
    const MOD_ADLER: u32 = 65_521;
    let mut a = 1_u32;
    let mut b = 0_u32;
    for byte in bytes {
        a = (a + u32::from(*byte)) % MOD_ADLER;
        b = (b + a) % MOD_ADLER;
    }
    (b << 16) | a
}

fn sha1(bytes: &[u8]) -> [u8; 20] {
    let bit_length = u64::try_from(bytes.len()).expect("fixture length").wrapping_mul(8);
    let mut padded = bytes.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_length.to_be_bytes());

    let mut state = [0x6745_2301_u32, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476, 0xc3d2_e1f0];
    for block in padded.chunks_exact(64) {
        let mut schedule = [0_u32; 80];
        for (index, word) in block.chunks_exact(4).take(16).enumerate() {
            schedule[index] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for index in 16..80 {
            schedule[index] = (schedule[index - 3]
                ^ schedule[index - 8]
                ^ schedule[index - 14]
                ^ schedule[index - 16])
                .rotate_left(1);
        }

        let [mut a, mut b, mut c, mut d, mut e] = state;
        for (index, word) in schedule.iter().enumerate() {
            let (function, constant) = match index {
                0..=19 => ((b & c) | ((!b) & d), 0x5a82_7999),
                20..=39 => (b ^ c ^ d, 0x6ed9_eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1b_bcdc),
                _ => (b ^ c ^ d, 0xca62_c1d6),
            };
            let next = a
                .rotate_left(5)
                .wrapping_add(function)
                .wrapping_add(e)
                .wrapping_add(constant)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = next;
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
    }

    let mut result = [0_u8; 20];
    for (index, value) in state.iter().enumerate() {
        result[index * 4..index * 4 + 4].copy_from_slice(&value.to_be_bytes());
    }
    result
}
