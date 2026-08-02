//! Built-in Java/Android core state and helpers for the DEX VM.

#![allow(clippy::too_many_lines)]

use std::collections::HashMap;

use crate::{HeapRef, ObjectRef, Value};

#[derive(Debug, Default)]
pub struct JavaRuntime {
    pub builders: HashMap<u32, String>,
    pub array_lists: HashMap<u32, Vec<Value>>,
    pub hash_maps: HashMap<u32, Vec<(ValueKey, Value)>>,
    pub array_deques: HashMap<u32, Vec<Value>>,
    pub atomic_ints: HashMap<u32, i32>,
    pub atomic_refs: HashMap<u32, Value>,
    pub weak_refs: HashMap<u32, Value>,
    pub enum_meta: HashMap<u32, (String, i32)>,
    pub main_looper: Option<ObjectRef>,
    pub main_thread: Option<ObjectRef>,
    pub posted: Vec<ObjectRef>,
}

/// Hashable wrapper for map keys.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ValueKey {
    Null,
    Int(i32),
    Long(i64),
    Ref(u32),
    Other,
}

impl ValueKey {
    #[must_use]
    pub fn from_value(value: &Value) -> Self {
        match value {
            Value::Null => Self::Null,
            Value::Int(v) => Self::Int(*v),
            Value::Long(v) => Self::Long(*v),
            Value::Reference(HeapRef::Object(ObjectRef(id)) | HeapRef::Array(crate::ArrayRef(id))) => {
                Self::Ref(*id)
            }
            _ => Self::Other,
        }
    }
}

#[must_use]
pub fn java_string_hash(text: &str) -> i32 {
    let mut hash = 0i32;
    for ch in text.chars() {
        hash = hash.wrapping_mul(31).wrapping_add(ch as i32);
    }
    hash
}

pub fn value_as_int(value: &Value) -> Option<i32> {
    match value {
        Value::Int(value) => Some(*value),
        _ => None,
    }
}

pub fn receiver_id(arguments: &[Value]) -> Option<u32> {
    match arguments.first() {
        Some(Value::Reference(HeapRef::Object(ObjectRef(id)))) => Some(*id),
        _ => None,
    }
}

pub fn is_java_builtin(class: &str) -> bool {
    matches!(
        class,
        "Ljava/lang/String;"
            | "Ljava/lang/StringBuilder;"
            | "Ljava/lang/StringBuffer;"
            | "Ljava/util/ArrayList;"
            | "Ljava/util/HashMap;"
            | "Ljava/util/WeakHashMap;"
            | "Ljava/util/ArrayDeque;"
            | "Ljava/util/concurrent/CopyOnWriteArraySet;"
            | "Ljava/util/concurrent/atomic/AtomicInteger;"
            | "Ljava/util/concurrent/atomic/AtomicReference;"
            | "Ljava/lang/ref/WeakReference;"
            | "Ljava/lang/Enum;"
            | "Ljava/lang/Thread;"
            | "Landroid/os/Looper;"
            | "Ljava/lang/Class;"
            | "Ljava/lang/System;"
            | "Ljava/lang/Math;"
            | "Ljava/util/Objects;"
            | "Ljava/util/Random;"
            | "Ljava/util/concurrent/Executors;"
            | "Ljava/util/concurrent/ExecutorService;"
            | "Ljava/util/concurrent/Executor;"
            | "Landroid/os/Handler;"
            | "Landroid/os/Bundle;"
    )
}
