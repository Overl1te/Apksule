use apksule_apk::ApkPackage;
use apksule_compat::{
    AndroidKeyCode, Context, InputEvent, KeyAction, MotionAction, Orientation, ResourceTable,
    UiHost, ViewKind, build_minimal_layout_axml, inflate_axml, inflate_layout,
};
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
    /// Lifecycle reached, but some framework calls were stubbed or truncated.
    Degraded {
        activity: String,
        reason: String,
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

/// Stable seam for the M3 interpreter. The host never depends on this trait.
pub trait DexRuntime {
    fn load(&mut self, package: &ApkPackage, context: &Context) -> Result<(), DexError>;
    fn on_lifecycle(&mut self, state: ActivityState) -> Result<(), DexError>;
    fn on_input(&mut self, event: &InputEvent) -> Result<(), DexError>;
    fn on_surface_changed(&mut self, width: u32, height: u32) -> Result<(), DexError>;
    fn status(&self) -> &DexStatus;
    fn ui_host(&self) -> Option<&UiHost> {
        None
    }
}

pub struct InterpretingDexRuntime {
    status: DexStatus,
    vm: Option<Vm>,
    context: Option<Context>,
    activity_descriptor: Option<String>,
    activity_object: Option<ObjectRef>,
    input_events_seen: u64,
    ui_host: UiHost,
}

impl Default for InterpretingDexRuntime {
    fn default() -> Self {
        Self {
            status: DexStatus::default(),
            vm: None,
            context: None,
            activity_descriptor: None,
            activity_object: None,
            input_events_seen: 0,
            ui_host: UiHost::new(),
        }
    }
}

impl InterpretingDexRuntime {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn ui(&self) -> &UiHost {
        &self.ui_host
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

    fn degrade(
        &mut self,
        context: &Context,
        activity: &str,
        operation: &str,
        reason: impl Into<String>,
    ) -> Result<(), DexError> {
        let reason = reason.into();
        tracing::warn!(%reason, %operation, activity, "DEX lifecycle limited by missing APIs");
        self.status = DexStatus::Degraded { activity: activity.to_owned(), reason: reason.clone() };
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
        let activity = descriptor_name(&descriptor);

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
                self.degrade(
                    &context,
                    &activity,
                    method,
                    format!("{descriptor}->{method} не найден; lifecycle продолжен ограниченно"),
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
                if !matches!(self.status, DexStatus::Degraded { .. }) {
                    self.status = DexStatus::Running {
                        activity: activity.clone(),
                        method: format!("{method}()"),
                    };
                }
                tracing::info!(activity = %descriptor, %method, ?state, "DEX method executed");
            }
            Err(error) => {
                // Lifecycle misses stay non-fatal: Notally and similar apps hit missing
                // framework surfaces that M3 deliberately stubs.
                self.degrade(&context, &activity, method, error.to_string())?;
            }
        }
        Ok(())
    }

    fn handle_key_down(&mut self, event: &apksule_compat::KeyEvent) {
        if let Some(text) = event.text.as_deref() {
            for ch in text.chars() {
                let _ = self.ui_host.type_char(ch);
            }
            return;
        }
        match &event.key_code {
            AndroidKeyCode::Character(value) => {
                for ch in value.chars() {
                    let _ = self.ui_host.type_char(ch);
                }
            }
            AndroidKeyCode::Space => {
                let _ = self.ui_host.type_char(' ');
            }
            AndroidKeyCode::Delete => {
                let _ = self.ui_host.type_char('\u{8}');
            }
            AndroidKeyCode::Enter => {
                let _ = self.ui_host.type_char('\n');
            }
            _ => {}
        }
    }
}

impl DexRuntime for InterpretingDexRuntime {
    fn load(&mut self, package: &ApkPackage, context: &Context) -> Result<(), DexError> {
        self.status = DexStatus::NotLoaded;
        self.vm = None;
        self.context = Some(context.clone());
        self.activity_descriptor = None;
        self.activity_object = None;
        self.ui_host = UiHost::new();

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
                    "M3 исполняет только {entry}; файлов DEX: {}",
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
        let mut vm = Vm::with_owned_native_bridge(
            dex,
            AndroidBridge::new(context.clone(), self.ui_host.clone()),
        );
        let object = match vm.allocate_object(&descriptor) {
            Ok(object) => object,
            Err(error) => return self.fail(context, "allocate", error.to_string()),
        };
        let receiver = Value::Reference(HeapRef::Object(object));
        let init_error = if vm.dex().find_method(&descriptor, "<init>", Some("()V")).is_some() {
            vm.invoke(&descriptor, "<init>", "()V", &[receiver]).err()
        } else {
            None
        };

        self.vm = Some(vm);
        self.activity_descriptor = Some(descriptor.clone());
        self.activity_object = Some(object);
        self.status = DexStatus::Ready {
            dex_files: package.resources.dex_entries.len(),
            classes: class_count,
        };
        if let Some(error) = init_error {
            // Framework-heavy Activity constructors must not abort the session.
            return self.degrade(
                context,
                &descriptor_name(&descriptor),
                "<init>",
                error.to_string(),
            );
        }
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

    fn on_input(&mut self, event: &InputEvent) -> Result<(), DexError> {
        self.input_events_seen = self.input_events_seen.saturating_add(1);
        match event {
            InputEvent::Motion(motion) if motion.action == MotionAction::Up => {
                #[allow(clippy::cast_possible_truncation)]
                let x = motion.x as i32;
                #[allow(clippy::cast_possible_truncation)]
                let y = motion.y as i32;
                if let Some(marker) = self.ui_host.pointer_up(x, y) {
                    let body = if marker.is_empty() {
                        "clicked\n".to_owned()
                    } else {
                        format!("{marker}\n")
                    };
                    if let Some(context) = &self.context {
                        context.storage().write_file("m3-click.txt", body.as_bytes())?;
                    }
                }
            }
            InputEvent::Key(key) if key.action == KeyAction::Down => {
                self.handle_key_down(key);
            }
            _ => {}
        }
        Ok(())
    }

    fn on_surface_changed(&mut self, width: u32, height: u32) -> Result<(), DexError> {
        self.ui_host.set_surface_size(width, height);
        tracing::debug!(width, height, "APK surface resized");
        Ok(())
    }

    fn status(&self) -> &DexStatus {
        &self.status
    }

    fn ui_host(&self) -> Option<&UiHost> {
        Some(&self.ui_host)
    }
}

#[derive(Debug, Clone)]
struct AndroidBridge {
    context: Context,
    ui_host: UiHost,
}

impl AndroidBridge {
    const MARKER_CLASS: &'static str = "Ldev/apksule/Bridge;";

    fn new(context: Context, ui_host: UiHost) -> Self {
        Self { context, ui_host }
    }

    fn install_main_layout(&mut self) -> Result<(), VmError> {
        let root = self.resolve_main_layout()?;
        self.ui_host.set_content_view(root);
        for node in self.ui_host.snapshot() {
            if matches!(node.kind, ViewKind::Button { .. }) {
                self.ui_host.set_click_marker(node.id, "m3-clicked");
            }
        }
        Ok(())
    }

    fn resolve_main_layout(&mut self) -> Result<apksule_compat::ViewId, VmError> {
        let table = self
            .context
            .resources()
            .load_compiled_table()
            .ok()
            .and_then(|bytes| ResourceTable::parse(&bytes).ok());

        let layout_bytes = self
            .context
            .resources()
            .load_entry("res/layout/main.xml")
            .or_else(|_| self.context.resources().load_raw_resource("layout/main.xml"));

        match (table.as_ref(), layout_bytes) {
            (Some(table), Ok(bytes)) => {
                if let Some(layout_id) = table.resource_id("layout", "main") {
                    inflate_layout(&self.ui_host, table, layout_id, &bytes)
                        .map_err(|error| VmError::NativeBridge(error.to_string()))
                } else {
                    inflate_axml(&self.ui_host, &bytes)
                        .map_err(|error| VmError::NativeBridge(error.to_string()))
                }
            }
            (None, Ok(bytes)) => inflate_axml(&self.ui_host, &bytes)
                .map_err(|error| VmError::NativeBridge(error.to_string())),
            _ => {
                let axml = build_minimal_layout_axml("Apksule M3", "Save");
                inflate_axml(&self.ui_host, &axml)
                    .map_err(|error| VmError::NativeBridge(error.to_string()))
            }
        }
    }

    fn handle_view_method(
        &mut self,
        method: &ResolvedMethod,
        arguments: &[Value],
    ) -> Option<NativeResult> {
        if !is_view_class(&method.class_descriptor) {
            return None;
        }

        match method.name.as_str() {
            "<init>" => {
                if let Some(object_id) = arguments.first().and_then(value_object_id) {
                    let kind = view_kind_for_descriptor(&method.class_descriptor);
                    let view = self.ui_host.create_view(kind);
                    self.ui_host.bind_object(object_id, view);
                }
                Some(NativeResult::Handled(Value::Null))
            }
            "addView" => {
                let parent = arguments.first().and_then(value_object_id);
                let child = arguments.get(1).and_then(value_object_id);
                if let (Some(parent), Some(child)) = (parent, child)
                    && let (Some(parent_view), Some(child_view)) = (
                        self.ui_host.view_for_object(parent),
                        self.ui_host.view_for_object(child),
                    )
                {
                    self.ui_host.add_child(parent_view, child_view);
                }
                Some(NativeResult::Handled(Value::Null))
            }
            "setText" => {
                if let Some(object_id) = arguments.first().and_then(value_object_id)
                    && let Some(view) = self.ui_host.view_for_object(object_id)
                {
                    if let Some(Value::Int(resource_id)) = arguments.get(1)
                        && let Ok(bytes) = self.context.resources().load_compiled_table()
                        && let Ok(table) = ResourceTable::parse(&bytes)
                        && let Some(text) =
                            table.resolve_string_id((*resource_id).cast_unsigned())
                    {
                        self.ui_host.set_text(view, text);
                    } else if matches!(arguments.get(1), Some(Value::Null)) {
                        self.ui_host.set_text(view, "");
                    }
                }
                Some(NativeResult::Handled(Value::Null))
            }
            "setOnClickListener" => {
                if let Some(object_id) = arguments.first().and_then(value_object_id)
                    && let Some(view) = self.ui_host.view_for_object(object_id)
                {
                    let has_listener =
                        arguments.get(1).is_some_and(|value| !matches!(value, Value::Null));
                    if has_listener {
                        self.ui_host.set_click_marker(view, "m3-clicked");
                    }
                }
                Some(NativeResult::Handled(Value::Null))
            }
            _ => None,
        }
    }
}

impl NativeBridge for AndroidBridge {
    fn invoke(
        &mut self,
        method: &ResolvedMethod,
        arguments: &[Value],
    ) -> Result<NativeResult, VmError> {
        if method.class_descriptor == Self::MARKER_CLASS {
            match method.name.as_str() {
                "markReached" => {
                    self.context
                        .storage()
                        .write_file("m3-on-create.txt", b"Activity.onCreate reached\n")
                        .map_err(|error| VmError::NativeBridge(error.to_string()))?;
                    return Ok(NativeResult::Handled(Value::Null));
                }
                "setContentView" | "installUi" => {
                    self.install_main_layout()?;
                    return Ok(NativeResult::Handled(Value::Null));
                }
                _ => {}
            }
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

        if method.name == "setContentView"
            && (method.prototype.starts_with("(I)")
                || method.prototype.contains("Landroid/view/View;"))
        {
            self.install_main_layout()?;
            return Ok(NativeResult::Handled(Value::Null));
        }

        if let Some(result) = self.handle_view_method(method, arguments) {
            return Ok(result);
        }

        if is_soft_stub_class(&method.class_descriptor) {
            self.context
                .unsupported_api(
                    method.class_descriptor.clone(),
                    method.name.clone(),
                    format!("M3 fallback для {}", method.prototype),
                )
                .map_err(|error| VmError::NativeBridge(error.to_string()))?;
            return Ok(NativeResult::Handled(default_return_value(&method.prototype)));
        }

        // Unknown APK-local natives still fail hard; standard/Java/Android never do.
        Ok(NativeResult::Unresolved)
    }
}

fn value_object_id(value: &Value) -> Option<u32> {
    match value {
        Value::Reference(HeapRef::Object(ObjectRef(id))) => Some(*id),
        _ => None,
    }
}

fn is_view_class(descriptor: &str) -> bool {
    matches!(
        descriptor,
        "Landroid/view/View;"
            | "Landroid/view/ViewGroup;"
            | "Landroid/widget/TextView;"
            | "Landroid/widget/EditText;"
            | "Landroid/widget/Button;"
            | "Landroid/widget/LinearLayout;"
            | "Landroid/widget/FrameLayout;"
    )
}

fn view_kind_for_descriptor(descriptor: &str) -> ViewKind {
    match descriptor {
        "Landroid/widget/Button;" => ViewKind::Button { text: String::new() },
        "Landroid/widget/EditText;" => ViewKind::EditText { text: String::new() },
        "Landroid/widget/TextView;" => ViewKind::TextView { text: String::new() },
        "Landroid/widget/FrameLayout;" | "Landroid/view/ViewGroup;" => {
            ViewKind::FrameLayout { children: Vec::new() }
        }
        "Landroid/widget/LinearLayout;" => ViewKind::LinearLayout {
            orientation: Orientation::Vertical,
            children: Vec::new(),
        },
        _ => ViewKind::View,
    }
}

fn is_soft_stub_class(descriptor: &str) -> bool {
    descriptor.starts_with("Ljava/")
        || descriptor.starts_with("Landroid/")
        || descriptor.starts_with("Landroidx/")
        || descriptor.starts_with("Ljavax/")
        || descriptor.starts_with("Lkotlin/")
        || descriptor.starts_with("Ldalvik/")
        || descriptor.starts_with("Llibcore/")
        || descriptor.starts_with("Lsun/")
        || descriptor.starts_with("Lcom/android/")
        || descriptor.starts_with("Lcom/google/")
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
    use super::{activity_descriptor, default_return_value, descriptor_name, is_soft_stub_class};
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

    #[test]
    fn soft_stubs_cover_weak_reference_and_java_lang() {
        assert!(is_soft_stub_class("Ljava/lang/ref/WeakReference;"));
        assert!(is_soft_stub_class("Ljava/util/concurrent/atomic/AtomicReference;"));
        assert!(is_soft_stub_class("Landroid/app/Activity;"));
        assert!(!is_soft_stub_class("Lcom/omgodse/notally/activities/MainActivity;"));
    }
}
