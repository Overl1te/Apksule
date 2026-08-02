//! Binary Android XML layout inflater (M3 subset).

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::doc_markdown,
    clippy::must_use_candidate,
    clippy::similar_names,
    clippy::too_many_lines,
    clippy::unreadable_literal
)]

use crate::arsc::ResourceTable;
use crate::error::{CompatError, Result};
use crate::ui_host::UiHost;
use crate::view::{LayoutParams, Orientation, ViewId, ViewKind};

const RES_XML_TYPE: u16 = 0x0003;
const RES_STRING_POOL_TYPE: u16 = 0x0001;
const RES_XML_START_ELEMENT_TYPE: u16 = 0x0102;
const RES_XML_END_ELEMENT_TYPE: u16 = 0x0103;
const RES_XML_RESOURCE_MAP_TYPE: u16 = 0x0180;

/// Inflate a compiled layout XML into the host, returning the root view id.
pub fn inflate_layout(
    host: &UiHost,
    table: &ResourceTable,
    layout_id: u32,
    axml: &[u8],
) -> Result<ViewId> {
    let _ = table.layout_name(layout_id);
    inflate_axml(host, axml)
}

pub fn inflate_axml(host: &UiHost, data: &[u8]) -> Result<ViewId> {
    if data.len() < 8 {
        return Err(CompatError::Resource {
            path: "layout".into(),
            message: "AXML too small".into(),
        });
    }
    let file_type = read_u16(data, 0)?;
    if file_type != RES_XML_TYPE {
        return Err(CompatError::Resource {
            path: "layout".into(),
            message: format!("unexpected AXML type {file_type:#x}"),
        });
    }

    let mut strings = Vec::new();
    let mut resource_map: Vec<u32> = Vec::new();
    let mut stack: Vec<ViewId> = Vec::new();
    let mut root: Option<ViewId> = None;

    let total = read_u32(data, 4)? as usize;
    let mut offset = read_u16(data, 2)? as usize;
    while offset + 8 <= total.min(data.len()) {
        let chunk_type = read_u16(data, offset)?;
        let header_size = read_u16(data, offset + 2)? as usize;
        let chunk_size = read_u32(data, offset + 4)? as usize;
        if chunk_size < 8 || offset + chunk_size > data.len() {
            break;
        }
        let chunk = &data[offset..offset + chunk_size];
        match chunk_type {
            RES_STRING_POOL_TYPE => strings = parse_string_pool(chunk),
            RES_XML_RESOURCE_MAP_TYPE => {
                let mut pos = header_size;
                while pos + 4 <= chunk.len() {
                    resource_map.push(u32::from_le_bytes([
                        chunk[pos],
                        chunk[pos + 1],
                        chunk[pos + 2],
                        chunk[pos + 3],
                    ]));
                    pos += 4;
                }
            }
            RES_XML_START_ELEMENT_TYPE => {
                // ResXMLTree_attrExt begins at header_size (line/comment are inside the header).
                let name_idx = read_u32(chunk, header_size + 4)? as usize;
                let name = strings.get(name_idx).cloned().unwrap_or_default();
                let attr_start = read_u16(chunk, header_size + 8)? as usize;
                let attr_size = {
                    let size = read_u16(chunk, header_size + 10)? as usize;
                    if size == 0 { 20 } else { size }
                };
                let attr_count = read_u16(chunk, header_size + 12)? as usize;
                let attrs_off = header_size + attr_start;

                let mut text = String::new();
                let mut android_id = 0i32;
                let mut orientation = Orientation::Vertical;
                for index in 0..attr_count {
                    let base = attrs_off + index * attr_size;
                    if base + 20 > chunk.len() {
                        break;
                    }
                    let attr_name_idx = read_u32(chunk, base + 4)? as usize;
                    let raw_value = read_u32(chunk, base + 8)? as usize;
                    let data_value = read_u32(chunk, base + 16)?;
                    let attr_name = strings.get(attr_name_idx).cloned().unwrap_or_default();
                    let attr_res = resource_map.get(attr_name_idx).copied();
                    let is_text = attr_name == "text" || attr_res == Some(0x0101_004f);
                    let is_id = attr_name == "id" || attr_res == Some(0x0101_00d0);
                    let is_orientation =
                        attr_name == "orientation" || attr_res == Some(0x0101_00c4);
                    if is_text {
                        text = strings
                            .get(raw_value)
                            .or_else(|| strings.get(data_value as usize))
                            .cloned()
                            .unwrap_or_default();
                    } else if is_id {
                        android_id = data_value as i32;
                    } else if is_orientation {
                        orientation = if data_value == 0 {
                            Orientation::Horizontal
                        } else {
                            Orientation::Vertical
                        };
                    }
                }

                let is_group = name.contains("Layout")
                    || name.contains("ViewGroup")
                    || name.contains("RecyclerView")
                    || name.contains("Toolbar")
                    || name.contains("AppBar");
                let kind = view_kind_for_name(&name, text, orientation);
                let id = host.create_view(kind);
                host.set_layout_params(
                    id,
                    if is_group {
                        LayoutParams::match_parent()
                    } else {
                        LayoutParams::wrap_content()
                    },
                );
                if android_id != 0 {
                    host.set_android_id(id, android_id);
                }
                if let Some(parent) = stack.last().copied() {
                    host.add_child(parent, id);
                } else {
                    root = Some(id);
                }
                stack.push(id);
            }
            RES_XML_END_ELEMENT_TYPE => {
                let _ = stack.pop();
            }
            _ => {}
        }
        offset += chunk_size;
    }

    root.ok_or_else(|| CompatError::Resource {
        path: "layout".into(),
        message: "AXML contained no elements".into(),
    })
}

fn view_kind_for_name(name: &str, text: String, orientation: Orientation) -> ViewKind {
    let short = name.rsplit('.').next().unwrap_or(name);
    if short.contains("RecyclerView") || name.contains("RecyclerView") {
        ViewKind::RecyclerView { children: Vec::new() }
    } else if short.contains("Button")
        || name.contains("MaterialButton")
        || name.contains("Button")
    {
        ViewKind::Button { text }
    } else if short.contains("EditText")
        || name.contains("TextInputEditText")
        || name.contains("EditText")
    {
        ViewKind::EditText { text }
    } else if short.contains("TextView") || name.contains("TextView") {
        ViewKind::TextView { text }
    } else if short.contains("Toolbar")
        || short.contains("AppBar")
        || name.contains("Toolbar")
        || name.contains("AppBarLayout")
        || name.contains("CoordinatorLayout")
        || name.contains("Frame")
        || short == "ViewGroup"
    {
        ViewKind::FrameLayout { children: Vec::new() }
    } else if short.contains("Linear")
        || name.contains("LinearLayout")
        || name.contains("Layout")
        || name.contains("Constraint")
    {
        ViewKind::LinearLayout { orientation, children: Vec::new() }
    } else {
        ViewKind::View
    }
}

fn parse_string_pool(chunk: &[u8]) -> Vec<String> {
    if chunk.len() < 28 {
        return Vec::new();
    }
    let string_count = u32::from_le_bytes(chunk[8..12].try_into().unwrap_or([0; 4])) as usize;
    let flags = u32::from_le_bytes(chunk[16..20].try_into().unwrap_or([0; 4]));
    let strings_start = u32::from_le_bytes(chunk[20..24].try_into().unwrap_or([0; 4])) as usize;
    let utf8 = flags & (1 << 8) != 0;
    let mut strings = Vec::with_capacity(string_count);
    for index in 0..string_count {
        let offset_pos = 28 + index * 4;
        if offset_pos + 4 > chunk.len() {
            break;
        }
        let rel = u32::from_le_bytes(chunk[offset_pos..offset_pos + 4].try_into().unwrap_or([0; 4]))
            as usize;
        let abs = strings_start.saturating_add(rel);
        if abs >= chunk.len() {
            strings.push(String::new());
            continue;
        }
        strings.push(if utf8 {
            read_utf8(&chunk[abs..])
        } else {
            read_utf16(&chunk[abs..])
        });
    }
    strings
}

fn read_utf8(data: &[u8]) -> String {
    let mut i = 0usize;
    let (_, n1) = read_uleb(data, i);
    i += n1;
    let (bytes, n2) = read_uleb(data, i);
    i += n2;
    let end = (i + bytes as usize).min(data.len());
    String::from_utf8_lossy(&data[i..end]).into_owned()
}

fn read_utf16(data: &[u8]) -> String {
    if data.len() < 2 {
        return String::new();
    }
    let first = u16::from_le_bytes([data[0], data[1]]);
    let (count, mut i) = if first & 0x8000 != 0 {
        if data.len() < 4 {
            return String::new();
        }
        let high = u16::from_le_bytes([data[2], data[3]]);
        ((u32::from(first & 0x7fff) << 16) | u32::from(high), 4usize)
    } else {
        (u32::from(first), 2usize)
    };
    let mut units = Vec::new();
    for _ in 0..count {
        if i + 2 > data.len() {
            break;
        }
        units.push(u16::from_le_bytes([data[i], data[i + 1]]));
        i += 2;
    }
    String::from_utf16_lossy(&units)
}

fn read_uleb(data: &[u8], start: usize) -> (u32, usize) {
    let mut result = 0u32;
    let mut shift = 0;
    for (index, byte) in data.get(start..).unwrap_or(&[]).iter().copied().enumerate().take(5) {
        result |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return (result, index + 1);
        }
        shift += 7;
    }
    (0, 0)
}

fn read_u16(data: &[u8], offset: usize) -> Result<u16> {
    data.get(offset..offset + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| CompatError::Resource {
            path: "layout".into(),
            message: format!("u16 OOB {offset}"),
        })
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32> {
    data.get(offset..offset + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| CompatError::Resource {
            path: "layout".into(),
            message: format!("u32 OOB {offset}"),
        })
}

/// Build a minimal binary layout XML for fixtures.
#[must_use]
pub fn build_minimal_layout_axml(title: &str, button: &str) -> Vec<u8> {
    let strings = [
        "LinearLayout".to_owned(),
        "TextView".to_owned(),
        "EditText".to_owned(),
        "Button".to_owned(),
        "text".to_owned(),
        "orientation".to_owned(),
        title.to_owned(),
        String::new(),
        button.to_owned(),
    ];
    let pool = encode_pool(&strings);
    let mut body = Vec::new();

    let mut res_map = Vec::new();
    res_map.extend_from_slice(&RES_XML_RESOURCE_MAP_TYPE.to_le_bytes());
    res_map.extend_from_slice(&8u16.to_le_bytes());
    res_map.extend_from_slice(&16u32.to_le_bytes());
    res_map.extend_from_slice(&0x0101_004fu32.to_le_bytes());
    res_map.extend_from_slice(&0x0101_00c4u32.to_le_bytes());
    let res_size = res_map.len() as u32;
    res_map[4..8].copy_from_slice(&res_size.to_le_bytes());
    body.extend_from_slice(&res_map);

    // LinearLayout orientation=1 (vertical) via attr name index 5, data=1
    body.extend_from_slice(&start_element(0, &[(5, 0xffff_ffff, 1)]));
    body.extend_from_slice(&start_element(1, &[(4, 6, 6)]));
    body.extend_from_slice(&end_element(1));
    body.extend_from_slice(&start_element(2, &[(4, 7, 7)]));
    body.extend_from_slice(&end_element(2));
    body.extend_from_slice(&start_element(3, &[(4, 8, 8)]));
    body.extend_from_slice(&end_element(3));
    body.extend_from_slice(&end_element(0));

    let header_size = 8u16;
    let mut out = Vec::new();
    out.extend_from_slice(&RES_XML_TYPE.to_le_bytes());
    out.extend_from_slice(&header_size.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&pool);
    out.extend_from_slice(&body);
    let size = out.len() as u32;
    out[4..8].copy_from_slice(&size.to_le_bytes());
    out
}

fn start_element(name_idx: u32, attrs: &[(u32, u32, u32)]) -> Vec<u8> {
    let header = 16u16;
    let attr_count = attrs.len() as u16;
    let attr_start = 20u16;
    let attr_size = 20u16;
    let mut chunk = Vec::new();
    chunk.extend_from_slice(&RES_XML_START_ELEMENT_TYPE.to_le_bytes());
    chunk.extend_from_slice(&header.to_le_bytes());
    chunk.extend_from_slice(&0u32.to_le_bytes());
    chunk.extend_from_slice(&1u32.to_le_bytes());
    chunk.extend_from_slice(&0xffff_ffffu32.to_le_bytes());
    chunk.extend_from_slice(&0xffff_ffffu32.to_le_bytes());
    chunk.extend_from_slice(&name_idx.to_le_bytes());
    chunk.extend_from_slice(&attr_start.to_le_bytes());
    chunk.extend_from_slice(&attr_size.to_le_bytes());
    chunk.extend_from_slice(&attr_count.to_le_bytes());
    chunk.extend_from_slice(&0u16.to_le_bytes());
    chunk.extend_from_slice(&0u16.to_le_bytes());
    chunk.extend_from_slice(&0u16.to_le_bytes());
    for (name, raw, data) in attrs {
        chunk.extend_from_slice(&0xffff_ffffu32.to_le_bytes());
        chunk.extend_from_slice(&name.to_le_bytes());
        chunk.extend_from_slice(&raw.to_le_bytes());
        chunk.extend_from_slice(&8u16.to_le_bytes());
        chunk.push(0);
        let data_type = if *raw == 0xffff_ffff { 0x10 } else { 0x03 };
        chunk.push(data_type);
        chunk.extend_from_slice(&data.to_le_bytes());
    }
    let size = chunk.len() as u32;
    chunk[4..8].copy_from_slice(&size.to_le_bytes());
    chunk
}

fn end_element(name_idx: u32) -> Vec<u8> {
    let mut chunk = Vec::new();
    chunk.extend_from_slice(&RES_XML_END_ELEMENT_TYPE.to_le_bytes());
    chunk.extend_from_slice(&16u16.to_le_bytes());
    chunk.extend_from_slice(&24u32.to_le_bytes());
    chunk.extend_from_slice(&1u32.to_le_bytes());
    chunk.extend_from_slice(&0xffff_ffffu32.to_le_bytes());
    chunk.extend_from_slice(&0xffff_ffffu32.to_le_bytes());
    chunk.extend_from_slice(&name_idx.to_le_bytes());
    chunk
}

fn encode_pool(strings: &[String]) -> Vec<u8> {
    let mut offsets = Vec::new();
    let mut blob = Vec::new();
    for string in strings {
        offsets.push(blob.len() as u32);
        let bytes = string.as_bytes();
        write_uleb(&mut blob, string.chars().count() as u32);
        write_uleb(&mut blob, bytes.len() as u32);
        blob.extend_from_slice(bytes);
        blob.push(0);
    }
    while blob.len() % 4 != 0 {
        blob.push(0);
    }
    let header_size = 28u16;
    let strings_start = u32::from(header_size) + (offsets.len() as u32) * 4;
    let mut out = Vec::new();
    out.extend_from_slice(&RES_STRING_POOL_TYPE.to_le_bytes());
    out.extend_from_slice(&header_size.to_le_bytes());
    let size = strings_start as usize + blob.len();
    out.extend_from_slice(&(size as u32).to_le_bytes());
    out.extend_from_slice(&(strings.len() as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(1u32 << 8).to_le_bytes());
    out.extend_from_slice(&strings_start.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for offset in offsets {
        out.extend_from_slice(&offset.to_le_bytes());
    }
    out.extend_from_slice(&blob);
    out
}

fn write_uleb(out: &mut Vec<u8>, mut value: u32) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
            out.push(byte);
        } else {
            out.push(byte);
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_host::UiHost;

    #[test]
    fn inflates_minimal_layout() {
        let host = UiHost::new();
        host.set_surface_size(400, 400);
        let axml = build_minimal_layout_axml("Hello M3", "Save");
        let root = inflate_axml(&host, &axml).expect("inflate");
        host.set_content_view(root);
        let snap = host.snapshot();
        assert!(snap.len() >= 3);
        assert!(snap.iter().any(|n| n.kind.text() == Some("Hello M3")));
        assert!(snap.iter().any(|n| n.kind.text() == Some("Save")));
    }
}
