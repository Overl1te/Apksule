//! Minimal `resources.arsc` reader for M3 string/layout resolution.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::doc_markdown,
    clippy::match_same_arms,
    clippy::must_use_candidate,
    clippy::similar_names,
    clippy::too_many_lines,
    clippy::unreadable_literal
)]

use crate::error::{CompatError, Result};

const RES_TABLE_TYPE: u16 = 0x0002;
const RES_STRING_POOL_TYPE: u16 = 0x0001;
const RES_TABLE_PACKAGE_TYPE: u16 = 0x0200;
const RES_TABLE_TYPE_TYPE: u16 = 0x0201;
const RES_TABLE_TYPE_SPEC_TYPE: u16 = 0x0202;

/// Parsed resource table subset used by inflate and `getString`.
#[derive(Debug, Clone, Default)]
pub struct ResourceTable {
    pub package_name: String,
    pub package_id: u8,
    /// type name → type index (1-based in Android, stored as in file)
    pub type_names: Vec<String>,
    pub key_names: Vec<String>,
    pub global_strings: Vec<String>,
    /// (type_index 0-based, entry_index) → value
    pub entries: Vec<ResourceEntry>,
}

#[derive(Debug, Clone)]
pub struct ResourceEntry {
    pub type_index: u16,
    pub entry_index: u16,
    pub key_index: u32,
    pub value: ResourceValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceValue {
    String(u32),
    Reference(u32),
    IntDec(i32),
    Raw(u32),
}

impl ResourceTable {
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < 8 {
            return Err(CompatError::Resource {
                path: "resources.arsc".into(),
                message: "table too small".into(),
            });
        }
        let header_type = read_u16(data, 0)?;
        let header_size = read_u16(data, 2)? as usize;
        let total_size = read_u32(data, 4)? as usize;
        if header_type != RES_TABLE_TYPE || total_size > data.len() || header_size > data.len() {
            return Err(CompatError::Resource {
                path: "resources.arsc".into(),
                message: "invalid table header".into(),
            });
        }

        let mut table = Self::default();
        let mut offset = header_size;
        // Optional global string pool immediately after header (after packageCount u32 at +8).
        if header_size >= 12 {
            // package count ignored; scan chunks
        }
        while offset + 8 <= total_size.min(data.len()) {
            let chunk_type = read_u16(data, offset)?;
            let chunk_header = read_u16(data, offset + 2)? as usize;
            let chunk_size = read_u32(data, offset + 4)? as usize;
            if chunk_size < 8 || offset + chunk_size > data.len() {
                break;
            }
            let chunk = &data[offset..offset + chunk_size];
            match chunk_type {
                RES_STRING_POOL_TYPE if table.global_strings.is_empty() => {
                    table.global_strings = parse_string_pool(chunk)?;
                }
                RES_TABLE_PACKAGE_TYPE => {
                    parse_package(chunk, &mut table)?;
                }
                _ => {}
            }
            offset += chunk_size;
            let _ = chunk_header;
        }
        Ok(table)
    }

    #[must_use]
    pub fn resource_id(&self, type_name: &str, entry_name: &str) -> Option<u32> {
        let type_index = self.type_names.iter().position(|name| name == type_name)? as u16;
        let key_index = self.key_names.iter().position(|name| name == entry_name)? as u32;
        let entry = self.entries.iter().find(|entry| {
            entry.type_index == type_index && entry.key_index == key_index
        })?;
        Some(encode_id(self.package_id, type_index + 1, entry.entry_index))
    }

    #[must_use]
    pub fn resolve_string_id(&self, id: u32) -> Option<&str> {
        let entry = self.find_entry(id)?;
        match entry.value {
            ResourceValue::String(index) => self.global_strings.get(index as usize).map(String::as_str),
            _ => None,
        }
    }

    #[must_use]
    pub fn resolve_reference(&self, id: u32) -> Option<&ResourceEntry> {
        self.find_entry(id)
    }

    #[must_use]
    pub fn layout_name(&self, id: u32) -> Option<&str> {
        let entry = self.find_entry(id)?;
        self.key_names.get(entry.key_index as usize).map(String::as_str)
    }

    /// Resolve a resource ID to an APK entry path (e.g. `res/-Q.xml` or `res/layout/main.xml`).
    #[must_use]
    pub fn resolve_resource_path(&self, id: u32) -> Option<&str> {
        let entry = self.find_entry(id)?;
        match entry.value {
            ResourceValue::String(index) => {
                let path = self.global_strings.get(index as usize)?.as_str();
                if path.starts_with("res/") { Some(path) } else { None }
            }
            _ => None,
        }
    }

    fn find_entry(&self, id: u32) -> Option<&ResourceEntry> {
        let package = ((id >> 24) & 0xff) as u8;
        let type_id = ((id >> 16) & 0xff) as u16;
        let entry_index = (id & 0xffff) as u16;
        if package != self.package_id || type_id == 0 {
            return None;
        }
        self.entries
            .iter()
            .find(|entry| entry.type_index + 1 == type_id && entry.entry_index == entry_index)
    }
}

fn encode_id(package_id: u8, type_id: u16, entry_index: u16) -> u32 {
    (u32::from(package_id) << 24) | (u32::from(type_id) << 16) | u32::from(entry_index)
}

fn parse_package(chunk: &[u8], table: &mut ResourceTable) -> Result<()> {
    if chunk.len() < 288 {
        return Err(CompatError::Resource {
            path: "resources.arsc".into(),
            message: "package chunk too small".into(),
        });
    }
    table.package_id = read_u32(chunk, 8)? as u8;
    table.package_name = read_utf16_z(&chunk[12..12 + 256]);
    let type_strings_off = read_u32(chunk, 268)? as usize;
    let key_strings_off = read_u32(chunk, 276)? as usize;

    if type_strings_off + 8 <= chunk.len() {
        table.type_names = parse_string_pool(&chunk[type_strings_off..])?;
    }
    if key_strings_off + 8 <= chunk.len() {
        table.key_names = parse_string_pool(&chunk[key_strings_off..])?;
    }

    let mut offset = read_u16(chunk, 2)? as usize;
    while offset + 8 <= chunk.len() {
        let chunk_type = read_u16(chunk, offset)?;
        let chunk_size = read_u32(chunk, offset + 4)? as usize;
        if chunk_size < 8 || offset + chunk_size > chunk.len() {
            break;
        }
        let inner = &chunk[offset..offset + chunk_size];
        match chunk_type {
            RES_TABLE_TYPE_SPEC_TYPE => {}
            RES_TABLE_TYPE_TYPE => parse_type_chunk(inner, table)?,
            RES_STRING_POOL_TYPE => {}
            _ => {}
        }
        offset += chunk_size;
    }
    Ok(())
}

fn parse_type_chunk(chunk: &[u8], table: &mut ResourceTable) -> Result<()> {
    if chunk.len() < 20 {
        return Ok(());
    }
    let type_id = u16::from(chunk[8]).saturating_sub(1); // file stores 1-based
    let entry_count = read_u32(chunk, 12)? as usize;
    let entries_start = read_u32(chunk, 16)? as usize;
    if entries_start > chunk.len() {
        return Ok(());
    }
    let offsets_start = read_u16(chunk, 2)? as usize;
    for index in 0..entry_count {
        let off_pos = offsets_start + index * 4;
        if off_pos + 4 > chunk.len() {
            break;
        }
        let entry_off = read_u32(chunk, off_pos)?;
        if entry_off == 0xffff_ffff {
            continue;
        }
        let abs = entries_start.saturating_add(entry_off as usize);
        if abs + 16 > chunk.len() {
            continue;
        }
        let _size = read_u16(chunk, abs)?;
        let flags = read_u16(chunk, abs + 2)?;
        let key = read_u32(chunk, abs + 4)?;
        if flags & 0x0001 != 0 {
            // complex — skip for M3
            continue;
        }
        let data_type = chunk[abs + 11];
        let data = read_u32(chunk, abs + 12)?;
        let value = match data_type {
            0x03 => ResourceValue::String(data),
            0x01 => ResourceValue::Reference(data),
            0x10 => ResourceValue::IntDec(data as i32),
            _ => ResourceValue::Raw(data),
        };
        table.entries.push(ResourceEntry {
            type_index: type_id,
            entry_index: index as u16,
            key_index: key,
            value,
        });
    }
    Ok(())
}

fn parse_string_pool(chunk: &[u8]) -> Result<Vec<String>> {
    if chunk.len() < 28 {
        return Ok(Vec::new());
    }
    let string_count = read_u32(chunk, 8)? as usize;
    let flags = read_u32(chunk, 16)?;
    let strings_start = read_u32(chunk, 20)? as usize;
    let utf8 = flags & (1 << 8) != 0;
    let mut strings = Vec::with_capacity(string_count);
    for index in 0..string_count {
        let offset_pos = 28 + index * 4;
        if offset_pos + 4 > chunk.len() {
            break;
        }
        let rel = read_u32(chunk, offset_pos)? as usize;
        let abs = strings_start.saturating_add(rel);
        if abs >= chunk.len() {
            strings.push(String::new());
            continue;
        }
        let value = if utf8 {
            read_utf8_string(&chunk[abs..])
        } else {
            read_utf16_string(&chunk[abs..])
        };
        strings.push(value);
    }
    Ok(strings)
}

fn read_utf8_string(data: &[u8]) -> String {
    if data.is_empty() {
        return String::new();
    }
    let mut i = 0;
    // skip char len uleb / byte len
    let (_chars, n1) = read_uleb128(data, i);
    i += n1;
    let (bytes, n2) = read_uleb128(data, i);
    i += n2;
    let end = (i + bytes as usize).min(data.len());
    String::from_utf8_lossy(&data[i..end]).into_owned()
}

fn read_utf16_string(data: &[u8]) -> String {
    if data.len() < 2 {
        return String::new();
    }
    let first = u16::from_le_bytes([data[0], data[1]]);
    let (char_count, mut i) = if first & 0x8000 != 0 {
        if data.len() < 4 {
            return String::new();
        }
        let high = u16::from_le_bytes([data[2], data[3]]);
        ((u32::from(first & 0x7fff) << 16) | u32::from(high), 4usize)
    } else {
        (u32::from(first), 2usize)
    };
    let mut units = Vec::with_capacity(char_count as usize);
    for _ in 0..char_count {
        if i + 2 > data.len() {
            break;
        }
        units.push(u16::from_le_bytes([data[i], data[i + 1]]));
        i += 2;
    }
    String::from_utf16_lossy(&units)
}

fn read_utf16_z(data: &[u8]) -> String {
    let mut units = Vec::new();
    for chunk in data.chunks_exact(2) {
        let unit = u16::from_le_bytes([chunk[0], chunk[1]]);
        if unit == 0 {
            break;
        }
        units.push(unit);
    }
    String::from_utf16_lossy(&units)
}

fn read_uleb128(data: &[u8], start: usize) -> (u32, usize) {
    let mut result = 0u32;
    let mut shift = 0;
    for (index, byte) in data[start..].iter().copied().enumerate().take(5) {
        result |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return (result, index + 1);
        }
        shift += 7;
    }
    (result, 0)
}

fn read_u16(data: &[u8], offset: usize) -> Result<u16> {
    data.get(offset..offset + 2)
        .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
        .ok_or_else(|| CompatError::Resource {
            path: "resources.arsc".into(),
            message: format!("u16 out of bounds at {offset}"),
        })
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32> {
    data.get(offset..offset + 4)
        .map(|bytes| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        .ok_or_else(|| CompatError::Resource {
            path: "resources.arsc".into(),
            message: format!("u32 out of bounds at {offset}"),
        })
}

/// Build a tiny hand-crafted `resources.arsc` for fixtures (package id 0x7f).
#[must_use]
pub fn build_minimal_arsc(package_name: &str, app_name: &str, layout_name: &str) -> Vec<u8> {
    // Simplified synthetic table understood by our parser:
    // ResTable + global string pool + package with type/key pools + one type chunk.
    let global_strings = [app_name.to_owned(), format!("res/layout/{layout_name}.xml")];
    let type_names = ["layout".to_owned(), "string".to_owned()];
    let key_names = [layout_name.to_owned(), "app_name".to_owned()];

    let mut out = Vec::new();
    // We'll assemble then patch sizes.
    let table_header_size = 12usize;
    out.extend_from_slice(&RES_TABLE_TYPE.to_le_bytes());
    out.extend_from_slice(&(table_header_size as u16).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // size patch
    out.extend_from_slice(&1u32.to_le_bytes()); // package count

    let pool = encode_string_pool(&global_strings, true);
    out.extend_from_slice(&pool);

    let mut package = Vec::new();
    let package_header_size = 288usize;
    package.extend_from_slice(&RES_TABLE_PACKAGE_TYPE.to_le_bytes());
    package.extend_from_slice(&(package_header_size as u16).to_le_bytes());
    package.extend_from_slice(&0u32.to_le_bytes()); // size patch
    package.extend_from_slice(&0x7fu32.to_le_bytes());
    let mut name_bytes = vec![0u8; 256];
    for (index, unit) in package_name.encode_utf16().enumerate() {
        if index * 2 + 1 >= 254 {
            break;
        }
        name_bytes[index * 2..index * 2 + 2].copy_from_slice(&unit.to_le_bytes());
    }
    package.extend_from_slice(&name_bytes);
    // typeStrings / lastPublicType / keyStrings / lastPublicKey / typeIdOffset
    let type_pool = encode_string_pool(&type_names, true);
    let key_pool = encode_string_pool(&key_names, true);
    let type_off = package_header_size;
    let key_off = type_off + type_pool.len();
    package.extend_from_slice(&(type_off as u32).to_le_bytes());
    package.extend_from_slice(&(type_names.len() as u32).to_le_bytes());
    package.extend_from_slice(&(key_off as u32).to_le_bytes());
    package.extend_from_slice(&(key_names.len() as u32).to_le_bytes());
    package.extend_from_slice(&0u32.to_le_bytes());
    // pad header to 288
    while package.len() < package_header_size {
        package.push(0);
    }
    package.extend_from_slice(&type_pool);
    package.extend_from_slice(&key_pool);

    // typeSpec for layout (type 1)
    let mut type_spec = Vec::new();
    type_spec.extend_from_slice(&RES_TABLE_TYPE_SPEC_TYPE.to_le_bytes());
    type_spec.extend_from_slice(&16u16.to_le_bytes());
    type_spec.extend_from_slice(&20u32.to_le_bytes());
    type_spec.push(1); // id
    type_spec.extend_from_slice(&[0, 0, 0]);
    type_spec.extend_from_slice(&1u32.to_le_bytes()); // entryCount
    type_spec.extend_from_slice(&0u32.to_le_bytes()); // flag
    package.extend_from_slice(&type_spec);

    // type chunk for layout entry 0 -> reference path string index 1 (layout file) as string value for simplicity
    // For inflate we resolve layout by key name; value can be string index of path.
    let mut type_chunk = Vec::new();
    let type_header = 20usize;
    type_chunk.extend_from_slice(&RES_TABLE_TYPE_TYPE.to_le_bytes());
    type_chunk.extend_from_slice(&(type_header as u16).to_le_bytes());
    type_chunk.extend_from_slice(&0u32.to_le_bytes()); // size
    type_chunk.push(1); // id layout
    type_chunk.extend_from_slice(&[0, 0, 0]);
    type_chunk.extend_from_slice(&1u32.to_le_bytes()); // entryCount
    type_chunk.extend_from_slice(&((type_header + 4) as u32).to_le_bytes()); // entriesStart
    // config is omitted when header=20 — offsets follow header
    type_chunk.extend_from_slice(&0u32.to_le_bytes()); // entry 0 offset 0
    // entry
    type_chunk.extend_from_slice(&8u16.to_le_bytes()); // size
    type_chunk.extend_from_slice(&0u16.to_le_bytes()); // flags
    type_chunk.extend_from_slice(&0u32.to_le_bytes()); // key index 0 = layout_name
    type_chunk.extend_from_slice(&8u16.to_le_bytes()); // value size
    type_chunk.push(0); // res0
    type_chunk.push(0x03); // TYPE_STRING
    type_chunk.extend_from_slice(&1u32.to_le_bytes()); // string index 1
    let type_size = type_chunk.len() as u32;
    type_chunk[4..8].copy_from_slice(&type_size.to_le_bytes());
    package.extend_from_slice(&type_chunk);

    // string typeSpec + entry for app_name
    let mut string_spec = Vec::new();
    string_spec.extend_from_slice(&RES_TABLE_TYPE_SPEC_TYPE.to_le_bytes());
    string_spec.extend_from_slice(&16u16.to_le_bytes());
    string_spec.extend_from_slice(&20u32.to_le_bytes());
    string_spec.push(2);
    string_spec.extend_from_slice(&[0, 0, 0]);
    string_spec.extend_from_slice(&1u32.to_le_bytes());
    string_spec.extend_from_slice(&0u32.to_le_bytes());
    package.extend_from_slice(&string_spec);

    let mut string_type = Vec::new();
    string_type.extend_from_slice(&RES_TABLE_TYPE_TYPE.to_le_bytes());
    string_type.extend_from_slice(&20u16.to_le_bytes());
    string_type.extend_from_slice(&0u32.to_le_bytes());
    string_type.push(2);
    string_type.extend_from_slice(&[0, 0, 0]);
    string_type.extend_from_slice(&1u32.to_le_bytes());
    string_type.extend_from_slice(&24u32.to_le_bytes());
    string_type.extend_from_slice(&0u32.to_le_bytes());
    string_type.extend_from_slice(&8u16.to_le_bytes());
    string_type.extend_from_slice(&0u16.to_le_bytes());
    string_type.extend_from_slice(&1u32.to_le_bytes()); // key app_name
    string_type.extend_from_slice(&8u16.to_le_bytes());
    string_type.push(0);
    string_type.push(0x03);
    string_type.extend_from_slice(&0u32.to_le_bytes()); // string 0 app_name
    let st_size = string_type.len() as u32;
    string_type[4..8].copy_from_slice(&st_size.to_le_bytes());
    package.extend_from_slice(&string_type);

    let package_size = package.len() as u32;
    package[4..8].copy_from_slice(&package_size.to_le_bytes());
    out.extend_from_slice(&package);

    let total = out.len() as u32;
    out[4..8].copy_from_slice(&total.to_le_bytes());
    out
}

fn encode_string_pool(strings: &[String], utf8: bool) -> Vec<u8> {
    let mut offsets = Vec::new();
    let mut blob = Vec::new();
    for string in strings {
        offsets.push(blob.len() as u32);
        if utf8 {
            let bytes = string.as_bytes();
            write_uleb128(&mut blob, string.chars().count() as u32);
            write_uleb128(&mut blob, bytes.len() as u32);
            blob.extend_from_slice(bytes);
            blob.push(0);
        } else {
            let units: Vec<u16> = string.encode_utf16().collect();
            blob.extend_from_slice(&(units.len() as u16).to_le_bytes());
            for unit in units {
                blob.extend_from_slice(&unit.to_le_bytes());
            }
            blob.extend_from_slice(&0u16.to_le_bytes());
        }
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
    out.extend_from_slice(&0u32.to_le_bytes()); // style count
    out.extend_from_slice(&(u32::from(utf8) << 8).to_le_bytes());
    out.extend_from_slice(&strings_start.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // styles start
    for offset in offsets {
        out.extend_from_slice(&offset.to_le_bytes());
    }
    out.extend_from_slice(&blob);
    out
}

fn write_uleb128(out: &mut Vec<u8>, mut value: u32) {
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

    #[test]
    fn roundtrips_minimal_arsc() {
        let bytes = build_minimal_arsc("dev.apksule.m3", "M3 Demo", "main");
        let table = ResourceTable::parse(&bytes).expect("parse");
        assert_eq!(table.package_id, 0x7f);
        assert!(table.resource_id("layout", "main").is_some());
        assert_eq!(table.resolve_string_id(table.resource_id("string", "app_name").unwrap()), Some("M3 Demo"));
    }
}
