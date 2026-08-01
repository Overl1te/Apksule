#![allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::too_many_lines)]

use crate::parser::update_digests;

const HEADER_SIZE: usize = 0x70;

struct CodeSpec {
    method_idx: u32,
    registers: u16,
    ins: u16,
    outs: u16,
    instructions: Vec<u16>,
    caught_type: Option<u32>,
}

pub(crate) fn build_test_dex() -> Vec<u8> {
    let strings = [
        "LTest;",
        "Ljava/lang/Object;",
        "I",
        "V",
        "[I",
        "LNative;",
        "LException;",
        "<clinit>",
        "add",
        "branch",
        "array",
        "object",
        "double",
        "callNative",
        "loop",
        "staticField",
        "convert",
        "caught",
        "staticValue",
        "value",
        "II",
    ];
    let type_descriptor_indices = [0_u32, 1, 2, 3, 4, 5, 6];
    let method_ids = [
        (0_u16, 2_u16, 7_u32),
        (0, 0, 8),
        (0, 1, 9),
        (0, 0, 10),
        (0, 0, 11),
        (5, 1, 12),
        (0, 0, 13),
        (0, 2, 14),
        (0, 0, 15),
        (0, 0, 16),
        (0, 0, 17),
    ];
    let code = vec![
        CodeSpec {
            method_idx: 0,
            registers: 1,
            ins: 0,
            outs: 0,
            instructions: vec![0x6012, 0x0067, 0x0000, 0x000e],
            caught_type: None,
        },
        CodeSpec {
            method_idx: 1,
            registers: 2,
            ins: 0,
            outs: 0,
            instructions: vec![0x2012, 0x3112, 0x0090, 0x0100, 0x000f],
            caught_type: None,
        },
        CodeSpec {
            method_idx: 2,
            registers: 2,
            ins: 1,
            outs: 0,
            instructions: vec![0x013d, 0x0004, 0x1012, 0x000f, 0x2012, 0x000f],
            caught_type: None,
        },
        CodeSpec {
            method_idx: 3,
            registers: 4,
            ins: 0,
            outs: 0,
            instructions: vec![
                0x2012, 0x0123, 0x0004, 0x7212, 0x1312, 0x024b, 0x0301, 0x0044, 0x0301, 0x000f,
            ],
            caught_type: None,
        },
        CodeSpec {
            method_idx: 4,
            registers: 2,
            ins: 0,
            outs: 0,
            // new-instance v0; monitor-enter v0; const/16 v1,#9; iput/iget; monitor-exit v0; return
            instructions: vec![
                0x0022, 0x0000, 0x001d, 0x0113, 0x0009, 0x0159, 0x0001, 0x0152, 0x0001, 0x001e,
                0x010f,
            ],
            caught_type: None,
        },
        CodeSpec {
            method_idx: 6,
            registers: 1,
            ins: 0,
            outs: 1,
            instructions: vec![0x4012, 0x1071, 0x0005, 0x0000, 0x000a, 0x000f],
            caught_type: None,
        },
        CodeSpec {
            method_idx: 7,
            registers: 0,
            ins: 0,
            outs: 0,
            instructions: vec![0x0028],
            caught_type: None,
        },
        CodeSpec {
            method_idx: 8,
            registers: 1,
            ins: 0,
            outs: 0,
            instructions: vec![0x0060, 0x0000, 0x000f],
            caught_type: None,
        },
        CodeSpec {
            method_idx: 9,
            registers: 1,
            ins: 0,
            outs: 0,
            instructions: vec![0x0013, 0x0082, 0x008d, 0x000f],
            caught_type: None,
        },
        CodeSpec {
            method_idx: 10,
            registers: 2,
            ins: 0,
            outs: 0,
            instructions: vec![0x0022, 0x0006, 0x0027, 0x0012, 0x000d, 0x5112, 0x010f],
            caught_type: Some(6),
        },
    ];

    let string_ids_off = HEADER_SIZE;
    let type_ids_off = string_ids_off + strings.len() * 4;
    let proto_ids_off = type_ids_off + type_descriptor_indices.len() * 4;
    let field_ids_off = proto_ids_off + 3 * 12;
    let method_ids_off = field_ids_off + 2 * 8;
    let class_defs_off = method_ids_off + method_ids.len() * 8;
    let data_off = align(class_defs_off + 32, 4);
    let mut bytes = vec![0_u8; data_off];

    let parameter_list_off = u32_offset(bytes.len());
    push_u32(&mut bytes, 1);
    push_u16(&mut bytes, 2);
    push_u16(&mut bytes, 0);

    let first_string_data = u32_offset(bytes.len());
    let mut string_offsets = Vec::with_capacity(strings.len());
    for string in strings {
        string_offsets.push(u32_offset(bytes.len()));
        push_uleb(&mut bytes, u32::try_from(string.encode_utf16().count()).unwrap());
        bytes.extend_from_slice(string.as_bytes());
        bytes.push(0);
    }

    let mut code_offsets = Vec::with_capacity(code.len());
    let mut first_code_off = None;
    for spec in &code {
        align_bytes(&mut bytes, 4);
        let offset = u32_offset(bytes.len());
        first_code_off.get_or_insert(offset);
        code_offsets.push((spec.method_idx, offset));
        push_code(&mut bytes, spec);
    }

    let class_data_off = u32_offset(bytes.len());
    push_uleb(&mut bytes, 1);
    push_uleb(&mut bytes, 1);
    push_uleb(&mut bytes, 10);
    push_uleb(&mut bytes, 0);
    push_uleb(&mut bytes, 0);
    push_uleb(&mut bytes, 0x0009);
    push_uleb(&mut bytes, 1);
    push_uleb(&mut bytes, 0x0001);
    let mut previous = 0_u32;
    for (position, method_idx) in [0_u32, 1, 2, 3, 4, 6, 7, 8, 9, 10].into_iter().enumerate() {
        let difference = if position == 0 { method_idx } else { method_idx - previous };
        push_uleb(&mut bytes, difference);
        push_uleb(&mut bytes, 0x0009);
        let code_off = code_offsets
            .iter()
            .find_map(|(index, offset)| (*index == method_idx).then_some(*offset))
            .unwrap();
        push_uleb(&mut bytes, code_off);
        previous = method_idx;
    }

    align_bytes(&mut bytes, 4);
    let map_off = u32_offset(bytes.len());
    let mut map = vec![
        (0x0000_u16, 1_u32, 0_u32),
        (0x0001, u32::try_from(strings.len()).unwrap(), u32_offset(string_ids_off)),
        (0x0002, u32::try_from(type_descriptor_indices.len()).unwrap(), u32_offset(type_ids_off)),
        (0x0003, 3, u32_offset(proto_ids_off)),
        (0x0004, 2, u32_offset(field_ids_off)),
        (0x0005, u32::try_from(method_ids.len()).unwrap(), u32_offset(method_ids_off)),
        (0x0006, 1, u32_offset(class_defs_off)),
        (0x1001, 1, parameter_list_off),
        (0x2002, u32::try_from(strings.len()).unwrap(), first_string_data),
        (0x2001, u32::try_from(code.len()).unwrap(), first_code_off.unwrap()),
        (0x2000, 1, class_data_off),
        (0x1000, 1, map_off),
    ];
    map.sort_by_key(|item| item.2);
    push_u32(&mut bytes, u32::try_from(map.len()).unwrap());
    for (type_code, size, offset) in map {
        push_u16(&mut bytes, type_code);
        push_u16(&mut bytes, 0);
        push_u32(&mut bytes, size);
        push_u32(&mut bytes, offset);
    }

    bytes[0..8].copy_from_slice(b"dex\n035\0");
    let file_size = u32_offset(bytes.len());
    let data_size = u32::try_from(bytes.len() - data_off).unwrap();
    write_u32(&mut bytes, 32, file_size);
    write_u32(&mut bytes, 36, 0x70);
    write_u32(&mut bytes, 40, 0x1234_5678);
    write_u32(&mut bytes, 52, map_off);
    write_u32(&mut bytes, 56, u32::try_from(strings.len()).unwrap());
    write_u32(&mut bytes, 60, u32_offset(string_ids_off));
    write_u32(&mut bytes, 64, u32::try_from(type_descriptor_indices.len()).unwrap());
    write_u32(&mut bytes, 68, u32_offset(type_ids_off));
    write_u32(&mut bytes, 72, 3);
    write_u32(&mut bytes, 76, u32_offset(proto_ids_off));
    write_u32(&mut bytes, 80, 2);
    write_u32(&mut bytes, 84, u32_offset(field_ids_off));
    write_u32(&mut bytes, 88, u32::try_from(method_ids.len()).unwrap());
    write_u32(&mut bytes, 92, u32_offset(method_ids_off));
    write_u32(&mut bytes, 96, 1);
    write_u32(&mut bytes, 100, u32_offset(class_defs_off));
    write_u32(&mut bytes, 104, data_size);
    write_u32(&mut bytes, 108, u32_offset(data_off));

    for (index, offset) in string_offsets.into_iter().enumerate() {
        write_u32(&mut bytes, string_ids_off + index * 4, offset);
    }
    for (index, descriptor_idx) in type_descriptor_indices.into_iter().enumerate() {
        write_u32(&mut bytes, type_ids_off + index * 4, descriptor_idx);
    }
    write_proto(&mut bytes, proto_ids_off, 2, 2, 0);
    write_proto(&mut bytes, proto_ids_off + 12, 20, 2, parameter_list_off);
    write_proto(&mut bytes, proto_ids_off + 24, 3, 3, 0);

    write_field(&mut bytes, field_ids_off, 0, 2, 18);
    write_field(&mut bytes, field_ids_off + 8, 0, 2, 19);
    for (index, (class_idx, proto_idx, name_idx)) in method_ids.into_iter().enumerate() {
        let offset = method_ids_off + index * 8;
        write_u16(&mut bytes, offset, class_idx);
        write_u16(&mut bytes, offset + 2, proto_idx);
        write_u32(&mut bytes, offset + 4, name_idx);
    }

    write_u32(&mut bytes, class_defs_off, 0);
    write_u32(&mut bytes, class_defs_off + 4, 0x0001);
    write_u32(&mut bytes, class_defs_off + 8, 1);
    write_u32(&mut bytes, class_defs_off + 16, u32::MAX);
    write_u32(&mut bytes, class_defs_off + 24, class_data_off);
    update_digests(&mut bytes);
    bytes
}

pub(crate) fn refresh_digests(bytes: &mut [u8]) {
    update_digests(bytes);
}

fn push_code(bytes: &mut Vec<u8>, spec: &CodeSpec) {
    push_u16(bytes, spec.registers);
    push_u16(bytes, spec.ins);
    push_u16(bytes, spec.outs);
    push_u16(bytes, u16::from(spec.caught_type.is_some()));
    push_u32(bytes, 0);
    push_u32(bytes, u32::try_from(spec.instructions.len()).unwrap());
    for instruction in &spec.instructions {
        push_u16(bytes, *instruction);
    }
    if let Some(type_idx) = spec.caught_type {
        if !spec.instructions.len().is_multiple_of(2) {
            push_u16(bytes, 0);
        }
        push_u32(bytes, 0);
        push_u16(bytes, 3);
        push_u16(bytes, 1);
        push_uleb(bytes, 1);
        push_sleb(bytes, 1);
        push_uleb(bytes, type_idx);
        push_uleb(bytes, 4);
    }
}

fn write_proto(bytes: &mut [u8], offset: usize, shorty: u32, return_type: u32, params: u32) {
    write_u32(bytes, offset, shorty);
    write_u32(bytes, offset + 4, return_type);
    write_u32(bytes, offset + 8, params);
}

fn write_field(bytes: &mut [u8], offset: usize, class_idx: u16, type_idx: u16, name_idx: u32) {
    write_u16(bytes, offset, class_idx);
    write_u16(bytes, offset + 2, type_idx);
    write_u32(bytes, offset + 4, name_idx);
}

fn push_uleb(bytes: &mut Vec<u8>, mut value: u32) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        bytes.push(byte);
        if value == 0 {
            return;
        }
    }
}

fn push_sleb(bytes: &mut Vec<u8>, mut value: i32) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        let done = (value == 0 && byte & 0x40 == 0) || (value == -1 && byte & 0x40 != 0);
        bytes.push(if done { byte } else { byte | 0x80 });
        if done {
            return;
        }
    }
}

fn align_bytes(bytes: &mut Vec<u8>, alignment: usize) {
    bytes.resize(align(bytes.len(), alignment), 0);
}

const fn align(value: usize, alignment: usize) -> usize {
    (value + alignment - 1) & !(alignment - 1)
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

fn u32_offset(value: usize) -> u32 {
    u32::try_from(value).unwrap()
}
