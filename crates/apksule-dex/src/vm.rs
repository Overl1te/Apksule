#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use thiserror::Error;

use crate::{CodeItem, DexError, DexFile, ResolvedMethod};

const ACC_STATIC: u32 = 0x0008;

/// Ссылка на объект в куче виртуальной машины.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ObjectRef(pub u32);

/// Ссылка на массив в куче виртуальной машины.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ArrayRef(pub u32);

/// Типизированная ссылка на элемент кучи.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HeapRef {
    Object(ObjectRef),
    Array(ArrayRef),
}

/// Значение регистра DEX.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    Reference(HeapRef),
}

impl Value {
    fn int(&self) -> Result<i32, VmError> {
        match self {
            Self::Int(value) => Ok(*value),
            _ => Err(VmError::TypeMismatch("expected an int register")),
        }
    }

    fn long(&self) -> Result<i64, VmError> {
        match self {
            Self::Long(value) => Ok(*value),
            _ => Err(VmError::TypeMismatch("expected a long register")),
        }
    }

    fn float(&self) -> Result<f32, VmError> {
        match self {
            Self::Float(value) => Ok(*value),
            _ => Err(VmError::TypeMismatch("expected a float register")),
        }
    }

    fn double(&self) -> Result<f64, VmError> {
        match self {
            Self::Double(value) => Ok(*value),
            _ => Err(VmError::TypeMismatch("expected a double register")),
        }
    }

    fn object(&self) -> Result<ObjectRef, VmError> {
        match self {
            Self::Reference(HeapRef::Object(reference)) => Ok(*reference),
            Self::Null => Err(VmError::NullReference),
            _ => Err(VmError::TypeMismatch("expected an object reference")),
        }
    }

    fn array(&self) -> Result<ArrayRef, VmError> {
        match self {
            Self::Reference(HeapRef::Array(reference)) => Ok(*reference),
            Self::Null => Err(VmError::NullReference),
            _ => Err(VmError::TypeMismatch("expected an array reference")),
        }
    }
}

/// Результат попытки обработать вызов внешним мостом.
#[derive(Debug, Clone, PartialEq)]
pub enum NativeResult {
    Handled(Value),
    Unresolved,
}

/// Мост для native- и framework-методов, отсутствующих в DEX.
pub trait NativeBridge {
    fn invoke(
        &mut self,
        method: &ResolvedMethod,
        arguments: &[Value],
    ) -> Result<NativeResult, VmError>;
}

/// Детерминированные ограничения одного верхнеуровневого вызова.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VmLimits {
    pub max_instructions: u64,
    pub max_duration: Duration,
    pub max_heap_entries: usize,
    pub max_array_length: usize,
    pub max_call_depth: usize,
}

impl Default for VmLimits {
    fn default() -> Self {
        Self {
            max_instructions: 1_000_000,
            max_duration: Duration::from_secs(5),
            max_heap_entries: 100_000,
            max_array_length: 1_000_000,
            max_call_depth: 256,
        }
    }
}

/// Ошибка исполнения DEX.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum VmError {
    #[error(transparent)]
    Dex(#[from] DexError),
    #[error("method not found: {class_descriptor}->{name}{prototype}")]
    MethodNotFound { class_descriptor: String, name: String, prototype: String },
    #[error("method has no DEX code and no native implementation: {0}")]
    UnresolvedNative(ResolvedMethod),
    #[error("native bridge failed: {0}")]
    NativeBridge(String),
    #[error("unsupported opcode {opcode:#04x} at code-unit {pc}")]
    UnsupportedOpcode { opcode: u8, pc: usize },
    #[error("malformed instruction at code-unit {pc}: {reason}")]
    MalformedInstruction { pc: usize, reason: String },
    #[error("register {index} is outside frame of size {count}")]
    InvalidRegister { index: usize, count: usize },
    #[error("{0}")]
    TypeMismatch(&'static str),
    #[error("null reference")]
    NullReference,
    #[error("invalid heap reference")]
    InvalidReference,
    #[error("array index {index} is outside length {length}")]
    ArrayBounds { index: i32, length: usize },
    #[error("integer division by zero")]
    DivisionByZero,
    #[error("uncaught exception: {0:?}")]
    UncaughtException(Value),
    #[error("instruction limit exceeded")]
    InstructionLimit,
    #[error("execution time limit exceeded")]
    TimeLimit,
    #[error("heap entry limit exceeded")]
    HeapLimit,
    #[error("array length limit exceeded")]
    ArrayLimit,
    #[error("call depth limit exceeded")]
    CallDepthLimit,
    #[error("branch target is outside method code")]
    InvalidBranch,
    #[error("argument count or register width does not match the method prototype")]
    InvalidArguments,
    #[error("negative array length {0}")]
    NegativeArrayLength(i32),
}

#[derive(Debug, Clone)]
enum HeapEntry {
    Object { class_idx: u32, fields: HashMap<u32, Value> },
    String(String),
    Class(u32),
    Array { type_idx: u32, elements: Vec<Value> },
}

#[derive(Debug)]
struct Frame {
    registers: Vec<Value>,
    pc: usize,
    result: Option<Value>,
    pending_exception: Option<Value>,
}

impl Frame {
    fn new(count: usize) -> Self {
        Self { registers: vec![Value::Null; count], pc: 0, result: None, pending_exception: None }
    }

    fn read(&self, index: usize) -> Result<Value, VmError> {
        self.registers
            .get(index)
            .cloned()
            .ok_or(VmError::InvalidRegister { index, count: self.registers.len() })
    }

    fn write(&mut self, index: usize, value: Value) -> Result<(), VmError> {
        let count = self.registers.len();
        let is_wide = matches!(value, Value::Long(_) | Value::Double(_));
        let destination =
            self.registers.get_mut(index).ok_or(VmError::InvalidRegister { index, count })?;
        *destination = value;
        if is_wide {
            let second = self
                .registers
                .get_mut(index + 1)
                .ok_or(VmError::InvalidRegister { index: index + 1, count })?;
            *second = Value::Null;
        }
        Ok(())
    }
}

#[derive(Debug)]
struct Budget {
    remaining: u64,
    started: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InvokeKind {
    Virtual,
    Super,
    Direct,
    Static,
    Interface,
}

/// Минимальная регистровая виртуальная машина DEX.
pub struct Vm {
    dex: Arc<DexFile>,
    limits: VmLimits,
    heap: Vec<HeapEntry>,
    static_fields: HashMap<u32, Value>,
    initialized_classes: HashSet<u32>,
    initializing_classes: HashSet<u32>,
    native_bridge: Option<Box<dyn NativeBridge>>,
}

impl Vm {
    #[must_use]
    pub fn new(dex: &DexFile) -> Self {
        Self::from_dex(dex.clone())
    }

    #[must_use]
    pub fn from_dex(dex: DexFile) -> Self {
        Self {
            dex: Arc::new(dex),
            limits: VmLimits::default(),
            heap: Vec::new(),
            static_fields: HashMap::new(),
            initialized_classes: HashSet::new(),
            initializing_classes: HashSet::new(),
            native_bridge: None,
        }
    }

    #[must_use]
    pub fn with_native_bridge<B>(dex: &DexFile, bridge: B) -> Self
    where
        B: NativeBridge + 'static,
    {
        let mut vm = Self::new(dex);
        vm.native_bridge = Some(Box::new(bridge));
        vm
    }

    #[must_use]
    pub fn with_owned_native_bridge<B>(dex: DexFile, bridge: B) -> Self
    where
        B: NativeBridge + 'static,
    {
        let mut vm = Self::from_dex(dex);
        vm.native_bridge = Some(Box::new(bridge));
        vm
    }

    pub fn set_native_bridge<B>(&mut self, bridge: B)
    where
        B: NativeBridge + 'static,
    {
        self.native_bridge = Some(Box::new(bridge));
    }

    #[must_use]
    pub fn dex(&self) -> &DexFile {
        &self.dex
    }

    #[must_use]
    pub const fn limits(&self) -> VmLimits {
        self.limits
    }

    pub const fn set_limits(&mut self, limits: VmLimits) {
        self.limits = limits;
    }

    #[must_use]
    pub fn heap_len(&self) -> usize {
        self.heap.len()
    }

    pub fn invoke(
        &mut self,
        class_descriptor: &str,
        name: &str,
        prototype: &str,
        arguments: &[Value],
    ) -> Result<Value, VmError> {
        let handle =
            self.dex.find_method(class_descriptor, name, Some(prototype)).ok_or_else(|| {
                VmError::MethodNotFound {
                    class_descriptor: class_descriptor.to_owned(),
                    name: name.to_owned(),
                    prototype: prototype.to_owned(),
                }
            })?;
        self.invoke_index(handle.index, arguments)
    }

    pub fn invoke_index(&mut self, method_idx: u32, arguments: &[Value]) -> Result<Value, VmError> {
        let mut budget =
            Budget { remaining: self.limits.max_instructions, started: Instant::now() };
        if self
            .dex
            .encoded_method(method_idx)
            .is_some_and(|method| method.access_flags & ACC_STATIC != 0)
        {
            let class_idx = u32::from(
                self.dex
                    .methods()
                    .get(usize_from_u32(method_idx)?)
                    .ok_or_else(|| DexError::InvalidIndex {
                        kind: "method",
                        index: method_idx,
                        limit: self.dex.methods().len(),
                    })?
                    .class_idx,
            );
            self.ensure_class_initialized(class_idx, &mut budget, 0)?;
        }
        self.execute_method(method_idx, arguments, &mut budget, 0)
    }

    pub fn allocate_object(&mut self, class_descriptor: &str) -> Result<ObjectRef, VmError> {
        let class = self
            .dex
            .find_class(class_descriptor)
            .ok_or(VmError::TypeMismatch("class is not defined in this DEX"))?;
        self.allocate_object_type(class.class_idx)
    }

    pub fn allocate_array(
        &mut self,
        type_descriptor: &str,
        length: usize,
    ) -> Result<ArrayRef, VmError> {
        let type_idx = self
            .dex
            .types()
            .iter()
            .enumerate()
            .find_map(|(index, _)| {
                let index = u32::try_from(index).ok()?;
                (self.dex.type_descriptor(index).ok()? == type_descriptor).then_some(index)
            })
            .ok_or(VmError::TypeMismatch("array type is absent from this DEX"))?;
        self.allocate_array_type(type_idx, length)
    }

    pub fn array_get(&self, reference: ArrayRef, index: i32) -> Result<Value, VmError> {
        let HeapEntry::Array { elements, .. } = self.heap_entry(reference.0)? else {
            return Err(VmError::InvalidReference);
        };
        let index = checked_array_index(index, elements.len())?;
        Ok(elements[index].clone())
    }

    pub fn array_set(
        &mut self,
        reference: ArrayRef,
        index: i32,
        value: Value,
    ) -> Result<(), VmError> {
        let entry = self.heap_entry_mut(reference.0)?;
        let HeapEntry::Array { elements, .. } = entry else {
            return Err(VmError::InvalidReference);
        };
        let index = checked_array_index(index, elements.len())?;
        elements[index] = value;
        Ok(())
    }

    #[must_use]
    pub fn string_value(&self, reference: ObjectRef) -> Option<&str> {
        match self.heap.get(usize::try_from(reference.0).ok()?)? {
            HeapEntry::String(value) => Some(value),
            _ => None,
        }
    }

    pub fn read_instance_field(
        &self,
        reference: ObjectRef,
        field_idx: u32,
    ) -> Result<Value, VmError> {
        let HeapEntry::Object { fields, .. } = self.heap_entry(reference.0)? else {
            return Err(VmError::InvalidReference);
        };
        if let Some(value) = fields.get(&field_idx) {
            return Ok(value.clone());
        }
        self.default_field_value(field_idx)
    }

    pub fn write_instance_field(
        &mut self,
        reference: ObjectRef,
        field_idx: u32,
        value: Value,
    ) -> Result<(), VmError> {
        self.dex.resolve_field(field_idx)?;
        let HeapEntry::Object { fields, .. } = self.heap_entry_mut(reference.0)? else {
            return Err(VmError::InvalidReference);
        };
        fields.insert(field_idx, value);
        Ok(())
    }

    fn tick(&self, budget: &mut Budget) -> Result<(), VmError> {
        if budget.remaining == 0 {
            return Err(VmError::InstructionLimit);
        }
        budget.remaining -= 1;
        if budget.started.elapsed() > self.limits.max_duration {
            return Err(VmError::TimeLimit);
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn execute_method(
        &mut self,
        method_idx: u32,
        arguments: &[Value],
        budget: &mut Budget,
        depth: usize,
    ) -> Result<Value, VmError> {
        if depth >= self.limits.max_call_depth {
            return Err(VmError::CallDepthLimit);
        }
        let Some(code) = self.dex.method_code(method_idx).cloned() else {
            return self.invoke_native(method_idx, arguments);
        };
        let mut frame = Frame::new(usize::from(code.registers_size));
        self.place_arguments(method_idx, &code, arguments, &mut frame)?;
        loop {
            self.tick(budget)?;
            let instruction = code_unit(&code, frame.pc)?;
            let opcode = instruction as u8;
            let instruction_pc = frame.pc;
            match opcode {
                0x00 => frame.pc += 1,
                0x01 | 0x04 | 0x07 => {
                    let destination = usize::from((instruction >> 8) & 0x0f);
                    let source = usize::from((instruction >> 12) & 0x0f);
                    frame.write(destination, frame.read(source)?)?;
                    frame.pc += 1;
                }
                0x02 | 0x05 | 0x08 => {
                    let destination = usize::from(instruction >> 8);
                    let source = usize::from(code_unit(&code, frame.pc + 1)?);
                    frame.write(destination, frame.read(source)?)?;
                    frame.pc += 2;
                }
                0x03 | 0x06 | 0x09 => {
                    let destination = usize::from(code_unit(&code, frame.pc + 1)?);
                    let source = usize::from(code_unit(&code, frame.pc + 2)?);
                    frame.write(destination, frame.read(source)?)?;
                    frame.pc += 3;
                }
                0x0a..=0x0c => {
                    let destination = usize::from(instruction >> 8);
                    let result = frame.result.take().ok_or_else(|| {
                        malformed(frame.pc, "move-result has no pending invocation result")
                    })?;
                    frame.write(destination, result)?;
                    frame.pc += 1;
                }
                0x0d => {
                    let destination = usize::from(instruction >> 8);
                    let exception = frame.pending_exception.take().ok_or_else(|| {
                        malformed(frame.pc, "move-exception has no pending exception")
                    })?;
                    frame.write(destination, exception)?;
                    frame.pc += 1;
                }
                0x0e => return Ok(Value::Null),
                0x0f..=0x11 => {
                    return frame.read(usize::from(instruction >> 8));
                }
                0x12 => {
                    let destination = usize::from((instruction >> 8) & 0x0f);
                    let literal = i32::from((instruction as i16) >> 12);
                    frame.write(destination, Value::Int(literal))?;
                    frame.pc += 1;
                }
                0x13 => {
                    let destination = usize::from(instruction >> 8);
                    let literal = i32::from(code_unit(&code, frame.pc + 1)? as i16);
                    frame.write(destination, Value::Int(literal))?;
                    frame.pc += 2;
                }
                0x14 => {
                    let destination = usize::from(instruction >> 8);
                    let literal = read_i32(&code, frame.pc + 1)?;
                    frame.write(destination, Value::Int(literal))?;
                    frame.pc += 3;
                }
                0x15 => {
                    let destination = usize::from(instruction >> 8);
                    let literal = i32::from(code_unit(&code, frame.pc + 1)?) << 16;
                    frame.write(destination, Value::Int(literal))?;
                    frame.pc += 2;
                }
                0x16 => {
                    let destination = usize::from(instruction >> 8);
                    let literal = i64::from(code_unit(&code, frame.pc + 1)? as i16);
                    frame.write(destination, Value::Long(literal))?;
                    frame.pc += 2;
                }
                0x17 => {
                    let destination = usize::from(instruction >> 8);
                    let literal = i64::from(read_i32(&code, frame.pc + 1)?);
                    frame.write(destination, Value::Long(literal))?;
                    frame.pc += 3;
                }
                0x18 => {
                    let destination = usize::from(instruction >> 8);
                    let literal = read_i64(&code, frame.pc + 1)?;
                    frame.write(destination, Value::Long(literal))?;
                    frame.pc += 5;
                }
                0x19 => {
                    let destination = usize::from(instruction >> 8);
                    let literal = i64::from(code_unit(&code, frame.pc + 1)?) << 48;
                    frame.write(destination, Value::Long(literal))?;
                    frame.pc += 2;
                }
                0x1a => {
                    let destination = usize::from(instruction >> 8);
                    let string_idx = u32::from(code_unit(&code, frame.pc + 1)?);
                    let value = self.dex.string(string_idx)?.to_owned();
                    let reference = self.allocate_heap(HeapEntry::String(value))?;
                    frame.write(
                        destination,
                        Value::Reference(HeapRef::Object(ObjectRef(reference))),
                    )?;
                    frame.pc += 2;
                }
                0x1b => {
                    let destination = usize::from(instruction >> 8);
                    let string_idx = read_u32(&code, frame.pc + 1)?;
                    let value = self.dex.string(string_idx)?.to_owned();
                    let reference = self.allocate_heap(HeapEntry::String(value))?;
                    frame.write(
                        destination,
                        Value::Reference(HeapRef::Object(ObjectRef(reference))),
                    )?;
                    frame.pc += 3;
                }
                0x1c => {
                    let destination = usize::from(instruction >> 8);
                    let type_idx = u32::from(code_unit(&code, frame.pc + 1)?);
                    self.dex.type_descriptor(type_idx)?;
                    let reference = self.allocate_heap(HeapEntry::Class(type_idx))?;
                    frame.write(
                        destination,
                        Value::Reference(HeapRef::Object(ObjectRef(reference))),
                    )?;
                    frame.pc += 2;
                }
                0x1f => {
                    let register = usize::from(instruction >> 8);
                    let type_idx = u32::from(code_unit(&code, frame.pc + 1)?);
                    let value = frame.read(register)?;
                    if !matches!(value, Value::Null) && !self.value_is_type(&value, type_idx)? {
                        return Err(VmError::TypeMismatch("check-cast failed"));
                    }
                    frame.pc += 2;
                }
                0x20 => {
                    let destination = usize::from((instruction >> 8) & 0x0f);
                    let source = usize::from(instruction >> 12);
                    let type_idx = u32::from(code_unit(&code, frame.pc + 1)?);
                    let result = self.value_is_type(&frame.read(source)?, type_idx)?;
                    frame.write(destination, Value::Int(i32::from(result)))?;
                    frame.pc += 2;
                }
                0x21 => {
                    let destination = usize::from((instruction >> 8) & 0x0f);
                    let source = usize::from(instruction >> 12);
                    let reference = frame.read(source)?.array()?;
                    let HeapEntry::Array { elements, .. } = self.heap_entry(reference.0)? else {
                        return Err(VmError::InvalidReference);
                    };
                    let length = i32::try_from(elements.len()).map_err(|_| VmError::ArrayLimit)?;
                    frame.write(destination, Value::Int(length))?;
                    frame.pc += 1;
                }
                0x22 => {
                    let destination = usize::from(instruction >> 8);
                    let type_idx = u32::from(code_unit(&code, frame.pc + 1)?);
                    self.ensure_class_initialized(type_idx, budget, depth + 1)?;
                    let reference = self.allocate_object_type(type_idx)?;
                    frame.write(destination, Value::Reference(HeapRef::Object(reference)))?;
                    frame.pc += 2;
                }
                0x23 => {
                    let destination = usize::from((instruction >> 8) & 0x0f);
                    let size_register = usize::from(instruction >> 12);
                    let type_idx = u32::from(code_unit(&code, frame.pc + 1)?);
                    let length = frame.read(size_register)?.int()?;
                    let length = usize::try_from(length)
                        .map_err(|_| VmError::NegativeArrayLength(length))?;
                    let reference = self.allocate_array_type(type_idx, length)?;
                    frame.write(destination, Value::Reference(HeapRef::Array(reference)))?;
                    frame.pc += 2;
                }
                0x24 | 0x25 => {
                    let (type_idx, registers, width) =
                        decode_invoke_registers(&code, frame.pc, opcode)?;
                    let length = registers.len();
                    let reference = self.allocate_array_type(type_idx, length)?;
                    for (index, register) in registers.into_iter().enumerate() {
                        self.array_set(
                            reference,
                            i32::try_from(index).map_err(|_| VmError::ArrayLimit)?,
                            frame.read(register)?,
                        )?;
                    }
                    frame.result = Some(Value::Reference(HeapRef::Array(reference)));
                    frame.pc += width;
                }
                0x26 => {
                    let array_register = usize::from(instruction >> 8);
                    let relative = read_i32(&code, frame.pc + 1)?;
                    let target = branch_target(&code, frame.pc, relative)?;
                    let reference = frame.read(array_register)?.array()?;
                    self.fill_array_data(&code, target, reference)?;
                    frame.pc += 3;
                }
                0x27 => {
                    let thrown = frame.read(usize::from(instruction >> 8))?;
                    if let Some(target) = self.catch_target(&code, frame.pc, &thrown)? {
                        frame.pending_exception = Some(thrown);
                        frame.pc = target;
                    } else {
                        return Err(VmError::UncaughtException(thrown));
                    }
                }
                0x28 => {
                    let relative = i32::from((instruction >> 8) as u8 as i8);
                    frame.pc = branch_target(&code, frame.pc, relative)?;
                }
                0x29 => {
                    let relative = i32::from(code_unit(&code, frame.pc + 1)? as i16);
                    frame.pc = branch_target(&code, frame.pc, relative)?;
                }
                0x2a => {
                    let relative = read_i32(&code, frame.pc + 1)?;
                    frame.pc = branch_target(&code, frame.pc, relative)?;
                }
                0x2b | 0x2c => {
                    let register = usize::from(instruction >> 8);
                    let relative = read_i32(&code, frame.pc + 1)?;
                    let target = branch_target(&code, frame.pc, relative)?;
                    let key = frame.read(register)?.int()?;
                    frame.pc = Self::switch_target(&code, frame.pc, target, key, opcode)?;
                }
                0x2d..=0x31 => {
                    execute_compare(&code, &mut frame, opcode)?;
                }
                0x32..=0x37 => {
                    let left = frame.read(usize::from((instruction >> 8) & 0x0f))?;
                    let right = frame.read(usize::from(instruction >> 12))?;
                    let relative = i32::from(code_unit(&code, frame.pc + 1)? as i16);
                    if compare_branch(opcode, &left, &right)? {
                        frame.pc = branch_target(&code, frame.pc, relative)?;
                    } else {
                        frame.pc += 2;
                    }
                }
                0x38..=0x3d => {
                    let value = frame.read(usize::from(instruction >> 8))?;
                    let relative = i32::from(code_unit(&code, frame.pc + 1)? as i16);
                    if compare_zero_branch(opcode, &value)? {
                        frame.pc = branch_target(&code, frame.pc, relative)?;
                    } else {
                        frame.pc += 2;
                    }
                }
                0x44..=0x4a => {
                    let destination = usize::from(instruction >> 8);
                    let registers = code_unit(&code, frame.pc + 1)?;
                    let array = frame.read(usize::from(registers & 0xff))?.array()?;
                    let index = frame.read(usize::from(registers >> 8))?.int()?;
                    frame.write(destination, self.array_get(array, index)?)?;
                    frame.pc += 2;
                }
                0x4b..=0x51 => {
                    let source = usize::from(instruction >> 8);
                    let registers = code_unit(&code, frame.pc + 1)?;
                    let array = frame.read(usize::from(registers & 0xff))?.array()?;
                    let index = frame.read(usize::from(registers >> 8))?.int()?;
                    self.array_set(array, index, frame.read(source)?)?;
                    frame.pc += 2;
                }
                0x52..=0x58 => {
                    let destination = usize::from((instruction >> 8) & 0x0f);
                    let object = frame.read(usize::from(instruction >> 12))?.object()?;
                    let field_idx = u32::from(code_unit(&code, frame.pc + 1)?);
                    frame.write(destination, self.read_instance_field(object, field_idx)?)?;
                    frame.pc += 2;
                }
                0x59..=0x5f => {
                    let source = usize::from((instruction >> 8) & 0x0f);
                    let object = frame.read(usize::from(instruction >> 12))?.object()?;
                    let field_idx = u32::from(code_unit(&code, frame.pc + 1)?);
                    self.write_instance_field(object, field_idx, frame.read(source)?)?;
                    frame.pc += 2;
                }
                0x60..=0x66 => {
                    let destination = usize::from(instruction >> 8);
                    let field_idx = u32::from(code_unit(&code, frame.pc + 1)?);
                    let field = self.dex.fields().get(usize_from_u32(field_idx)?).ok_or(
                        DexError::InvalidIndex {
                            kind: "field",
                            index: field_idx,
                            limit: self.dex.fields().len(),
                        },
                    )?;
                    self.ensure_class_initialized(u32::from(field.class_idx), budget, depth + 1)?;
                    let value = self
                        .static_fields
                        .get(&field_idx)
                        .cloned()
                        .map_or_else(|| self.default_field_value(field_idx), Ok)?;
                    frame.write(destination, value)?;
                    frame.pc += 2;
                }
                0x67..=0x6d => {
                    let source = usize::from(instruction >> 8);
                    let field_idx = u32::from(code_unit(&code, frame.pc + 1)?);
                    let field = self.dex.fields().get(usize_from_u32(field_idx)?).ok_or(
                        DexError::InvalidIndex {
                            kind: "field",
                            index: field_idx,
                            limit: self.dex.fields().len(),
                        },
                    )?;
                    self.ensure_class_initialized(u32::from(field.class_idx), budget, depth + 1)?;
                    self.static_fields.insert(field_idx, frame.read(source)?);
                    frame.pc += 2;
                }
                0x6e..=0x72 | 0x74..=0x78 => {
                    let (called_idx, raw_registers, width) =
                        decode_invoke_registers(&code, frame.pc, opcode)?;
                    let kind = match opcode {
                        0x6e | 0x74 => InvokeKind::Virtual,
                        0x6f | 0x75 => InvokeKind::Super,
                        0x70 | 0x76 => InvokeKind::Direct,
                        0x71 | 0x77 => InvokeKind::Static,
                        _ => InvokeKind::Interface,
                    };
                    let arguments = self.collect_arguments(
                        called_idx,
                        &raw_registers,
                        kind == InvokeKind::Static,
                        &frame,
                    )?;
                    match self.invoke_target(
                        kind,
                        called_idx,
                        method_idx,
                        &arguments,
                        budget,
                        depth + 1,
                    ) {
                        Ok(result) => {
                            frame.result = Some(result);
                            frame.pc += width;
                        }
                        Err(VmError::UncaughtException(thrown)) => {
                            if let Some(target) =
                                self.catch_target(&code, instruction_pc, &thrown)?
                            {
                                frame.pending_exception = Some(thrown);
                                frame.pc = target;
                            } else {
                                return Err(VmError::UncaughtException(thrown));
                            }
                        }
                        Err(error) => return Err(error),
                    }
                }
                0x7b..=0x8f => execute_conversion(&code, &mut frame, opcode)?,
                0x90..=0xaf => execute_binary(&code, &mut frame, opcode)?,
                0xb0..=0xcf => execute_binary_2addr(&code, &mut frame, opcode)?,
                0xd0..=0xd7 => execute_literal16(&code, &mut frame, opcode)?,
                0xd8..=0xe2 => execute_literal8(&code, &mut frame, opcode)?,
                _ => {
                    return Err(VmError::UnsupportedOpcode { opcode, pc: frame.pc });
                }
            }
        }
    }

    fn invoke_native(&mut self, method_idx: u32, arguments: &[Value]) -> Result<Value, VmError> {
        let resolved = self.dex.resolve_method(method_idx)?;
        let result = if let Some(bridge) = self.native_bridge.as_mut() {
            bridge.invoke(&resolved, arguments)?
        } else {
            NativeResult::Unresolved
        };
        match result {
            NativeResult::Handled(value) => Ok(value),
            NativeResult::Unresolved => Err(VmError::UnresolvedNative(resolved)),
        }
    }

    #[allow(clippy::similar_names)]
    fn invoke_target(
        &mut self,
        kind: InvokeKind,
        called_idx: u32,
        caller_idx: u32,
        arguments: &[Value],
        budget: &mut Budget,
        depth: usize,
    ) -> Result<Value, VmError> {
        let target_idx = match kind {
            InvokeKind::Virtual | InvokeKind::Interface => {
                let receiver = arguments.first().ok_or(VmError::InvalidArguments)?;
                let class_idx = self.object_class(receiver.object()?)?;
                self.find_virtual_override(class_idx, called_idx)?.unwrap_or(called_idx)
            }
            InvokeKind::Super => {
                let caller = self.dex.methods().get(usize_from_u32(caller_idx)?).ok_or(
                    DexError::InvalidIndex {
                        kind: "method",
                        index: caller_idx,
                        limit: self.dex.methods().len(),
                    },
                )?;
                let caller_class = self
                    .dex
                    .class_by_type(u32::from(caller.class_idx))
                    .ok_or(VmError::TypeMismatch("caller class is not defined"))?;
                caller_class
                    .superclass_idx
                    .and_then(|superclass| self.find_virtual_override(superclass, called_idx).ok())
                    .flatten()
                    .unwrap_or(called_idx)
            }
            InvokeKind::Direct | InvokeKind::Static => called_idx,
        };
        if kind == InvokeKind::Static {
            let method = self.dex.methods().get(usize_from_u32(target_idx)?).ok_or(
                DexError::InvalidIndex {
                    kind: "method",
                    index: target_idx,
                    limit: self.dex.methods().len(),
                },
            )?;
            self.ensure_class_initialized(u32::from(method.class_idx), budget, depth)?;
        }
        self.execute_method(target_idx, arguments, budget, depth)
    }

    fn find_virtual_override(
        &self,
        mut class_idx: u32,
        declared_idx: u32,
    ) -> Result<Option<u32>, VmError> {
        let declared = self.dex.methods().get(usize_from_u32(declared_idx)?).ok_or(
            DexError::InvalidIndex {
                kind: "method",
                index: declared_idx,
                limit: self.dex.methods().len(),
            },
        )?;
        let declared_name = self.dex.string(declared.name_idx)?;
        let declared_proto = declared.proto_idx;
        loop {
            for (index, method) in self.dex.methods().iter().enumerate() {
                if u32::from(method.class_idx) == class_idx
                    && method.proto_idx == declared_proto
                    && self.dex.string(method.name_idx)? == declared_name
                    && self
                        .dex
                        .encoded_method(
                            u32::try_from(index)
                                .map_err(|_| VmError::TypeMismatch("method table is too large"))?,
                        )
                        .is_some()
                {
                    return Ok(Some(
                        u32::try_from(index)
                            .map_err(|_| VmError::TypeMismatch("method table is too large"))?,
                    ));
                }
            }
            let Some(class) = self.dex.class_by_type(class_idx) else {
                return Ok(None);
            };
            let Some(superclass) = class.superclass_idx else {
                return Ok(None);
            };
            class_idx = superclass;
        }
    }

    fn collect_arguments(
        &self,
        method_idx: u32,
        registers: &[usize],
        is_static: bool,
        frame: &Frame,
    ) -> Result<Vec<Value>, VmError> {
        let method =
            self.dex.methods().get(usize_from_u32(method_idx)?).ok_or(DexError::InvalidIndex {
                kind: "method",
                index: method_idx,
                limit: self.dex.methods().len(),
            })?;
        let proto =
            self.dex.protos().get(usize::from(method.proto_idx)).ok_or(DexError::InvalidIndex {
                kind: "prototype",
                index: u32::from(method.proto_idx),
                limit: self.dex.protos().len(),
            })?;
        let mut values = Vec::with_capacity(proto.parameters.len() + usize::from(!is_static));
        let mut position = 0;
        if !is_static {
            let register = *registers.get(position).ok_or(VmError::InvalidArguments)?;
            values.push(frame.read(register)?);
            position += 1;
        }
        for type_idx in &proto.parameters {
            let register = *registers.get(position).ok_or(VmError::InvalidArguments)?;
            values.push(frame.read(register)?);
            position += descriptor_width(self.dex.type_descriptor(*type_idx)?);
        }
        if position != registers.len() {
            return Err(VmError::InvalidArguments);
        }
        Ok(values)
    }

    fn place_arguments(
        &self,
        method_idx: u32,
        code: &CodeItem,
        arguments: &[Value],
        frame: &mut Frame,
    ) -> Result<(), VmError> {
        let method =
            self.dex.methods().get(usize_from_u32(method_idx)?).ok_or(DexError::InvalidIndex {
                kind: "method",
                index: method_idx,
                limit: self.dex.methods().len(),
            })?;
        let proto =
            self.dex.protos().get(usize::from(method.proto_idx)).ok_or(DexError::InvalidIndex {
                kind: "prototype",
                index: u32::from(method.proto_idx),
                limit: self.dex.protos().len(),
            })?;
        let is_static = self
            .dex
            .encoded_method(method_idx)
            .is_some_and(|encoded| encoded.access_flags & ACC_STATIC != 0);
        let expected = proto.parameters.len() + usize::from(!is_static);
        if arguments.len() != expected {
            return Err(VmError::InvalidArguments);
        }
        let mut register = frame
            .registers
            .len()
            .checked_sub(usize::from(code.ins_size))
            .ok_or(VmError::InvalidArguments)?;
        let mut argument = 0;
        if !is_static {
            frame.write(register, arguments[argument].clone())?;
            register += 1;
            argument += 1;
        }
        for type_idx in &proto.parameters {
            frame.write(register, arguments[argument].clone())?;
            register += descriptor_width(self.dex.type_descriptor(*type_idx)?);
            argument += 1;
        }
        if register != frame.registers.len() {
            return Err(VmError::InvalidArguments);
        }
        Ok(())
    }

    fn ensure_class_initialized(
        &mut self,
        class_idx: u32,
        budget: &mut Budget,
        depth: usize,
    ) -> Result<(), VmError> {
        if self.initialized_classes.contains(&class_idx)
            || self.initializing_classes.contains(&class_idx)
        {
            return Ok(());
        }
        let Some(class) = self.dex.class_by_type(class_idx).cloned() else {
            return Ok(());
        };
        self.initializing_classes.insert(class_idx);
        let result = (|| {
            if let Some(superclass) = class.superclass_idx {
                self.ensure_class_initialized(superclass, budget, depth + 1)?;
            }
            let initializer = class.direct_methods.iter().find_map(|encoded| {
                let method = self.dex.methods().get(usize_from_u32(encoded.method_idx).ok()?)?;
                (self.dex.string(method.name_idx).ok()? == "<clinit>"
                    && self.dex.prototype_descriptor(u32::from(method.proto_idx)).ok()? == "()V")
                    .then_some(encoded.method_idx)
            });
            if let Some(method_idx) = initializer {
                self.execute_method(method_idx, &[], budget, depth + 1)?;
            }
            Ok(())
        })();
        self.initializing_classes.remove(&class_idx);
        if result.is_ok() {
            self.initialized_classes.insert(class_idx);
        }
        result
    }

    fn allocate_heap(&mut self, entry: HeapEntry) -> Result<u32, VmError> {
        if self.heap.len() >= self.limits.max_heap_entries {
            return Err(VmError::HeapLimit);
        }
        let reference = u32::try_from(self.heap.len()).map_err(|_| VmError::HeapLimit)?;
        self.heap.push(entry);
        Ok(reference)
    }

    fn allocate_object_type(&mut self, class_idx: u32) -> Result<ObjectRef, VmError> {
        self.dex.type_descriptor(class_idx)?;
        let reference =
            self.allocate_heap(HeapEntry::Object { class_idx, fields: HashMap::new() })?;
        Ok(ObjectRef(reference))
    }

    fn allocate_array_type(&mut self, type_idx: u32, length: usize) -> Result<ArrayRef, VmError> {
        let descriptor = self.dex.type_descriptor(type_idx)?;
        if !descriptor.starts_with('[') {
            return Err(VmError::TypeMismatch("new-array type is not an array"));
        }
        if length > self.limits.max_array_length {
            return Err(VmError::ArrayLimit);
        }
        let default = default_value(&descriptor[1..]);
        let reference =
            self.allocate_heap(HeapEntry::Array { type_idx, elements: vec![default; length] })?;
        Ok(ArrayRef(reference))
    }

    fn heap_entry(&self, reference: u32) -> Result<&HeapEntry, VmError> {
        self.heap.get(usize_from_u32(reference)?).ok_or(VmError::InvalidReference)
    }

    fn heap_entry_mut(&mut self, reference: u32) -> Result<&mut HeapEntry, VmError> {
        self.heap.get_mut(usize_from_u32(reference)?).ok_or(VmError::InvalidReference)
    }

    fn object_class(&self, reference: ObjectRef) -> Result<u32, VmError> {
        match self.heap_entry(reference.0)? {
            HeapEntry::Object { class_idx, .. } | HeapEntry::Class(class_idx) => Ok(*class_idx),
            HeapEntry::String(_) => self
                .dex
                .types()
                .iter()
                .enumerate()
                .find_map(|(index, _)| {
                    let index = u32::try_from(index).ok()?;
                    (self.dex.type_descriptor(index).ok()? == "Ljava/lang/String;").then_some(index)
                })
                .ok_or(VmError::TypeMismatch("java.lang.String type is absent from this DEX")),
            HeapEntry::Array { .. } => Err(VmError::TypeMismatch("array is not an object here")),
        }
    }

    fn value_is_type(&self, value: &Value, expected: u32) -> Result<bool, VmError> {
        match value {
            Value::Null => Ok(false),
            Value::Reference(HeapRef::Object(reference)) => {
                let actual = self.object_class(*reference)?;
                Ok(self.is_assignable(actual, expected))
            }
            Value::Reference(HeapRef::Array(reference)) => {
                let HeapEntry::Array { type_idx, .. } = self.heap_entry(reference.0)? else {
                    return Err(VmError::InvalidReference);
                };
                Ok(*type_idx == expected
                    || self.dex.type_descriptor(expected)? == "Ljava/lang/Object;")
            }
            _ => Err(VmError::TypeMismatch("instance operation on primitive value")),
        }
    }

    fn is_assignable(&self, mut actual: u32, expected: u32) -> bool {
        loop {
            if actual == expected {
                return true;
            }
            let Some(class) = self.dex.class_by_type(actual) else {
                return false;
            };
            if class.interfaces.contains(&expected) {
                return true;
            }
            let Some(superclass) = class.superclass_idx else {
                return false;
            };
            actual = superclass;
        }
    }

    fn default_field_value(&self, field_idx: u32) -> Result<Value, VmError> {
        let field = self.dex.resolve_field(field_idx)?;
        Ok(default_value(&field.type_descriptor))
    }

    fn catch_target(
        &self,
        code: &CodeItem,
        pc: usize,
        thrown: &Value,
    ) -> Result<Option<usize>, VmError> {
        let pc = u32::try_from(pc).map_err(|_| VmError::InvalidBranch)?;
        let Some(item) = code.tries.iter().find(|item| {
            pc >= item.start_addr
                && pc < item.start_addr.saturating_add(u32::from(item.instruction_count))
        }) else {
            return Ok(None);
        };
        let handler = code
            .handlers
            .iter()
            .find(|handler| handler.offset == u32::from(item.handler_off))
            .ok_or_else(|| {
                malformed(usize_from_u32(pc).unwrap_or(usize::MAX), "missing handler")
            })?;
        for (type_idx, address) in &handler.catches {
            if self.value_is_type(thrown, *type_idx).unwrap_or(false) {
                return usize_from_u32(*address).map(Some);
            }
        }
        handler.catch_all_addr.map_or(Ok(None), |address| usize_from_u32(address).map(Some))
    }

    fn fill_array_data(
        &mut self,
        code: &CodeItem,
        target: usize,
        reference: ArrayRef,
    ) -> Result<(), VmError> {
        if code_unit(code, target)? != 0x0300 {
            return Err(malformed(target, "invalid fill-array-data payload"));
        }
        let element_width = usize::from(code_unit(code, target + 1)?);
        let count = usize_from_u32(read_u32(code, target + 2)?)?;
        let HeapEntry::Array { elements, .. } = self.heap_entry_mut(reference.0)? else {
            return Err(VmError::InvalidReference);
        };
        if count > elements.len() {
            return Err(VmError::ArrayBounds {
                index: i32::try_from(count).unwrap_or(i32::MAX),
                length: elements.len(),
            });
        }
        let data_start = target + 4;
        let bytes = code_units_as_bytes(&code.instructions);
        let byte_start = data_start.checked_mul(2).ok_or(VmError::InvalidBranch)?;
        let byte_count = count.checked_mul(element_width).ok_or(VmError::ArrayLimit)?;
        let data = bytes
            .get(byte_start..byte_start + byte_count)
            .ok_or_else(|| malformed(target, "truncated fill-array-data payload"))?;
        for (index, chunk) in data.chunks_exact(element_width).enumerate() {
            elements[index] = match element_width {
                1 => Value::Int(i32::from(chunk[0] as i8)),
                2 => Value::Int(i32::from(i16::from_le_bytes([chunk[0], chunk[1]]))),
                4 => Value::Int(i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])),
                8 => Value::Long(i64::from_le_bytes([
                    chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7],
                ])),
                _ => return Err(malformed(target, "unsupported array element width")),
            };
        }
        Ok(())
    }

    fn switch_target(
        code: &CodeItem,
        switch_pc: usize,
        payload: usize,
        key: i32,
        opcode: u8,
    ) -> Result<usize, VmError> {
        let ident = code_unit(code, payload)?;
        let count = usize::from(code_unit(code, payload + 1)?);
        let relative = if opcode == 0x2b {
            if ident != 0x0100 {
                return Err(malformed(payload, "invalid packed-switch payload"));
            }
            let first_key = read_i32(code, payload + 2)?;
            let delta = key.wrapping_sub(first_key);
            let index = usize::try_from(delta).ok().filter(|value| *value < count);
            index.map(|value| read_i32(code, payload + 4 + value * 2))
        } else {
            if ident != 0x0200 {
                return Err(malformed(payload, "invalid sparse-switch payload"));
            }
            let mut found = None;
            for index in 0..count {
                if read_i32(code, payload + 2 + index * 2)? == key {
                    found = Some(read_i32(code, payload + 2 + count * 2 + index * 2));
                    break;
                }
            }
            found
        };
        match relative {
            Some(offset) => branch_target(code, switch_pc, offset?),
            None => Ok(switch_pc + 3),
        }
    }
}

fn code_unit(code: &CodeItem, pc: usize) -> Result<u16, VmError> {
    code.instructions.get(pc).copied().ok_or_else(|| malformed(pc, "instruction is truncated"))
}

fn malformed(pc: usize, reason: &str) -> VmError {
    VmError::MalformedInstruction { pc, reason: reason.to_owned() }
}

fn read_u32(code: &CodeItem, pc: usize) -> Result<u32, VmError> {
    Ok(u32::from(code_unit(code, pc)?) | (u32::from(code_unit(code, pc + 1)?) << 16))
}

fn read_i32(code: &CodeItem, pc: usize) -> Result<i32, VmError> {
    Ok(read_u32(code, pc)? as i32)
}

fn read_i64(code: &CodeItem, pc: usize) -> Result<i64, VmError> {
    let low = u64::from(read_u32(code, pc)?);
    let high = u64::from(read_u32(code, pc + 2)?);
    Ok((low | (high << 32)) as i64)
}

fn branch_target(code: &CodeItem, pc: usize, relative: i32) -> Result<usize, VmError> {
    let pc = i64::try_from(pc).map_err(|_| VmError::InvalidBranch)?;
    let target = pc.checked_add(i64::from(relative)).ok_or(VmError::InvalidBranch)?;
    let target = usize::try_from(target).map_err(|_| VmError::InvalidBranch)?;
    if target >= code.instructions.len() {
        return Err(VmError::InvalidBranch);
    }
    Ok(target)
}

fn decode_invoke_registers(
    code: &CodeItem,
    pc: usize,
    opcode: u8,
) -> Result<(u32, Vec<usize>, usize), VmError> {
    let first = code_unit(code, pc)?;
    let index = u32::from(code_unit(code, pc + 1)?);
    if matches!(opcode, 0x25 | 0x74..=0x78) {
        let count = usize::from(first >> 8);
        let start = usize::from(code_unit(code, pc + 2)?);
        let end =
            start.checked_add(count).ok_or_else(|| malformed(pc, "register range overflows"))?;
        Ok((index, (start..end).collect(), 3))
    } else {
        let count = usize::from(first >> 12);
        if count > 5 {
            return Err(malformed(pc, "35c register count exceeds five"));
        }
        let fifth = usize::from((first >> 8) & 0x0f);
        let packed = code_unit(code, pc + 2)?;
        let mut registers = [
            usize::from(packed & 0x0f),
            usize::from((packed >> 4) & 0x0f),
            usize::from((packed >> 8) & 0x0f),
            usize::from((packed >> 12) & 0x0f),
            fifth,
        ]
        .to_vec();
        registers.truncate(count);
        Ok((index, registers, 3))
    }
}

fn execute_compare(code: &CodeItem, frame: &mut Frame, opcode: u8) -> Result<(), VmError> {
    let instruction = code_unit(code, frame.pc)?;
    let destination = usize::from(instruction >> 8);
    let registers = code_unit(code, frame.pc + 1)?;
    let left = frame.read(usize::from(registers & 0xff))?;
    let right = frame.read(usize::from(registers >> 8))?;
    let ordering = match opcode {
        0x2d => float_compare(&left.float()?, &right.float()?, -1),
        0x2e => float_compare(&left.float()?, &right.float()?, 1),
        0x2f => float_compare(&left.double()?, &right.double()?, -1),
        0x30 => float_compare(&left.double()?, &right.double()?, 1),
        0x31 => left.long()?.cmp(&right.long()?) as i32,
        _ => unreachable!(),
    };
    frame.write(destination, Value::Int(ordering))?;
    frame.pc += 2;
    Ok(())
}

fn float_compare<T>(left: &T, right: &T, nan_result: i32) -> i32
where
    T: PartialOrd,
{
    left.partial_cmp(right).map_or(nan_result, |ordering| match ordering {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    })
}

fn compare_branch(opcode: u8, left: &Value, right: &Value) -> Result<bool, VmError> {
    if matches!(
        (left, right),
        (Value::Null | Value::Reference(_), Value::Null | Value::Reference(_))
    ) {
        let equal = match (left, right) {
            (Value::Null, Value::Null) => true,
            (Value::Reference(left), Value::Reference(right)) => left == right,
            _ => false,
        };
        return match opcode {
            0x32 => Ok(equal),
            0x33 => Ok(!equal),
            _ => Err(VmError::TypeMismatch("ordered comparison of object references")),
        };
    }
    let left = left.int()?;
    let right = right.int()?;
    Ok(match opcode {
        0x32 => left == right,
        0x33 => left != right,
        0x34 => left < right,
        0x35 => left >= right,
        0x36 => left > right,
        0x37 => left <= right,
        _ => false,
    })
}

fn compare_zero_branch(opcode: u8, value: &Value) -> Result<bool, VmError> {
    if matches!(value, Value::Null | Value::Reference(_)) {
        let is_null = matches!(value, Value::Null);
        return match opcode {
            0x38 => Ok(is_null),
            0x39 => Ok(!is_null),
            _ => Err(VmError::TypeMismatch("ordered comparison of object reference")),
        };
    }
    let value = value.int()?;
    Ok(match opcode {
        0x38 => value == 0,
        0x39 => value != 0,
        0x3a => value < 0,
        0x3b => value >= 0,
        0x3c => value > 0,
        0x3d => value <= 0,
        _ => false,
    })
}

fn execute_conversion(code: &CodeItem, frame: &mut Frame, opcode: u8) -> Result<(), VmError> {
    let encoded = code_unit(code, frame.pc)?;
    let destination = usize::from((encoded >> 8) & 0x0f);
    let source = frame.read(usize::from(encoded >> 12))?;
    let converted = match opcode {
        0x7b => Value::Int(source.int()?.wrapping_neg()),
        0x7c => Value::Int(!source.int()?),
        0x7d => Value::Long(source.long()?.wrapping_neg()),
        0x7e => Value::Long(!source.long()?),
        0x7f => Value::Float(-source.float()?),
        0x80 => Value::Double(-source.double()?),
        0x81 => Value::Long(i64::from(source.int()?)),
        0x82 => Value::Float(source.int()? as f32),
        0x83 => Value::Double(f64::from(source.int()?)),
        0x84 => Value::Int(source.long()? as i32),
        0x85 => Value::Float(source.long()? as f32),
        0x86 => Value::Double(source.long()? as f64),
        0x87 => Value::Int(source.float()? as i32),
        0x88 => Value::Long(source.float()? as i64),
        0x89 => Value::Double(f64::from(source.float()?)),
        0x8a => Value::Int(source.double()? as i32),
        0x8b => Value::Long(source.double()? as i64),
        0x8c => Value::Float(source.double()? as f32),
        0x8d => Value::Int(i32::from(source.int()? as i8)),
        0x8e => Value::Int(i32::from(source.int()? as u16)),
        0x8f => Value::Int(i32::from(source.int()? as i16)),
        _ => return Err(VmError::UnsupportedOpcode { opcode, pc: frame.pc }),
    };
    frame.write(destination, converted)?;
    frame.pc += 1;
    Ok(())
}

fn execute_binary(code: &CodeItem, frame: &mut Frame, opcode: u8) -> Result<(), VmError> {
    let first = code_unit(code, frame.pc)?;
    let destination = usize::from(first >> 8);
    let sources = code_unit(code, frame.pc + 1)?;
    let left = frame.read(usize::from(sources & 0xff))?;
    let right = frame.read(usize::from(sources >> 8))?;
    frame.write(destination, binary_value(opcode, &left, &right)?)?;
    frame.pc += 2;
    Ok(())
}

fn execute_binary_2addr(code: &CodeItem, frame: &mut Frame, opcode: u8) -> Result<(), VmError> {
    let encoded = code_unit(code, frame.pc)?;
    let destination = usize::from((encoded >> 8) & 0x0f);
    let right = usize::from(encoded >> 12);
    let value = binary_value(opcode - 0x20, &frame.read(destination)?, &frame.read(right)?)?;
    frame.write(destination, value)?;
    frame.pc += 1;
    Ok(())
}

fn binary_value(opcode: u8, left: &Value, right: &Value) -> Result<Value, VmError> {
    match opcode {
        0x90 => Ok(Value::Int(left.int()?.wrapping_add(right.int()?))),
        0x91 => Ok(Value::Int(left.int()?.wrapping_sub(right.int()?))),
        0x92 => Ok(Value::Int(left.int()?.wrapping_mul(right.int()?))),
        0x93 => Ok(Value::Int(divide_i32(left.int()?, right.int()?)?)),
        0x94 => Ok(Value::Int(remainder_i32(left.int()?, right.int()?)?)),
        0x95 => Ok(Value::Int(left.int()? & right.int()?)),
        0x96 => Ok(Value::Int(left.int()? | right.int()?)),
        0x97 => Ok(Value::Int(left.int()? ^ right.int()?)),
        0x98 => Ok(Value::Int(left.int()?.wrapping_shl((right.int()? & 0x1f) as u32))),
        0x99 => Ok(Value::Int(left.int()?.wrapping_shr((right.int()? & 0x1f) as u32))),
        0x9a => Ok(Value::Int(((left.int()? as u32) >> ((right.int()? & 0x1f) as u32)) as i32)),
        0x9b => Ok(Value::Long(left.long()?.wrapping_add(right.long()?))),
        0x9c => Ok(Value::Long(left.long()?.wrapping_sub(right.long()?))),
        0x9d => Ok(Value::Long(left.long()?.wrapping_mul(right.long()?))),
        0x9e => Ok(Value::Long(divide_i64(left.long()?, right.long()?)?)),
        0x9f => Ok(Value::Long(remainder_i64(left.long()?, right.long()?)?)),
        0xa0 => Ok(Value::Long(left.long()? & right.long()?)),
        0xa1 => Ok(Value::Long(left.long()? | right.long()?)),
        0xa2 => Ok(Value::Long(left.long()? ^ right.long()?)),
        0xa3 => Ok(Value::Long(left.long()?.wrapping_shl((right.int()? & 0x3f) as u32))),
        0xa4 => Ok(Value::Long(left.long()?.wrapping_shr((right.int()? & 0x3f) as u32))),
        0xa5 => Ok(Value::Long(((left.long()? as u64) >> ((right.int()? & 0x3f) as u32)) as i64)),
        0xa6 => Ok(Value::Float(left.float()? + right.float()?)),
        0xa7 => Ok(Value::Float(left.float()? - right.float()?)),
        0xa8 => Ok(Value::Float(left.float()? * right.float()?)),
        0xa9 => Ok(Value::Float(left.float()? / right.float()?)),
        0xaa => Ok(Value::Float(left.float()? % right.float()?)),
        0xab => Ok(Value::Double(left.double()? + right.double()?)),
        0xac => Ok(Value::Double(left.double()? - right.double()?)),
        0xad => Ok(Value::Double(left.double()? * right.double()?)),
        0xae => Ok(Value::Double(left.double()? / right.double()?)),
        0xaf => Ok(Value::Double(left.double()? % right.double()?)),
        _ => Err(VmError::UnsupportedOpcode { opcode, pc: 0 }),
    }
}

fn execute_literal16(code: &CodeItem, frame: &mut Frame, opcode: u8) -> Result<(), VmError> {
    let first = code_unit(code, frame.pc)?;
    let destination = usize::from((first >> 8) & 0x0f);
    let source = usize::from(first >> 12);
    let left = frame.read(source)?.int()?;
    let literal = i32::from(code_unit(code, frame.pc + 1)? as i16);
    let result = literal_binary(opcode, left, literal)?;
    frame.write(destination, Value::Int(result))?;
    frame.pc += 2;
    Ok(())
}

fn execute_literal8(code: &CodeItem, frame: &mut Frame, opcode: u8) -> Result<(), VmError> {
    let first = code_unit(code, frame.pc)?;
    let destination = usize::from(first >> 8);
    let second = code_unit(code, frame.pc + 1)?;
    let source = usize::from(second & 0xff);
    let literal = i32::from((second >> 8) as u8 as i8);
    let left = frame.read(source)?.int()?;
    let result = literal_binary(opcode, left, literal)?;
    frame.write(destination, Value::Int(result))?;
    frame.pc += 2;
    Ok(())
}

fn literal_binary(opcode: u8, left: i32, literal: i32) -> Result<i32, VmError> {
    Ok(match opcode {
        0xd0 | 0xd8 => left.wrapping_add(literal),
        0xd1 | 0xd9 => literal.wrapping_sub(left),
        0xd2 | 0xda => left.wrapping_mul(literal),
        0xd3 | 0xdb => divide_i32(left, literal)?,
        0xd4 | 0xdc => remainder_i32(left, literal)?,
        0xd5 | 0xdd => left & literal,
        0xd6 | 0xde => left | literal,
        0xd7 | 0xdf => left ^ literal,
        0xe0 => left.wrapping_shl((literal & 0x1f) as u32),
        0xe1 => left.wrapping_shr((literal & 0x1f) as u32),
        0xe2 => ((left as u32) >> ((literal & 0x1f) as u32)) as i32,
        _ => return Err(VmError::UnsupportedOpcode { opcode, pc: 0 }),
    })
}

fn divide_i32(left: i32, right: i32) -> Result<i32, VmError> {
    if right == 0 {
        Err(VmError::DivisionByZero)
    } else if left == i32::MIN && right == -1 {
        Ok(i32::MIN)
    } else {
        Ok(left / right)
    }
}

fn remainder_i32(left: i32, right: i32) -> Result<i32, VmError> {
    if right == 0 {
        Err(VmError::DivisionByZero)
    } else if left == i32::MIN && right == -1 {
        Ok(0)
    } else {
        Ok(left % right)
    }
}

fn divide_i64(left: i64, right: i64) -> Result<i64, VmError> {
    if right == 0 {
        Err(VmError::DivisionByZero)
    } else if left == i64::MIN && right == -1 {
        Ok(i64::MIN)
    } else {
        Ok(left / right)
    }
}

fn remainder_i64(left: i64, right: i64) -> Result<i64, VmError> {
    if right == 0 {
        Err(VmError::DivisionByZero)
    } else if left == i64::MIN && right == -1 {
        Ok(0)
    } else {
        Ok(left % right)
    }
}

fn descriptor_width(descriptor: &str) -> usize {
    usize::from(matches!(descriptor.as_bytes().first(), Some(b'J' | b'D'))) + 1
}

fn default_value(descriptor: &str) -> Value {
    match descriptor.as_bytes().first() {
        Some(b'J') => Value::Long(0),
        Some(b'F') => Value::Float(0.0),
        Some(b'D') => Value::Double(0.0),
        Some(b'L' | b'[') => Value::Null,
        _ => Value::Int(0),
    }
}

fn checked_array_index(index: i32, length: usize) -> Result<usize, VmError> {
    let converted = usize::try_from(index).map_err(|_| VmError::ArrayBounds { index, length })?;
    if converted >= length {
        return Err(VmError::ArrayBounds { index, length });
    }
    Ok(converted)
}

fn usize_from_u32(value: u32) -> Result<usize, VmError> {
    usize::try_from(value).map_err(|_| VmError::InvalidReference)
}

fn code_units_as_bytes(units: &[u16]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(units.len() * 2);
    for unit in units {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}
