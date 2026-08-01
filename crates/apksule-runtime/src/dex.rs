use apksule_apk::ApkPackage;
use apksule_compat::{Context, InputEvent};
use apksule_dex::{
    DexFile, HeapRef, NativeBridge, NativeResult, ObjectRef, ResolvedMethod, Value, Vm, VmError,
};
use thiserror::Error;

use crate::lifecycle::ActivityState;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum DexStatus {
    #[default]
    NotLoaded,
    Ready {
        dex_files: usize,
        classes: usize,
    },
    Running {
        activity: String,
        method: String,
    },
    Failed {
        reason: String,
    },
    Unsupported {
        dex_files: usize,
        reason: String,
    },
}

#[derive(Debug, Error)]
pub enum DexError {
    #[error("compatibility layer failed while handling DEX: {0}")]
    Compat(#[from] apksule_compat::CompatError),
    #[error("failed to read DEX from APK: {0}")]
    Apk(String),
    #[error("DEX parser failed: {0}")]
    Parse(#[from] apksule_dex::DexError),
    #[error("DEX interpreter failed: {0}")]
    Vm(#[from] VmError),
    #[error("DEX runtime failed: {0}")]
    Runtime(String),
}

/// Stable seam for the M2 interpreter. The host never depends on this trait.
pub trait DexRuntime {
    fn load(&mut self, package: &ApkPackage, context: &Context) -> Result<(), DexError>;
    fn on_lifecycle(&mut self, state: ActivityState) -> Result<(), DexError>;
    fn on_input(&mut self, event: &InputEvent) -> Result<(), DexError>;
    fn on_surface_changed(&mut self, width: u32, height: u32) -> Result<(), DexError>;
    fn status(&self) -> &DexStatus;
}

#[derive(Default)]
pub struct InterpretingDexRuntime {
    status: DexStatus,
    vm: Option<Vm>,
    context: Option<Context>,
    activity_descriptor: Option<String>,
    activity_object: Option<ObjectRef>,
    input_events_seen: u64,
}

impl InterpretingDexRuntime {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn fail(
        &mut self,
        context: &Context,
        operation: &str,
        reason: impl Into<String>,
    ) -> Result<(), DexError> {
        let reason = reason.into();
        tracing::warn!(%reason, %operation, "DEX interpreter stopped");
        self.status = DexStatus::Failed { reason: reason.clone() };
        context.unsupported_api("dalvik.system.DexFile", operation, reason)?;
        Ok(())
    }

    fn invoke_activity_method(
        &mut self,
        state: ActivityState,
        method: &str,
    ) -> Result<(), DexError> {
        let (Some(descriptor), Some(object), Some(context)) =
            (self.activity_descriptor.clone(), self.activity_object, self.context.clone())
        else {
            return Ok(());
        };

        let prototype = self.vm.as_ref().and_then(|vm| {
            let dex = vm.dex();
            if state == ActivityState::Created
                && dex.find_method(&descriptor, method, Some("(Landroid/os/Bundle;)V")).is_some()
            {
                Some("(Landroid/os/Bundle;)V")
            } else if dex.find_method(&descriptor, method, Some("()V")).is_some() {
                Some("()V")
            } else {
                None
            }
        });
        let Some(prototype) = prototype else {
            if state == ActivityState::Created {
                self.fail(
                    &context,
                    method,
                    format!("{descriptor}->{method}(Landroid/os/Bundle;)V не найден"),
                )?;
            }
            return Ok(());
        };

        let receiver = Value::Reference(HeapRef::Object(object));
        let arguments =
            if prototype == "()V" { vec![receiver] } else { vec![receiver, Value::Null] };
        let result = self
            .vm
            .as_mut()
            .ok_or_else(|| DexError::Runtime("VM не загружена".to_owned()))?
            .invoke(&descriptor, method, prototype, &arguments);
        match result {
            Ok(_) => {
                self.status = DexStatus::Running {
                    activity: descriptor_name(&descriptor),
                    method: format!("{method}()"),
                };
                tracing::info!(activity = %descriptor, %method, ?state, "DEX method executed");
            }
            Err(error) => {
                self.fail(&context, method, error.to_string())?;
            }
        }
        Ok(())
    }
}

impl DexRuntime for InterpretingDexRuntime {
    fn load(&mut self, package: &ApkPackage, context: &Context) -> Result<(), DexError> {
        self.status = DexStatus::NotLoaded;
        self.vm = None;
        self.context = Some(context.clone());
        self.activity_descriptor = None;
        self.activity_object = None;

        let Some(entry) = package
            .resources
            .dex_entries
            .iter()
            .find(|entry| entry.as_str() == "classes.dex")
            .or_else(|| package.resources.dex_entries.first())
        else {
            return self.fail(context, "load", "В APK нет classes*.dex");
        };
        if package.resources.dex_entries.len() > 1 {
            context.unsupported_api(
                "dalvik.system.DexFile",
                "multidex",
                format!(
                    "M2 исполняет только {entry}; файлов DEX: {}",
                    package.resources.dex_entries.len()
                ),
            )?;
        }

        let bytes = match package.read_entry(entry) {
            Ok(bytes) => bytes,
            Err(error) => return self.fail(context, "load", error.to_string()),
        };
        let dex = match DexFile::parse(bytes) {
            Ok(dex) => dex,
            Err(error) => return self.fail(context, "parse", error.to_string()),
        };

        let Some(activity) = package.main_activity.as_deref() else {
            return self.fail(context, "resolve", "В манифесте нет launcher Activity");
        };
        let descriptor = activity_descriptor(&package.package_name, activity);
        if dex.find_class(&descriptor).is_none() {
            return self.fail(
                context,
                "resolve",
                format!("Класс launcher Activity {descriptor} не найден в classes.dex"),
            );
        }

        let class_count = dex.classes().len();
        let mut vm = Vm::with_owned_native_bridge(dex, AndroidBridge::new(context.clone()));
        let object = match vm.allocate_object(&descriptor) {
            Ok(object) => object,
            Err(error) => return self.fail(context, "allocate", error.to_string()),
        };
        let receiver = Value::Reference(HeapRef::Object(object));
        if vm.dex().find_method(&descriptor, "<init>", Some("()V")).is_some()
            && let Err(error) = vm.invoke(&descriptor, "<init>", "()V", &[receiver])
        {
            return self.fail(context, "<init>", error.to_string());
        }

        self.vm = Some(vm);
        self.activity_descriptor = Some(descriptor);
        self.activity_object = Some(object);
        self.status = DexStatus::Ready {
            dex_files: package.resources.dex_entries.len(),
            classes: class_count,
        };
        Ok(())
    }

    fn on_lifecycle(&mut self, state: ActivityState) -> Result<(), DexError> {
        let method = match state {
            ActivityState::Created => "onCreate",
            ActivityState::Started => "onStart",
            ActivityState::Resumed => "onResume",
            ActivityState::Paused => "onPause",
            ActivityState::Stopped => "onStop",
            ActivityState::Destroyed => "onDestroy",
        };
        self.invoke_activity_method(state, method)
    }

    fn on_input(&mut self, _event: &InputEvent) -> Result<(), DexError> {
        self.input_events_seen = self.input_events_seen.saturating_add(1);
        Ok(())
    }

    fn on_surface_changed(&mut self, width: u32, height: u32) -> Result<(), DexError> {
        tracing::debug!(width, height, "APK surface resized");
        Ok(())
    }

    fn status(&self) -> &DexStatus {
        &self.status
    }
}

#[derive(Debug, Clone)]
struct AndroidBridge {
    context: Context,
}

impl AndroidBridge {
    const MARKER_CLASS: &'static str = "Ldev/apksule/Bridge;";

    const fn new(context: Context) -> Self {
        Self { context }
    }
}

impl NativeBridge for AndroidBridge {
    fn invoke(
        &mut self,
        method: &ResolvedMethod,
        arguments: &[Value],
    ) -> Result<NativeResult, VmError> {
        if method.class_descriptor == Self::MARKER_CLASS && method.name == "markReached" {
            self.context
                .storage()
                .write_file("m2-on-create.txt", b"Activity.onCreate reached\n")
                .map_err(|error| VmError::NativeBridge(error.to_string()))?;
            return Ok(NativeResult::Handled(Value::Null));
        }

        if method.class_descriptor == "Ljava/lang/Object;" {
            let value = match method.name.as_str() {
                "<init>" => Value::Null,
                "hashCode" => Value::Int(0),
                "equals" => Value::Int(i32::from(arguments.first() == arguments.get(1))),
                _ => default_return_value(&method.prototype),
            };
            return Ok(NativeResult::Handled(value));
        }

        if method.class_descriptor == "Ljava/lang/String;"
            || method.class_descriptor.starts_with("Ljava/util/")
            || is_framework_class(&method.class_descriptor)
        {
            self.context
                .unsupported_api(
                    method.class_descriptor.clone(),
                    method.name.clone(),
                    format!("M2 fallback для {}", method.prototype),
                )
                .map_err(|error| VmError::NativeBridge(error.to_string()))?;
            return Ok(NativeResult::Handled(default_return_value(&method.prototype)));
        }

        Ok(NativeResult::Unresolved)
    }
}

fn is_framework_class(descriptor: &str) -> bool {
    descriptor.starts_with("Landroid/")
        || descriptor.starts_with("Landroidx/")
        || descriptor.starts_with("Ljavax/")
        || descriptor.starts_with("Lkotlin/")
}

fn default_return_value(prototype: &str) -> Value {
    match prototype.rsplit_once(')').map(|(_, result)| result) {
        Some("J") => Value::Long(0),
        Some("F") => Value::Float(0.0),
        Some("D") => Value::Double(0.0),
        Some("Z" | "B" | "C" | "S" | "I") => Value::Int(0),
        _ => Value::Null,
    }
}

fn activity_descriptor(package_name: &str, activity_name: &str) -> String {
    let qualified = if activity_name.starts_with('.') {
        format!("{package_name}{activity_name}")
    } else if activity_name.contains('.') {
        activity_name.to_owned()
    } else {
        format!("{package_name}.{activity_name}")
    };
    format!("L{};", qualified.replace('.', "/"))
}

fn descriptor_name(descriptor: &str) -> String {
    descriptor
        .strip_prefix('L')
        .and_then(|value| value.strip_suffix(';'))
        .unwrap_or(descriptor)
        .replace('/', ".")
}

#[derive(Debug, Default)]
pub struct StubDexRuntime {
    status: DexStatus,
    input_events_seen: u64,
}

impl StubDexRuntime {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn input_events_seen(&self) -> u64 {
        self.input_events_seen
    }
}

impl DexRuntime for StubDexRuntime {
    fn load(&mut self, package: &ApkPackage, context: &Context) -> Result<(), DexError> {
        let dex_files = package.resources.dex_entries.len();
        let reason = if dex_files == 0 {
            "В APK НЕТ CLASSES*.DEX".to_owned()
        } else {
            "ИСПОЛНЕНИЕ БАЙТКОДА DALVIK БУДЕТ В M2".to_owned()
        };
        context.unsupported_api(
            "dalvik.system.DexFile",
            "execute",
            format!("package={} dex_files={} reason={reason}", package.package_name, dex_files),
        )?;
        self.status = DexStatus::Unsupported { dex_files, reason };
        Ok(())
    }

    fn on_lifecycle(&mut self, state: ActivityState) -> Result<(), DexError> {
        tracing::debug!(?state, "Activity lifecycle delivered to DEX boundary");
        Ok(())
    }

    fn on_input(&mut self, _event: &InputEvent) -> Result<(), DexError> {
        self.input_events_seen = self.input_events_seen.saturating_add(1);
        Ok(())
    }

    fn on_surface_changed(&mut self, width: u32, height: u32) -> Result<(), DexError> {
        tracing::debug!(width, height, "APK surface resized");
        Ok(())
    }

    fn status(&self) -> &DexStatus {
        &self.status
    }
}

#[cfg(test)]
mod tests {
    use super::{activity_descriptor, default_return_value, descriptor_name};
    use apksule_dex::Value;

    #[test]
    fn normalizes_activity_names_to_dex_descriptors() {
        assert_eq!(
            activity_descriptor("dev.apksule.m2", ".MainActivity"),
            "Ldev/apksule/m2/MainActivity;"
        );
        assert_eq!(
            activity_descriptor("dev.apksule.m2", "dev.example.MainActivity"),
            "Ldev/example/MainActivity;"
        );
        assert_eq!(descriptor_name("Ldev/apksule/m2/MainActivity;"), "dev.apksule.m2.MainActivity");
    }

    #[test]
    fn native_fallback_uses_return_type_defaults() {
        assert_eq!(default_return_value("()I"), Value::Int(0));
        assert_eq!(default_return_value("()J"), Value::Long(0));
        assert_eq!(default_return_value("()Ljava/lang/String;"), Value::Null);
        assert_eq!(default_return_value("()V"), Value::Null);
    }
}
