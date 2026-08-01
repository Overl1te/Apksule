use std::collections::HashMap;
use std::fmt;

use thiserror::Error;

const HEADER_SIZE: usize = 0x70;
const ENDIAN_CONSTANT: u32 = 0x1234_5678;
const REVERSE_ENDIAN_CONSTANT: u32 = 0x7856_3412;
const NO_INDEX: u32 = u32::MAX;

/// Ошибка проверки или разбора DEX-файла.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DexError {
    #[error("DEX file is shorter than its header")]
    TooShort,
    #[error("invalid DEX magic")]
    InvalidMagic,
    #[error("unsupported DEX version {0}")]
    UnsupportedVersion(String),
    #[error("reverse-endian DEX files are not supported")]
    ReverseEndian,
    #[error("invalid DEX header: {0}")]
    InvalidHeader(String),
    #[error("DEX checksum mismatch: expected {expected:#010x}, computed {actual:#010x}")]
    ChecksumMismatch { expected: u32, actual: u32 },
    #[error("DEX SHA-1 signature mismatch")]
    SignatureMismatch,
    #[error("{section} range at {offset:#x} with size {size:#x} is out of bounds")]
    Bounds { section: &'static str, offset: usize, size: usize },
    #[error("invalid LEB128 value at {offset:#x}")]
    InvalidLeb128 { offset: usize },
    #[error("invalid modified UTF-8 string at {offset:#x}: {reason}")]
    InvalidString { offset: usize, reason: String },
    #[error("{kind} index {index} is outside table of length {limit}")]
    InvalidIndex { kind: &'static str, index: u32, limit: usize },
    #[error("malformed {section}: {reason}")]
    Malformed { section: &'static str, reason: String },
}

/// Проверенные поля заголовка DEX.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DexHeader {
    pub magic: [u8; 8],
    pub checksum: u32,
    pub signature: [u8; 20],
    pub file_size: u32,
    pub header_size: u32,
    pub endian_tag: u32,
    pub link_size: u32,
    pub link_off: u32,
    pub map_off: u32,
    pub string_ids_size: u32,
    pub string_ids_off: u32,
    pub type_ids_size: u32,
    pub type_ids_off: u32,
    pub proto_ids_size: u32,
    pub proto_ids_off: u32,
    pub field_ids_size: u32,
    pub field_ids_off: u32,
    pub method_ids_size: u32,
    pub method_ids_off: u32,
    pub class_defs_size: u32,
    pub class_defs_off: u32,
    pub data_size: u32,
    pub data_off: u32,
}

/// Элемент таблицы `map_list`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapItem {
    pub type_code: u16,
    pub size: u32,
    pub offset: u32,
}

/// Ссылка типа на строку-дескриптор.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeId {
    pub descriptor_idx: u32,
}

/// Описание прототипа метода.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtoId {
    pub shorty_idx: u32,
    pub return_type_idx: u32,
    pub parameters_off: u32,
    pub parameters: Vec<u32>,
}

/// Идентификатор поля.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldId {
    pub class_idx: u16,
    pub type_idx: u16,
    pub name_idx: u32,
}

/// Идентификатор метода.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MethodId {
    pub class_idx: u16,
    pub proto_idx: u16,
    pub name_idx: u32,
}

/// Поле из `class_data_item`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncodedField {
    pub field_idx: u32,
    pub access_flags: u32,
}

/// Метод из `class_data_item`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedMethod {
    pub method_idx: u32,
    pub access_flags: u32,
    pub code_off: u32,
}

/// Обработчик исключений в `code_item`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatchHandler {
    pub offset: u32,
    pub catches: Vec<(u32, u32)>,
    pub catch_all_addr: Option<u32>,
}

/// Защищённый диапазон инструкций.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TryItem {
    pub start_addr: u32,
    pub instruction_count: u16,
    pub handler_off: u16,
}

/// Проверенное тело DEX-метода.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeItem {
    pub offset: u32,
    pub registers_size: u16,
    pub ins_size: u16,
    pub outs_size: u16,
    pub debug_info_off: u32,
    pub instructions: Vec<u16>,
    pub tries: Vec<TryItem>,
    pub handlers: Vec<CatchHandler>,
}

/// Определение класса и его закодированные члены.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassDef {
    pub class_idx: u32,
    pub access_flags: u32,
    pub superclass_idx: Option<u32>,
    pub interfaces_off: u32,
    pub interfaces: Vec<u32>,
    pub source_file_idx: Option<u32>,
    pub annotations_off: u32,
    pub class_data_off: u32,
    pub static_values_off: u32,
    pub static_fields: Vec<EncodedField>,
    pub instance_fields: Vec<EncodedField>,
    pub direct_methods: Vec<EncodedMethod>,
    pub virtual_methods: Vec<EncodedMethod>,
}

impl ClassDef {
    pub fn methods(&self) -> impl Iterator<Item = &EncodedMethod> {
        self.direct_methods.iter().chain(self.virtual_methods.iter())
    }
}

/// Найденный метод вместе с записью `class_data`, если она существует.
#[derive(Debug, Clone, Copy)]
pub struct MethodHandle<'a> {
    pub index: u32,
    pub id: &'a MethodId,
    pub encoded: Option<&'a EncodedMethod>,
}

/// Полностью разрешённое имя метода.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedMethod {
    pub index: u32,
    pub class_descriptor: String,
    pub name: String,
    pub prototype: String,
}

impl fmt::Display for ResolvedMethod {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}->{}{}", self.class_descriptor, self.name, self.prototype)
    }
}

/// Полностью разрешённое имя поля.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedField {
    pub index: u32,
    pub class_descriptor: String,
    pub name: String,
    pub type_descriptor: String,
}

impl fmt::Display for ResolvedField {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}->{}:{}", self.class_descriptor, self.name, self.type_descriptor)
    }
}

/// Разобранный DEX-файл, сохраняющий исходные байты.
#[derive(Debug, Clone)]
pub struct DexFile {
    bytes: Vec<u8>,
    header: DexHeader,
    map: Vec<MapItem>,
    strings: Vec<String>,
    types: Vec<TypeId>,
    protos: Vec<ProtoId>,
    fields: Vec<FieldId>,
    methods: Vec<MethodId>,
    classes: Vec<ClassDef>,
    method_code: HashMap<u32, CodeItem>,
}

impl DexFile {
    /// Разбирает и полностью проверяет основные структуры DEX.
    pub fn parse(bytes: Vec<u8>) -> Result<Self, DexError> {
        let reader = Reader::new(&bytes);
        let header = parse_header(&reader)?;
        validate_digest(&bytes, &header)?;
        validate_header_ranges(&reader, &header)?;
        let map = parse_map(&reader, &header)?;
        let strings = parse_strings(&reader, &header)?;
        let types = parse_types(&reader, &header, strings.len())?;
        let protos = parse_protos(&reader, &header, strings.len(), types.len())?;
        let fields = parse_fields(&reader, &header, strings.len(), types.len())?;
        let methods = parse_methods(&reader, &header, strings.len(), types.len(), protos.len())?;
        let (classes, method_code) =
            parse_classes(&reader, &header, types.len(), fields.len(), methods.len())?;

        Ok(Self {
            bytes,
            header,
            map,
            strings,
            types,
            protos,
            fields,
            methods,
            classes,
            method_code,
        })
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub const fn header(&self) -> &DexHeader {
        &self.header
    }

    #[must_use]
    pub fn map(&self) -> &[MapItem] {
        &self.map
    }

    #[must_use]
    pub fn strings(&self) -> &[String] {
        &self.strings
    }

    #[must_use]
    pub fn types(&self) -> &[TypeId] {
        &self.types
    }

    #[must_use]
    pub fn protos(&self) -> &[ProtoId] {
        &self.protos
    }

    #[must_use]
    pub fn fields(&self) -> &[FieldId] {
        &self.fields
    }

    #[must_use]
    pub fn methods(&self) -> &[MethodId] {
        &self.methods
    }

    #[must_use]
    pub fn classes(&self) -> &[ClassDef] {
        &self.classes
    }

    pub fn string(&self, index: u32) -> Result<&str, DexError> {
        get_index(&self.strings, index, "string").map(String::as_str)
    }

    pub fn type_descriptor(&self, index: u32) -> Result<&str, DexError> {
        let type_id = get_index(&self.types, index, "type")?;
        self.string(type_id.descriptor_idx)
    }

    pub fn prototype_descriptor(&self, index: u32) -> Result<String, DexError> {
        let proto = get_index(&self.protos, index, "prototype")?;
        let mut descriptor = String::from("(");
        for parameter in &proto.parameters {
            descriptor.push_str(self.type_descriptor(*parameter)?);
        }
        descriptor.push(')');
        descriptor.push_str(self.type_descriptor(proto.return_type_idx)?);
        Ok(descriptor)
    }

    #[must_use]
    pub fn find_class(&self, descriptor: &str) -> Option<&ClassDef> {
        self.classes.iter().find(|class| {
            self.type_descriptor(class.class_idx).is_ok_and(|candidate| candidate == descriptor)
        })
    }

    #[must_use]
    pub fn class_by_type(&self, type_idx: u32) -> Option<&ClassDef> {
        self.classes.iter().find(|class| class.class_idx == type_idx)
    }

    #[must_use]
    pub fn find_method(
        &self,
        class_descriptor: &str,
        name: &str,
        prototype: Option<&str>,
    ) -> Option<MethodHandle<'_>> {
        self.methods.iter().enumerate().find_map(|(index, method)| {
            let class = self.type_descriptor(u32::from(method.class_idx)).ok()?;
            let method_name = self.string(method.name_idx).ok()?;
            if class != class_descriptor || method_name != name {
                return None;
            }
            if prototype.is_some_and(|expected| {
                self.prototype_descriptor(u32::from(method.proto_idx))
                    .map_or(true, |actual| actual != expected)
            }) {
                return None;
            }
            let index = u32::try_from(index).ok()?;
            Some(MethodHandle { index, id: method, encoded: self.encoded_method(index) })
        })
    }

    pub fn resolve_method(&self, index: u32) -> Result<ResolvedMethod, DexError> {
        let method = get_index(&self.methods, index, "method")?;
        Ok(ResolvedMethod {
            index,
            class_descriptor: self.type_descriptor(u32::from(method.class_idx))?.to_owned(),
            name: self.string(method.name_idx)?.to_owned(),
            prototype: self.prototype_descriptor(u32::from(method.proto_idx))?,
        })
    }

    pub fn resolve_field(&self, index: u32) -> Result<ResolvedField, DexError> {
        let field = get_index(&self.fields, index, "field")?;
        Ok(ResolvedField {
            index,
            class_descriptor: self.type_descriptor(u32::from(field.class_idx))?.to_owned(),
            name: self.string(field.name_idx)?.to_owned(),
            type_descriptor: self.type_descriptor(u32::from(field.type_idx))?.to_owned(),
        })
    }

    #[must_use]
    pub fn encoded_method(&self, index: u32) -> Option<&EncodedMethod> {
        self.classes.iter().flat_map(ClassDef::methods).find(|method| method.method_idx == index)
    }

    #[must_use]
    pub fn method_code(&self, index: u32) -> Option<&CodeItem> {
        self.method_code.get(&index)
    }
}

#[derive(Clone, Copy)]
struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    fn range(
        &self,
        offset: usize,
        size: usize,
        section: &'static str,
    ) -> Result<&'a [u8], DexError> {
        let end = offset.checked_add(size).ok_or(DexError::Bounds { section, offset, size })?;
        self.bytes.get(offset..end).ok_or(DexError::Bounds { section, offset, size })
    }

    fn u8(&self, offset: usize, section: &'static str) -> Result<u8, DexError> {
        self.range(offset, 1, section).map(|value| value[0])
    }

    fn u16(&self, offset: usize, section: &'static str) -> Result<u16, DexError> {
        let bytes = self.range(offset, 2, section)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&self, offset: usize, section: &'static str) -> Result<u32, DexError> {
        let bytes = self.range(offset, 4, section)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn uleb128(&self, cursor: &mut usize) -> Result<u32, DexError> {
        let start = *cursor;
        let mut value = 0_u32;
        for shift in (0..=28).step_by(7) {
            let byte = self.u8(*cursor, "ULEB128")?;
            *cursor = cursor.checked_add(1).ok_or(DexError::InvalidLeb128 { offset: start })?;
            let payload = u32::from(byte & 0x7f);
            if shift == 28 && payload > 0x0f {
                return Err(DexError::InvalidLeb128 { offset: start });
            }
            value |= payload << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(DexError::InvalidLeb128 { offset: start })
    }

    fn sleb128(&self, cursor: &mut usize) -> Result<i32, DexError> {
        let start = *cursor;
        let mut value = 0_i32;
        let mut shift = 0_u32;
        for index in 0..5 {
            let byte = self.u8(*cursor, "SLEB128")?;
            *cursor = cursor.checked_add(1).ok_or(DexError::InvalidLeb128 { offset: start })?;
            let payload = i32::from(byte & 0x7f);
            value |= payload << shift;
            shift += 7;
            if byte & 0x80 == 0 {
                if shift < 32 && byte & 0x40 != 0 {
                    value |= !0_i32 << shift;
                }
                if index == 4 && !(-8..=7).contains(&payload) {
                    return Err(DexError::InvalidLeb128 { offset: start });
                }
                return Ok(value);
            }
        }
        Err(DexError::InvalidLeb128 { offset: start })
    }
}

fn parse_header(reader: &Reader<'_>) -> Result<DexHeader, DexError> {
    if reader.bytes.len() < HEADER_SIZE {
        return Err(DexError::TooShort);
    }
    let mut magic = [0_u8; 8];
    magic.copy_from_slice(reader.range(0, 8, "magic")?);
    if &magic[..4] != b"dex\n" || magic[7] != 0 {
        return Err(DexError::InvalidMagic);
    }
    let version = std::str::from_utf8(&magic[4..7]).map_err(|_| DexError::InvalidMagic)?;
    let numeric_version = version.parse::<u16>().map_err(|_| DexError::InvalidMagic)?;
    if !(35..=41).contains(&numeric_version) {
        return Err(DexError::UnsupportedVersion(version.to_owned()));
    }
    let mut signature = [0_u8; 20];
    signature.copy_from_slice(reader.range(12, 20, "signature")?);
    let header = DexHeader {
        magic,
        checksum: reader.u32(8, "header")?,
        signature,
        file_size: reader.u32(32, "header")?,
        header_size: reader.u32(36, "header")?,
        endian_tag: reader.u32(40, "header")?,
        link_size: reader.u32(44, "header")?,
        link_off: reader.u32(48, "header")?,
        map_off: reader.u32(52, "header")?,
        string_ids_size: reader.u32(56, "header")?,
        string_ids_off: reader.u32(60, "header")?,
        type_ids_size: reader.u32(64, "header")?,
        type_ids_off: reader.u32(68, "header")?,
        proto_ids_size: reader.u32(72, "header")?,
        proto_ids_off: reader.u32(76, "header")?,
        field_ids_size: reader.u32(80, "header")?,
        field_ids_off: reader.u32(84, "header")?,
        method_ids_size: reader.u32(88, "header")?,
        method_ids_off: reader.u32(92, "header")?,
        class_defs_size: reader.u32(96, "header")?,
        class_defs_off: reader.u32(100, "header")?,
        data_size: reader.u32(104, "header")?,
        data_off: reader.u32(108, "header")?,
    };
    if header.header_size != 0x70 {
        return Err(DexError::InvalidHeader(format!(
            "header_size is {:#x}, expected {HEADER_SIZE:#x}",
            header.header_size
        )));
    }
    let actual_size = u32::try_from(reader.bytes.len())
        .map_err(|_| DexError::InvalidHeader("file exceeds the DEX size limit".to_owned()))?;
    if header.file_size != actual_size {
        return Err(DexError::InvalidHeader(format!(
            "file_size is {}, actual length is {actual_size}",
            header.file_size
        )));
    }
    match header.endian_tag {
        ENDIAN_CONSTANT => {}
        REVERSE_ENDIAN_CONSTANT => return Err(DexError::ReverseEndian),
        value => {
            return Err(DexError::InvalidHeader(format!("unknown endian tag {value:#010x}")));
        }
    }
    Ok(header)
}

fn validate_digest(bytes: &[u8], header: &DexHeader) -> Result<(), DexError> {
    let actual_checksum = adler32(&bytes[12..]);
    if actual_checksum != header.checksum {
        return Err(DexError::ChecksumMismatch {
            expected: header.checksum,
            actual: actual_checksum,
        });
    }
    if sha1(&bytes[32..]) != header.signature {
        return Err(DexError::SignatureMismatch);
    }
    Ok(())
}

fn validate_header_ranges(reader: &Reader<'_>, header: &DexHeader) -> Result<(), DexError> {
    validate_table(reader, header.string_ids_off, header.string_ids_size, 4, "string_ids")?;
    validate_table(reader, header.type_ids_off, header.type_ids_size, 4, "type_ids")?;
    validate_table(reader, header.proto_ids_off, header.proto_ids_size, 12, "proto_ids")?;
    validate_table(reader, header.field_ids_off, header.field_ids_size, 8, "field_ids")?;
    validate_table(reader, header.method_ids_off, header.method_ids_size, 8, "method_ids")?;
    validate_table(reader, header.class_defs_off, header.class_defs_size, 32, "class_defs")?;
    validate_table(reader, header.link_off, header.link_size, 1, "link_data")?;
    let data_off = usize_from_u32(header.data_off, "data")?;
    let data_size = usize_from_u32(header.data_size, "data")?;
    reader.range(data_off, data_size, "data")?;
    if header.data_size > 0 && data_off < HEADER_SIZE {
        return Err(DexError::InvalidHeader("data section overlaps the header".to_owned()));
    }
    if header.map_off == 0 {
        return Err(DexError::InvalidHeader("map_off is zero".to_owned()));
    }
    validate_data_offset(header, header.map_off, "map_list")
}

fn validate_table(
    reader: &Reader<'_>,
    offset: u32,
    count: u32,
    item_size: usize,
    section: &'static str,
) -> Result<(), DexError> {
    if count == 0 {
        if offset != 0 {
            return Err(DexError::Malformed {
                section,
                reason: "empty section has a non-zero offset".to_owned(),
            });
        }
        return Ok(());
    }
    if offset == 0 {
        return Err(DexError::Malformed {
            section,
            reason: "non-empty section has a zero offset".to_owned(),
        });
    }
    let offset = usize_from_u32(offset, section)?;
    let count = usize_from_u32(count, section)?;
    let size = count.checked_mul(item_size).ok_or(DexError::Bounds {
        section,
        offset,
        size: usize::MAX,
    })?;
    reader.range(offset, size, section)?;
    Ok(())
}

fn validate_data_offset(
    header: &DexHeader,
    offset: u32,
    section: &'static str,
) -> Result<(), DexError> {
    if offset == 0 {
        return Ok(());
    }
    let data_end = header
        .data_off
        .checked_add(header.data_size)
        .ok_or_else(|| DexError::InvalidHeader("data range overflows".to_owned()))?;
    if offset < header.data_off || offset >= data_end {
        return Err(DexError::Bounds {
            section,
            offset: usize_from_u32(offset, section)?,
            size: 1,
        });
    }
    Ok(())
}

fn parse_map(reader: &Reader<'_>, header: &DexHeader) -> Result<Vec<MapItem>, DexError> {
    let offset = usize_from_u32(header.map_off, "map_list")?;
    let count = reader.u32(offset, "map_list")?;
    let count_usize = usize_from_u32(count, "map_list")?;
    let list_size = count_usize
        .checked_mul(12)
        .and_then(|size| size.checked_add(4))
        .ok_or(DexError::Bounds { section: "map_list", offset, size: usize::MAX })?;
    reader.range(offset, list_size, "map_list")?;
    let mut items = Vec::with_capacity(count_usize);
    let mut previous_offset = None;
    for index in 0..count_usize {
        let item_off = offset + 4 + index * 12;
        let item = MapItem {
            type_code: reader.u16(item_off, "map_list")?,
            size: reader.u32(item_off + 4, "map_list")?,
            offset: reader.u32(item_off + 8, "map_list")?,
        };
        if item.size == 0 {
            return Err(DexError::Malformed {
                section: "map_list",
                reason: "map item has zero size".to_owned(),
            });
        }
        if previous_offset.is_some_and(|previous| item.offset < previous) {
            return Err(DexError::Malformed {
                section: "map_list",
                reason: "map items are not sorted by offset".to_owned(),
            });
        }
        previous_offset = Some(item.offset);
        validate_map_item(reader, header, item)?;
        items.push(item);
    }
    Ok(items)
}

fn validate_map_item(
    reader: &Reader<'_>,
    header: &DexHeader,
    item: MapItem,
) -> Result<(), DexError> {
    let fixed_width = match item.type_code {
        0x0000 => Some(HEADER_SIZE),
        0x0001 | 0x0002 => Some(4),
        0x0003 | 0x1000 => Some(12),
        0x0004 | 0x0005 => Some(8),
        0x0006 => Some(32),
        0x1001 | 0x1002 | 0x1003 | 0x2000..=0x2006 => None,
        value => {
            return Err(DexError::Malformed {
                section: "map_list",
                reason: format!("unknown map item type {value:#06x}"),
            });
        }
    };
    if item.type_code == 0x0000 && (item.offset != 0 || item.size != 1) {
        return Err(DexError::Malformed {
            section: "map_list",
            reason: "invalid header map item".to_owned(),
        });
    }
    if item.type_code >= 0x1000 {
        validate_data_offset(header, item.offset, "map item")?;
    }
    let offset = usize_from_u32(item.offset, "map item")?;
    if let Some(width) = fixed_width {
        let count = usize_from_u32(item.size, "map item")?;
        let size = count.checked_mul(width).ok_or(DexError::Bounds {
            section: "map item",
            offset,
            size: usize::MAX,
        })?;
        reader.range(offset, size, "map item")?;
    } else {
        reader.range(offset, 1, "map item")?;
    }
    Ok(())
}

fn parse_strings(reader: &Reader<'_>, header: &DexHeader) -> Result<Vec<String>, DexError> {
    let count = usize_from_u32(header.string_ids_size, "string_ids")?;
    let table = usize_from_u32(header.string_ids_off, "string_ids")?;
    let mut strings = Vec::with_capacity(count);
    for index in 0..count {
        let data_off = reader.u32(table + index * 4, "string_ids")?;
        validate_data_offset(header, data_off, "string_data")?;
        strings.push(parse_string_data(reader, usize_from_u32(data_off, "string_data")?)?);
    }
    Ok(strings)
}

fn parse_string_data(reader: &Reader<'_>, offset: usize) -> Result<String, DexError> {
    let mut cursor = offset;
    let utf16_size = reader.uleb128(&mut cursor)?;
    let mut units = Vec::with_capacity(usize_from_u32(utf16_size, "string_data")?);
    loop {
        let first = reader.u8(cursor, "string_data")?;
        cursor += 1;
        if first == 0 {
            break;
        }
        let unit = if first & 0x80 == 0 {
            u16::from(first)
        } else if first & 0xe0 == 0xc0 {
            let second = reader.u8(cursor, "string_data")?;
            cursor += 1;
            if second & 0xc0 != 0x80 {
                return invalid_string(offset, "invalid two-byte sequence");
            }
            let value = (u16::from(first & 0x1f) << 6) | u16::from(second & 0x3f);
            if value != 0 && value < 0x80 {
                return invalid_string(offset, "overlong two-byte sequence");
            }
            value
        } else if first & 0xf0 == 0xe0 {
            let second = reader.u8(cursor, "string_data")?;
            let third = reader.u8(cursor + 1, "string_data")?;
            cursor += 2;
            if second & 0xc0 != 0x80 || third & 0xc0 != 0x80 {
                return invalid_string(offset, "invalid three-byte sequence");
            }
            let value = (u16::from(first & 0x0f) << 12)
                | (u16::from(second & 0x3f) << 6)
                | u16::from(third & 0x3f);
            if value < 0x800 {
                return invalid_string(offset, "overlong three-byte sequence");
            }
            value
        } else {
            return invalid_string(offset, "four-byte UTF-8 is not valid MUTF-8");
        };
        units.push(unit);
    }
    if units.len() != usize_from_u32(utf16_size, "string_data")? {
        return invalid_string(offset, "declared UTF-16 length does not match data");
    }
    String::from_utf16(&units)
        .map_err(|error| DexError::InvalidString { offset, reason: error.to_string() })
}

fn invalid_string<T>(offset: usize, reason: &str) -> Result<T, DexError> {
    Err(DexError::InvalidString { offset, reason: reason.to_owned() })
}

fn parse_types(
    reader: &Reader<'_>,
    header: &DexHeader,
    string_count: usize,
) -> Result<Vec<TypeId>, DexError> {
    let count = usize_from_u32(header.type_ids_size, "type_ids")?;
    let table = usize_from_u32(header.type_ids_off, "type_ids")?;
    let mut types = Vec::with_capacity(count);
    for index in 0..count {
        let descriptor_idx = reader.u32(table + index * 4, "type_ids")?;
        validate_index(descriptor_idx, string_count, "string")?;
        types.push(TypeId { descriptor_idx });
    }
    Ok(types)
}

fn parse_protos(
    reader: &Reader<'_>,
    header: &DexHeader,
    string_count: usize,
    type_count: usize,
) -> Result<Vec<ProtoId>, DexError> {
    let count = usize_from_u32(header.proto_ids_size, "proto_ids")?;
    let table = usize_from_u32(header.proto_ids_off, "proto_ids")?;
    let mut protos = Vec::with_capacity(count);
    for index in 0..count {
        let offset = table + index * 12;
        let shorty_idx = reader.u32(offset, "proto_ids")?;
        let return_type_idx = reader.u32(offset + 4, "proto_ids")?;
        let parameters_off = reader.u32(offset + 8, "proto_ids")?;
        validate_index(shorty_idx, string_count, "string")?;
        validate_index(return_type_idx, type_count, "type")?;
        let parameters = if parameters_off == 0 {
            Vec::new()
        } else {
            validate_data_offset(header, parameters_off, "type_list")?;
            parse_type_list(reader, parameters_off, type_count)?
        };
        protos.push(ProtoId { shorty_idx, return_type_idx, parameters_off, parameters });
    }
    Ok(protos)
}

fn parse_type_list(
    reader: &Reader<'_>,
    offset: u32,
    type_count: usize,
) -> Result<Vec<u32>, DexError> {
    let offset = usize_from_u32(offset, "type_list")?;
    let count = usize_from_u32(reader.u32(offset, "type_list")?, "type_list")?;
    let byte_size = count.checked_mul(2).ok_or(DexError::Bounds {
        section: "type_list",
        offset,
        size: usize::MAX,
    })?;
    reader.range(offset + 4, byte_size, "type_list")?;
    let mut result = Vec::with_capacity(count);
    for index in 0..count {
        let type_idx = u32::from(reader.u16(offset + 4 + index * 2, "type_list")?);
        validate_index(type_idx, type_count, "type")?;
        result.push(type_idx);
    }
    Ok(result)
}

fn parse_fields(
    reader: &Reader<'_>,
    header: &DexHeader,
    string_count: usize,
    type_count: usize,
) -> Result<Vec<FieldId>, DexError> {
    let count = usize_from_u32(header.field_ids_size, "field_ids")?;
    let table = usize_from_u32(header.field_ids_off, "field_ids")?;
    let mut fields = Vec::with_capacity(count);
    for index in 0..count {
        let offset = table + index * 8;
        let field = FieldId {
            class_idx: reader.u16(offset, "field_ids")?,
            type_idx: reader.u16(offset + 2, "field_ids")?,
            name_idx: reader.u32(offset + 4, "field_ids")?,
        };
        validate_index(u32::from(field.class_idx), type_count, "type")?;
        validate_index(u32::from(field.type_idx), type_count, "type")?;
        validate_index(field.name_idx, string_count, "string")?;
        fields.push(field);
    }
    Ok(fields)
}

fn parse_methods(
    reader: &Reader<'_>,
    header: &DexHeader,
    string_count: usize,
    type_count: usize,
    proto_count: usize,
) -> Result<Vec<MethodId>, DexError> {
    let count = usize_from_u32(header.method_ids_size, "method_ids")?;
    let table = usize_from_u32(header.method_ids_off, "method_ids")?;
    let mut methods = Vec::with_capacity(count);
    for index in 0..count {
        let offset = table + index * 8;
        let method = MethodId {
            class_idx: reader.u16(offset, "method_ids")?,
            proto_idx: reader.u16(offset + 2, "method_ids")?,
            name_idx: reader.u32(offset + 4, "method_ids")?,
        };
        validate_index(u32::from(method.class_idx), type_count, "type")?;
        validate_index(u32::from(method.proto_idx), proto_count, "prototype")?;
        validate_index(method.name_idx, string_count, "string")?;
        methods.push(method);
    }
    Ok(methods)
}

fn parse_classes(
    reader: &Reader<'_>,
    header: &DexHeader,
    type_count: usize,
    field_count: usize,
    method_count: usize,
) -> Result<(Vec<ClassDef>, HashMap<u32, CodeItem>), DexError> {
    let count = usize_from_u32(header.class_defs_size, "class_defs")?;
    let table = usize_from_u32(header.class_defs_off, "class_defs")?;
    let mut classes = Vec::with_capacity(count);
    let mut method_code = HashMap::new();
    for index in 0..count {
        let offset = table + index * 32;
        let class_idx = reader.u32(offset, "class_defs")?;
        let access_flags = reader.u32(offset + 4, "class_defs")?;
        let superclass_raw = reader.u32(offset + 8, "class_defs")?;
        let interfaces_off = reader.u32(offset + 12, "class_defs")?;
        let source_file_raw = reader.u32(offset + 16, "class_defs")?;
        let annotations_off = reader.u32(offset + 20, "class_defs")?;
        let class_data_off = reader.u32(offset + 24, "class_defs")?;
        let static_values_off = reader.u32(offset + 28, "class_defs")?;
        validate_index(class_idx, type_count, "type")?;
        let superclass_idx = optional_index(superclass_raw, type_count, "type")?;
        let source_file_idx = if source_file_raw == NO_INDEX {
            None
        } else {
            validate_index(
                source_file_raw,
                usize_from_u32(header.string_ids_size, "string_ids")?,
                "string",
            )?;
            Some(source_file_raw)
        };
        let interfaces = if interfaces_off == 0 {
            Vec::new()
        } else {
            validate_data_offset(header, interfaces_off, "interfaces")?;
            parse_type_list(reader, interfaces_off, type_count)?
        };
        validate_data_offset(header, annotations_off, "annotations")?;
        validate_data_offset(header, class_data_off, "class_data")?;
        validate_data_offset(header, static_values_off, "static_values")?;
        let (static_fields, instance_fields, direct_methods, virtual_methods) =
            if class_data_off == 0 {
                (Vec::new(), Vec::new(), Vec::new(), Vec::new())
            } else {
                parse_class_data(
                    reader,
                    header,
                    class_data_off,
                    type_count,
                    field_count,
                    method_count,
                    &mut method_code,
                )?
            };
        classes.push(ClassDef {
            class_idx,
            access_flags,
            superclass_idx,
            interfaces_off,
            interfaces,
            source_file_idx,
            annotations_off,
            class_data_off,
            static_values_off,
            static_fields,
            instance_fields,
            direct_methods,
            virtual_methods,
        });
    }
    Ok((classes, method_code))
}

type ClassData = (Vec<EncodedField>, Vec<EncodedField>, Vec<EncodedMethod>, Vec<EncodedMethod>);

fn parse_class_data(
    reader: &Reader<'_>,
    header: &DexHeader,
    class_data_off: u32,
    type_count: usize,
    field_count: usize,
    method_count: usize,
    method_code: &mut HashMap<u32, CodeItem>,
) -> Result<ClassData, DexError> {
    let mut cursor = usize_from_u32(class_data_off, "class_data")?;
    let static_count = reader.uleb128(&mut cursor)?;
    let instance_count = reader.uleb128(&mut cursor)?;
    let direct_count = reader.uleb128(&mut cursor)?;
    let virtual_count = reader.uleb128(&mut cursor)?;
    let static_fields = parse_encoded_fields(reader, &mut cursor, static_count, field_count)?;
    let instance_fields = parse_encoded_fields(reader, &mut cursor, instance_count, field_count)?;
    let direct_methods = parse_encoded_methods(
        reader,
        header,
        &mut cursor,
        direct_count,
        type_count,
        method_count,
        method_code,
    )?;
    let virtual_methods = parse_encoded_methods(
        reader,
        header,
        &mut cursor,
        virtual_count,
        type_count,
        method_count,
        method_code,
    )?;
    Ok((static_fields, instance_fields, direct_methods, virtual_methods))
}

fn parse_encoded_fields(
    reader: &Reader<'_>,
    cursor: &mut usize,
    count: u32,
    field_count: usize,
) -> Result<Vec<EncodedField>, DexError> {
    let mut fields = Vec::with_capacity(usize_from_u32(count, "class_data")?);
    let mut index = 0_u32;
    for _ in 0..count {
        let difference = reader.uleb128(cursor)?;
        index = index.checked_add(difference).ok_or_else(|| DexError::Malformed {
            section: "class_data",
            reason: "field index overflows".to_owned(),
        })?;
        validate_index(index, field_count, "field")?;
        fields.push(EncodedField { field_idx: index, access_flags: reader.uleb128(cursor)? });
    }
    Ok(fields)
}

fn parse_encoded_methods(
    reader: &Reader<'_>,
    header: &DexHeader,
    cursor: &mut usize,
    count: u32,
    type_count: usize,
    method_count: usize,
    method_code: &mut HashMap<u32, CodeItem>,
) -> Result<Vec<EncodedMethod>, DexError> {
    let mut methods = Vec::with_capacity(usize_from_u32(count, "class_data")?);
    let mut index = 0_u32;
    for _ in 0..count {
        let difference = reader.uleb128(cursor)?;
        index = index.checked_add(difference).ok_or_else(|| DexError::Malformed {
            section: "class_data",
            reason: "method index overflows".to_owned(),
        })?;
        validate_index(index, method_count, "method")?;
        let access_flags = reader.uleb128(cursor)?;
        let code_off = reader.uleb128(cursor)?;
        if code_off != 0 {
            validate_data_offset(header, code_off, "code_item")?;
            if code_off % 4 != 0 {
                return Err(DexError::Malformed {
                    section: "code_item",
                    reason: "code offset is not four-byte aligned".to_owned(),
                });
            }
            let code = parse_code_item(reader, code_off, type_count)?;
            if method_code.insert(index, code).is_some() {
                return Err(DexError::Malformed {
                    section: "class_data",
                    reason: format!("method {index} has duplicate code"),
                });
            }
        }
        methods.push(EncodedMethod { method_idx: index, access_flags, code_off });
    }
    Ok(methods)
}

fn parse_code_item(
    reader: &Reader<'_>,
    code_off: u32,
    type_count: usize,
) -> Result<CodeItem, DexError> {
    let offset = usize_from_u32(code_off, "code_item")?;
    let registers_size = reader.u16(offset, "code_item")?;
    let ins_size = reader.u16(offset + 2, "code_item")?;
    let outs_size = reader.u16(offset + 4, "code_item")?;
    let tries_size = reader.u16(offset + 6, "code_item")?;
    let debug_info_off = reader.u32(offset + 8, "code_item")?;
    let instructions_size = reader.u32(offset + 12, "code_item")?;
    if ins_size > registers_size {
        return Err(DexError::Malformed {
            section: "code_item",
            reason: "ins_size exceeds registers_size".to_owned(),
        });
    }
    if debug_info_off != 0 {
        reader.range(usize_from_u32(debug_info_off, "debug_info")?, 1, "debug_info")?;
    }
    let units = usize_from_u32(instructions_size, "code_item")?;
    let instruction_bytes = units.checked_mul(2).ok_or(DexError::Bounds {
        section: "code_item",
        offset,
        size: usize::MAX,
    })?;
    reader.range(offset + 16, instruction_bytes, "code_item")?;
    let mut instructions = Vec::with_capacity(units);
    for index in 0..units {
        instructions.push(reader.u16(offset + 16 + index * 2, "code_item")?);
    }
    let mut tries = Vec::with_capacity(usize::from(tries_size));
    let mut handlers = Vec::new();
    if tries_size > 0 {
        let padding = usize::try_from(instructions_size % 2).map_err(|_| DexError::Malformed {
            section: "code_item",
            reason: "instruction padding overflows".to_owned(),
        })? * 2;
        let tries_off = offset + 16 + instruction_bytes + padding;
        let tries_bytes = usize::from(tries_size) * 8;
        reader.range(tries_off, tries_bytes, "try_items")?;
        for index in 0..usize::from(tries_size) {
            let item_off = tries_off + index * 8;
            let item = TryItem {
                start_addr: reader.u32(item_off, "try_items")?,
                instruction_count: reader.u16(item_off + 4, "try_items")?,
                handler_off: reader.u16(item_off + 6, "try_items")?,
            };
            let end = item.start_addr.checked_add(u32::from(item.instruction_count)).ok_or_else(
                || DexError::Malformed {
                    section: "try_items",
                    reason: "protected range overflows".to_owned(),
                },
            )?;
            if item.instruction_count == 0 || end > instructions_size {
                return Err(DexError::Malformed {
                    section: "try_items",
                    reason: "protected range is outside instructions".to_owned(),
                });
            }
            tries.push(item);
        }
        let handlers_off = tries_off + tries_bytes;
        handlers = parse_catch_handlers(reader, handlers_off, instructions_size, type_count)?;
        for item in &tries {
            if !handlers.iter().any(|handler| handler.offset == u32::from(item.handler_off)) {
                return Err(DexError::Malformed {
                    section: "try_items",
                    reason: "handler_off does not identify a catch handler".to_owned(),
                });
            }
        }
    }
    Ok(CodeItem {
        offset: code_off,
        registers_size,
        ins_size,
        outs_size,
        debug_info_off,
        instructions,
        tries,
        handlers,
    })
}

fn parse_catch_handlers(
    reader: &Reader<'_>,
    offset: usize,
    instructions_size: u32,
    type_count: usize,
) -> Result<Vec<CatchHandler>, DexError> {
    let mut cursor = offset;
    let count = reader.uleb128(&mut cursor)?;
    let mut handlers = Vec::with_capacity(usize_from_u32(count, "catch_handlers")?);
    for _ in 0..count {
        let relative = u32::try_from(cursor - offset).map_err(|_| DexError::Malformed {
            section: "catch_handlers",
            reason: "handler offset exceeds u32".to_owned(),
        })?;
        let signed_count = reader.sleb128(&mut cursor)?;
        if signed_count == i32::MIN {
            return Err(DexError::Malformed {
                section: "catch_handlers",
                reason: "handler count overflows".to_owned(),
            });
        }
        let handler_type_count = signed_count.unsigned_abs();
        let mut catches = Vec::with_capacity(usize_from_u32(handler_type_count, "catch_handlers")?);
        for _ in 0..handler_type_count {
            let type_idx = reader.uleb128(&mut cursor)?;
            validate_index(type_idx, type_count, "type")?;
            let address = reader.uleb128(&mut cursor)?;
            if address >= instructions_size {
                return Err(DexError::Malformed {
                    section: "catch_handlers",
                    reason: "handler address is outside instructions".to_owned(),
                });
            }
            catches.push((type_idx, address));
        }
        let catch_all_addr = if signed_count <= 0 {
            let address = reader.uleb128(&mut cursor)?;
            if address >= instructions_size {
                return Err(DexError::Malformed {
                    section: "catch_handlers",
                    reason: "catch-all address is outside instructions".to_owned(),
                });
            }
            Some(address)
        } else {
            None
        };
        handlers.push(CatchHandler { offset: relative, catches, catch_all_addr });
    }
    Ok(handlers)
}

fn validate_index(index: u32, limit: usize, kind: &'static str) -> Result<(), DexError> {
    let converted =
        usize::try_from(index).map_err(|_| DexError::InvalidIndex { kind, index, limit })?;
    if converted >= limit {
        return Err(DexError::InvalidIndex { kind, index, limit });
    }
    Ok(())
}

fn optional_index(index: u32, limit: usize, kind: &'static str) -> Result<Option<u32>, DexError> {
    if index == NO_INDEX {
        Ok(None)
    } else {
        validate_index(index, limit, kind)?;
        Ok(Some(index))
    }
}

fn get_index<'a, T>(values: &'a [T], index: u32, kind: &'static str) -> Result<&'a T, DexError> {
    let converted = usize::try_from(index).map_err(|_| DexError::InvalidIndex {
        kind,
        index,
        limit: values.len(),
    })?;
    values.get(converted).ok_or(DexError::InvalidIndex { kind, index, limit: values.len() })
}

fn usize_from_u32(value: u32, section: &'static str) -> Result<usize, DexError> {
    usize::try_from(value).map_err(|_| DexError::Bounds { section, offset: usize::MAX, size: 0 })
}

fn adler32(bytes: &[u8]) -> u32 {
    const MODULUS: u32 = 65_521;
    let mut first = 1_u32;
    let mut second = 0_u32;
    for chunk in bytes.chunks(5_552) {
        for byte in chunk {
            first += u32::from(*byte);
            second += first;
        }
        first %= MODULUS;
        second %= MODULUS;
    }
    (second << 16) | first
}

#[allow(clippy::many_single_char_names)]
fn sha1(bytes: &[u8]) -> [u8; 20] {
    let bit_length = u64::try_from(bytes.len()).unwrap_or(u64::MAX).wrapping_mul(8);
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

#[cfg(test)]
pub(crate) fn update_digests(bytes: &mut [u8]) {
    let signature = sha1(&bytes[32..]);
    bytes[12..32].copy_from_slice(&signature);
    let checksum = adler32(&bytes[12..]);
    bytes[8..12].copy_from_slice(&checksum.to_le_bytes());
}

#[cfg(test)]
mod digest_tests {
    use super::{adler32, sha1};

    #[test]
    fn digest_implementations_match_known_vectors() {
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
        assert_eq!(
            sha1(b"abc"),
            [
                0xa9, 0x99, 0x3e, 0x36, 0x47, 0x06, 0x81, 0x6a, 0xba, 0x3e, 0x25, 0x71, 0x78, 0x50,
                0xc2, 0x6c, 0x9c, 0xd0, 0xd8, 0x9d,
            ]
        );
    }
}
