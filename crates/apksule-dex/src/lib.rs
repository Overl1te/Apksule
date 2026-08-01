#![forbid(unsafe_code)]

//! Безопасный минимальный разборщик DEX и регистровая виртуальная машина.

mod parser;
mod vm;

pub use parser::{
    CatchHandler, ClassDef, CodeItem, DexError, DexFile, DexHeader, EncodedField, EncodedMethod,
    FieldId, MapItem, MethodHandle, MethodId, ProtoId, ResolvedField, ResolvedMethod, TryItem,
    TypeId,
};
pub use vm::{
    ArrayRef, HeapRef, NativeBridge, NativeResult, ObjectRef, Value, Vm, VmError, VmLimits,
};

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;
