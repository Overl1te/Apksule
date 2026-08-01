use std::time::Duration;

use crate::test_support::{build_test_dex, refresh_digests};
use crate::{
    DexError, DexFile, NativeBridge, NativeResult, ResolvedMethod, Value, Vm, VmError, VmLimits,
};

#[test]
fn parses_generated_dex_and_resolves_symbols() {
    let bytes = build_test_dex();
    let dex = DexFile::parse(bytes.clone()).expect("generated DEX must parse");

    assert_eq!(dex.bytes(), bytes);
    assert_eq!(dex.header().header_size, 0x70);
    assert_eq!(dex.string(0).unwrap(), "LTest;");
    assert_eq!(dex.type_descriptor(4).unwrap(), "[I");
    assert_eq!(dex.prototype_descriptor(1).unwrap(), "(I)I");
    assert!(dex.find_class("LTest;").is_some());
    let method = dex.find_method("LTest;", "add", Some("()I")).expect("method must resolve");
    assert!(method.encoded.is_some());
    assert!(dex.method_code(method.index).is_some());
    assert_eq!(dex.resolve_field(1).unwrap().name, "value");
}

#[test]
fn rejects_bad_checksum() {
    let mut bytes = build_test_dex();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    assert!(matches!(DexFile::parse(bytes), Err(DexError::ChecksumMismatch { .. })));
}

#[test]
fn rejects_malformed_section_bounds_after_valid_digest() {
    let mut bytes = build_test_dex();
    bytes[56..60].copy_from_slice(&u32::MAX.to_le_bytes());
    refresh_digests(&mut bytes);
    assert!(matches!(DexFile::parse(bytes), Err(DexError::Bounds { section: "string_ids", .. })));
}

#[test]
fn rejects_unterminated_uleb128() {
    let mut bytes = build_test_dex();
    let offset = u32::from_le_bytes(bytes[0x70..0x74].try_into().unwrap());
    let offset = usize::try_from(offset).unwrap();
    bytes[offset..offset + 5].fill(0x80);
    refresh_digests(&mut bytes);
    assert!(matches!(DexFile::parse(bytes), Err(DexError::InvalidLeb128 { .. })));
}

#[test]
fn executes_constants_arithmetic_and_class_initializer() {
    let dex = DexFile::parse(build_test_dex()).unwrap();
    let mut vm = Vm::new(&dex);

    assert_eq!(vm.invoke("LTest;", "add", "()I", &[]).unwrap(), Value::Int(5));
    assert_eq!(vm.invoke("LTest;", "staticField", "()I", &[]).unwrap(), Value::Int(6));
    assert_eq!(vm.invoke("LTest;", "convert", "()I", &[]).unwrap(), Value::Int(-126));
}

#[test]
fn executes_conditional_branch() {
    let dex = DexFile::parse(build_test_dex()).unwrap();
    let mut vm = Vm::new(&dex);

    assert_eq!(vm.invoke("LTest;", "branch", "(I)I", &[Value::Int(3)]).unwrap(), Value::Int(1));
    assert_eq!(vm.invoke("LTest;", "branch", "(I)I", &[Value::Int(0)]).unwrap(), Value::Int(2));
}

#[test]
fn executes_array_and_instance_field_access() {
    let dex = DexFile::parse(build_test_dex()).unwrap();
    let mut vm = Vm::new(&dex);

    assert_eq!(vm.invoke("LTest;", "array", "()I", &[]).unwrap(), Value::Int(7));
    assert_eq!(vm.invoke("LTest;", "object", "()I", &[]).unwrap(), Value::Int(9));
}

struct DoublingBridge;

impl NativeBridge for DoublingBridge {
    fn invoke(
        &mut self,
        method: &ResolvedMethod,
        arguments: &[Value],
    ) -> Result<NativeResult, VmError> {
        if method.class_descriptor == "LNative;" && method.name == "double" {
            let [Value::Int(value)] = arguments else {
                return Err(VmError::InvalidArguments);
            };
            return Ok(NativeResult::Handled(Value::Int(value * 2)));
        }
        Ok(NativeResult::Unresolved)
    }
}

#[test]
fn invokes_native_bridge_and_reports_unresolved_methods() {
    let dex = DexFile::parse(build_test_dex()).unwrap();
    let mut vm = Vm::with_native_bridge(&dex, DoublingBridge);
    assert_eq!(vm.invoke("LTest;", "callNative", "()I", &[]).unwrap(), Value::Int(8));

    let mut vm = Vm::new(&dex);
    assert!(matches!(
        vm.invoke("LNative;", "double", "(I)I", &[Value::Int(2)]),
        Err(VmError::UnresolvedNative(_))
    ));
}

#[test]
fn catches_explicit_throw() {
    let dex = DexFile::parse(build_test_dex()).unwrap();
    let mut vm = Vm::new(&dex);
    assert_eq!(vm.invoke("LTest;", "caught", "()I", &[]).unwrap(), Value::Int(5));
}

#[test]
fn enforces_instruction_and_heap_limits() {
    let dex = DexFile::parse(build_test_dex()).unwrap();
    let mut vm = Vm::new(&dex);
    vm.set_limits(VmLimits {
        max_instructions: 10,
        max_duration: Duration::from_secs(1),
        ..VmLimits::default()
    });
    assert_eq!(vm.invoke("LTest;", "loop", "()V", &[]), Err(VmError::InstructionLimit));

    let mut vm = Vm::new(&dex);
    vm.set_limits(VmLimits { max_heap_entries: 0, ..VmLimits::default() });
    assert_eq!(vm.invoke("LTest;", "object", "()I", &[]), Err(VmError::HeapLimit));
}
