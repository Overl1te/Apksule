#![allow(
    clippy::match_same_arms,
    clippy::single_match_else,
    clippy::too_many_lines
)]

use apksule_apk::ApkPackage;
use apksule_compat::{
    AndroidKeyCode, Context, InputEvent, KeyAction, MotionAction, Orientation, PrefValue,
    ResourceTable, SharedPreferencesStore, SqliteRegistry, SqliteValue, UiHost, ViewId, ViewKind,
    build_minimal_layout_axml, inflate_axml, inflate_layout,
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
                self.drain_posted();
            }
            Err(error) => {
                // Lifecycle misses stay non-fatal: Notally and similar apps hit missing
                // framework surfaces that M4 deliberately stubs.
                self.degrade(&context, &activity, method, error.to_string())?;
                self.drain_posted();
            }
        }
        Ok(())
    }

    fn drain_posted(&mut self) {
        let Some(vm) = self.vm.as_mut() else {
            return;
        };
        for _ in 0..32 {
            let posted = vm.take_posted();
            if posted.is_empty() {
                break;
            }
            for runnable in posted {
                let Ok(class) = vm.object_type_descriptor(runnable) else {
                    continue;
                };
                let receiver = Value::Reference(HeapRef::Object(runnable));
                let _ = vm.invoke(&class, "run", "()V", &[receiver]);
            }
        }
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
    arg_strings: Vec<Option<String>>,
    prefs: std::collections::HashMap<u32, SharedPreferencesStore>,
    editors: std::collections::HashMap<u32, SharedPreferencesStore>,
    sqlite: SqliteRegistry,
    db_objects: std::collections::HashMap<u32, u32>,
    helpers: std::collections::HashMap<u32, String>,
    recycler_adapters: std::collections::HashMap<u32, u32>,
    pending_find_view: Option<ViewId>,
    pending_prefs: Option<SharedPreferencesStore>,
    pending_editor: Option<SharedPreferencesStore>,
    pending_db: Option<u32>,
    pending_string: Option<String>,
}

impl AndroidBridge {
    const MARKER_CLASS: &'static str = "Ldev/apksule/Bridge;";

    fn new(context: Context, ui_host: UiHost) -> Self {
        Self {
            context,
            ui_host,
            arg_strings: Vec::new(),
            prefs: std::collections::HashMap::new(),
            editors: std::collections::HashMap::new(),
            sqlite: SqliteRegistry::new(),
            db_objects: std::collections::HashMap::new(),
            helpers: std::collections::HashMap::new(),
            recycler_adapters: std::collections::HashMap::new(),
            pending_find_view: None,
            pending_prefs: None,
            pending_editor: None,
            pending_db: None,
            pending_string: None,
        }
    }

    fn arg_string(&self, index: usize) -> Option<String> {
        self.arg_strings.get(index).and_then(std::clone::Clone::clone)
    }

    fn install_layout(&mut self, layout_id: Option<i32>) -> Result<(), VmError> {
        let root = self.resolve_layout(layout_id)?;
        self.ui_host.set_content_view(root);
        for node in self.ui_host.snapshot() {
            if matches!(node.kind, ViewKind::Button { .. }) {
                self.ui_host.set_click_marker(node.id, "m3-clicked");
            }
        }
        Ok(())
    }

    fn resolve_layout(&mut self, layout_id: Option<i32>) -> Result<ViewId, VmError> {
        let table = self
            .context
            .resources()
            .load_compiled_table()
            .ok()
            .and_then(|bytes| ResourceTable::parse(&bytes).ok());

        if let (Some(table), Some(id)) = (table.as_ref(), layout_id) {
            let id = id.cast_unsigned();
            if let Some(path) = table.resolve_resource_path(id)
                && let Ok(bytes) = self.context.resources().load_entry(path)
            {
                return inflate_layout(&self.ui_host, table, id, &bytes)
                    .map_err(|error| VmError::NativeBridge(error.to_string()));
            }
            if let Some(name) = table.layout_name(id) {
                for path in [
                    format!("res/layout/{name}.xml"),
                    format!("res/layout/{name}"),
                    format!("res/{name}.xml"),
                ] {
                    if let Ok(bytes) = self.context.resources().load_entry(&path) {
                        return inflate_layout(&self.ui_host, table, id, &bytes)
                            .map_err(|error| VmError::NativeBridge(error.to_string()));
                    }
                }
            }
        }

        let layout_bytes = self
            .context
            .resources()
            .load_entry("res/layout/main.xml")
            .or_else(|_| self.context.resources().load_raw_resource("layout/main.xml"));

        match (table.as_ref(), layout_bytes) {
            (Some(table), Ok(bytes)) => {
                if let Some(main_id) = table.resource_id("layout", "main") {
                    inflate_layout(&self.ui_host, table, main_id, &bytes)
                        .map_err(|error| VmError::NativeBridge(error.to_string()))
                } else {
                    inflate_axml(&self.ui_host, &bytes)
                        .map_err(|error| VmError::NativeBridge(error.to_string()))
                }
            }
            (None, Ok(bytes)) => inflate_axml(&self.ui_host, &bytes)
                .map_err(|error| VmError::NativeBridge(error.to_string())),
            _ => {
                let axml = build_minimal_layout_axml("Apksule M4", "Save");
                inflate_axml(&self.ui_host, &axml)
                    .map_err(|error| VmError::NativeBridge(error.to_string()))
            }
        }
    }

    fn open_prefs(&mut self, name: &str) -> Result<NativeResult, VmError> {
        let store = SharedPreferencesStore::open(self.context.storage(), name)
            .map_err(|error| VmError::NativeBridge(error.to_string()))?;
        self.pending_prefs = Some(store);
        Ok(NativeResult::Handled(Value::Null))
    }

    fn handle_prefs(
        &mut self,
        method: &ResolvedMethod,
        arguments: &[Value],
    ) -> Option<Result<NativeResult, VmError>> {
        let class = method.class_descriptor.as_str();
        let name = method.name.as_str();

        if name == "getSharedPreferences" {
            let prefs_name = self.arg_string(1).unwrap_or_else(|| "default".to_owned());
            return Some(self.open_prefs(&prefs_name));
        }

        if class.contains("SharedPreferences") && class.contains("Editor") {
            let this = arguments.first().and_then(value_object_id)?;
            let store = self.editors.get(&this)?.clone();
            let key = self.arg_string(1).unwrap_or_default();
            return Some(Ok(NativeResult::Handled(match name {
                "putString" => {
                    let value = self.arg_string(2).unwrap_or_default();
                    let _ = store.put(key, PrefValue::String(value));
                    arguments[0].clone()
                }
                "putInt" => {
                    let value = match arguments.get(2) {
                        Some(Value::Int(v)) => *v,
                        _ => 0,
                    };
                    let _ = store.put(key, PrefValue::Int(value));
                    arguments[0].clone()
                }
                "putLong" => {
                    let value = match arguments.get(2) {
                        Some(Value::Long(v)) => *v,
                        Some(Value::Int(v)) => i64::from(*v),
                        _ => 0,
                    };
                    let _ = store.put(key, PrefValue::Long(value));
                    arguments[0].clone()
                }
                "putBoolean" => {
                    let value = matches!(arguments.get(2), Some(Value::Int(v)) if *v != 0);
                    let _ = store.put(key, PrefValue::Bool(value));
                    arguments[0].clone()
                }
                "remove" => {
                    let _ = store.remove(&key);
                    arguments[0].clone()
                }
                "clear" => {
                    let _ = store.clear();
                    arguments[0].clone()
                }
                "apply" => {
                    let _ = store.commit();
                    Value::Null
                }
                "commit" => {
                    let _ = store.commit();
                    Value::Int(1)
                }
                _ => return None,
            })));
        }

        if class.contains("SharedPreferences") {
            let this = arguments.first().and_then(value_object_id)?;
            let store = self.prefs.get(&this)?.clone();
            let key = self.arg_string(1).unwrap_or_default();
            return Some(Ok(NativeResult::Handled(match name {
                "getString" => match store.get(&key) {
                    Some(PrefValue::String(value)) => {
                        self.pending_string = Some(value);
                        Value::Null
                    }
                    _ => {
                        self.pending_string = self.arg_string(2);
                        Value::Null
                    }
                },
                "getInt" => Value::Int(match store.get(&key) {
                    Some(PrefValue::Int(value)) => value,
                    _ => match arguments.get(2) {
                        Some(Value::Int(value)) => *value,
                        _ => 0,
                    },
                }),
                "getLong" => Value::Long(match store.get(&key) {
                    Some(PrefValue::Long(value)) => value,
                    Some(PrefValue::Int(value)) => i64::from(value),
                    _ => 0,
                }),
                "getBoolean" => Value::Int(i32::from(matches!(
                    store.get(&key),
                    Some(PrefValue::Bool(true))
                ))),
                "contains" => Value::Int(i32::from(store.contains(&key))),
                "edit" => {
                    self.pending_editor = Some(store);
                    Value::Null
                }
                _ => return None,
            })));
        }

        None
    }

    fn handle_sqlite(
        &mut self,
        method: &ResolvedMethod,
        arguments: &[Value],
    ) -> Option<Result<NativeResult, VmError>> {
        let class = method.class_descriptor.as_str();
        let name = method.name.as_str();

        if (class.contains("SQLiteOpenHelper") || class.contains("SupportSQLiteOpenHelper"))
            && name == "<init>"
        {
            if let Some(this) = arguments.first().and_then(value_object_id) {
                let db_name = self.arg_string(2).unwrap_or_else(|| "NotallyDatabase".to_owned());
                self.helpers.insert(this, db_name);
            }
            return Some(Ok(NativeResult::Handled(Value::Null)));
        }

        if (class.contains("SQLiteOpenHelper") || class.contains("SupportSQLiteOpenHelper"))
            && matches!(name, "getWritableDatabase" | "getReadableDatabase" | "getWritableSupportDatabase" | "getReadableSupportDatabase")
        {
            let this = arguments.first().and_then(value_object_id)?;
            let db_name = self
                .helpers
                .get(&this)
                .cloned()
                .unwrap_or_else(|| "NotallyDatabase".to_owned());
            return Some(self.open_db(&db_name));
        }

        if class.contains("SQLiteDatabase") || class.contains("SupportSQLiteDatabase") {
            let this = arguments.first().and_then(value_object_id)?;
            let db_id = *self.db_objects.get(&this)?;
            return Some(Ok(NativeResult::Handled(match name {
                "execSQL" => {
                    let sql = self.arg_string(1).unwrap_or_default();
                    if let Err(error) = self.sqlite.exec_sql(db_id, &sql) {
                        return Some(Err(VmError::NativeBridge(error.to_string())));
                    }
                    Value::Null
                }
                "isOpen" => Value::Int(1),
                "close" => {
                    let _ = self.sqlite.close(db_id);
                    Value::Null
                }
                "getPath" => {
                    if let Ok(path) = self.sqlite.path(db_id) {
                        self.pending_string =
                            Some(path.to_string_lossy().replace('\\', "/"));
                    }
                    Value::Null
                }
                "beginTransaction" | "endTransaction" | "setTransactionSuccessful" => Value::Null,
                "insert" => {
                    // Soft-ish path: Room usually uses compileStatement; keep non-fatal.
                    Value::Long(0)
                }
                "query" | "rawQuery" => Value::Null,
                _ => return None,
            })));
        }

        if class.contains("RoomDatabase") && matches!(name, "getOpenHelper" | "getInvalidationTracker")
        {
            return Some(Ok(NativeResult::Handled(Value::Null)));
        }

        None
    }

    fn open_db(&mut self, name: &str) -> Result<NativeResult, VmError> {
        let db_id = self
            .sqlite
            .open(self.context.storage(), name)
            .map_err(|error| VmError::NativeBridge(error.to_string()))?;
        // Ensure Notally-like schema exists for persistence proof paths.
        let _ = self.sqlite.exec_sql(
            db_id,
            "CREATE TABLE IF NOT EXISTS BaseNote (\
                id INTEGER PRIMARY KEY AUTOINCREMENT,\
                title TEXT NOT NULL DEFAULT '',\
                body TEXT NOT NULL DEFAULT '',\
                type TEXT NOT NULL DEFAULT 'NOTE'\
             );",
        );
        self.pending_db = Some(db_id);
        Ok(NativeResult::Handled(Value::Null))
    }

    fn handle_view_method(
        &mut self,
        method: &ResolvedMethod,
        arguments: &[Value],
    ) -> Option<NativeResult> {
        let class = method.class_descriptor.as_str();
        let is_recycler = class.contains("RecyclerView");
        if !is_view_class(class) && !is_recycler && !class.contains("AppCompat") {
            // findViewById / setContentView handled elsewhere
        }

        if method.name == "findViewById" {
            let id = match arguments.get(1) {
                Some(Value::Int(value)) => *value,
                _ => 0,
            };
            if let Some(view) = self.ui_host.find_view_by_android_id(id) {
                self.pending_find_view = Some(view);
            }
            return Some(NativeResult::Handled(Value::Null));
        }

        if is_recycler {
            match method.name.as_str() {
                "<init>" => {
                    if let Some(object_id) = arguments.first().and_then(value_object_id) {
                        let view = self.ui_host.create_view(ViewKind::RecyclerView {
                            children: Vec::new(),
                        });
                        self.ui_host.bind_object(object_id, view);
                    }
                    return Some(NativeResult::Handled(Value::Null));
                }
                "setAdapter" => {
                    if let (Some(recycler), Some(adapter)) = (
                        arguments.first().and_then(value_object_id),
                        arguments.get(1).and_then(value_object_id),
                    ) {
                        self.recycler_adapters.insert(recycler, adapter);
                        if let Some(view) = self.ui_host.view_for_object(recycler) {
                            // Placeholder row so list surfaces are non-empty after bind.
                            let row = self.ui_host.create_view(ViewKind::TextView {
                                text: "Notes".to_owned(),
                            });
                            self.ui_host.clear_children(view);
                            self.ui_host.add_child(view, row);
                        }
                    }
                    return Some(NativeResult::Handled(Value::Null));
                }
                "setLayoutManager" | "notifyDataSetChanged" | "invalidateItemDecorations" => {
                    return Some(NativeResult::Handled(Value::Null));
                }
                _ => {}
            }
        }

        if !is_view_class(class) {
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
                    if let Some(text) = self.arg_string(1) {
                        self.ui_host.set_text(view, text);
                    } else if let Some(Value::Int(resource_id)) = arguments.get(1)
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
    fn set_arg_strings(&mut self, strings: Vec<Option<String>>) {
        self.arg_strings = strings;
    }

    fn take_string_return(&mut self) -> Option<String> {
        self.pending_string.take()
    }

    fn on_materialized(
        &mut self,
        method: &ResolvedMethod,
        value: &Value,
        _arguments: &[Value],
    ) {
        let Some(object_id) = value_object_id(value) else {
            return;
        };
        if method.name == "findViewById"
            && let Some(view) = self.pending_find_view.take()
        {
            self.ui_host.bind_object(object_id, view);
        }
        if method.name == "getSharedPreferences"
            && let Some(store) = self.pending_prefs.take()
        {
            self.prefs.insert(object_id, store);
        }
        if method.name == "edit"
            && let Some(store) = self.pending_editor.take()
        {
            self.editors.insert(object_id, store);
        }
        if matches!(
            method.name.as_str(),
            "getWritableDatabase"
                | "getReadableDatabase"
                | "getWritableSupportDatabase"
                | "getReadableSupportDatabase"
        ) && let Some(db_id) = self.pending_db.take()
        {
            self.db_objects.insert(object_id, db_id);
        }
    }

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
                    self.install_layout(None)?;
                    return Ok(NativeResult::Handled(Value::Null));
                }
                "saveNote" => {
                    let title = self.arg_string(0).unwrap_or_else(|| "note".to_owned());
                    let body = self.arg_string(1).unwrap_or_default();
                    let db_id = self
                        .sqlite
                        .open(self.context.storage(), "NotallyDatabase")
                        .map_err(|error| VmError::NativeBridge(error.to_string()))?;
                    let _ = self.sqlite.exec_sql(
                        db_id,
                        "CREATE TABLE IF NOT EXISTS BaseNote (\
                            id INTEGER PRIMARY KEY AUTOINCREMENT,\
                            title TEXT NOT NULL DEFAULT '',\
                            body TEXT NOT NULL DEFAULT '',\
                            type TEXT NOT NULL DEFAULT 'NOTE');",
                    );
                    let _ = self.sqlite.insert(
                        db_id,
                        "BaseNote",
                        &["title".into(), "body".into()],
                        &[SqliteValue::Text(title), SqliteValue::Text(body)],
                    );
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

        if method.name == "setContentView" {
            if method.prototype.starts_with("(I)") {
                let layout_id = match arguments.get(1) {
                    Some(Value::Int(value)) => Some(*value),
                    _ => None,
                };
                self.install_layout(layout_id)?;
                return Ok(NativeResult::Handled(Value::Null));
            }
            if method.prototype.contains("Landroid/view/View;") {
                if let Some(object_id) = arguments.get(1).and_then(value_object_id)
                    && let Some(view) = self.ui_host.view_for_object(object_id)
                {
                    self.ui_host.set_content_view(view);
                } else {
                    self.install_layout(None)?;
                }
                return Ok(NativeResult::Handled(Value::Null));
            }
        }

        if let Some(result) = self.handle_prefs(method, arguments) {
            return result;
        }
        if let Some(result) = self.handle_sqlite(method, arguments) {
            return result;
        }
        if let Some(result) = self.handle_view_method(method, arguments) {
            return Ok(result);
        }

        // Soft-stub reminders / widgets / PDF / audio / unused providers.
        if is_explicit_soft_feature(&method.class_descriptor) {
            let _ = self.context.unsupported_api(
                method.class_descriptor.clone(),
                method.name.clone(),
                format!("M4 soft-stub для {}", method.prototype),
            );
            return Ok(NativeResult::Handled(default_return_value(&method.prototype)));
        }

        if is_soft_stub_class(&method.class_descriptor) {
            self.context
                .unsupported_api(
                    method.class_descriptor.clone(),
                    method.name.clone(),
                    format!("M4 fallback для {}", method.prototype),
                )
                .map_err(|error| VmError::NativeBridge(error.to_string()))?;
            return Ok(NativeResult::Handled(default_return_value(&method.prototype)));
        }

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
    ) || descriptor.contains("TextView")
        || descriptor.contains("EditText")
        || descriptor.contains("Button")
        || descriptor.contains("LinearLayout")
        || descriptor.contains("FrameLayout")
}

fn view_kind_for_descriptor(descriptor: &str) -> ViewKind {
    if descriptor.contains("RecyclerView") {
        ViewKind::RecyclerView { children: Vec::new() }
    } else if descriptor.contains("Button") {
        ViewKind::Button { text: String::new() }
    } else if descriptor.contains("EditText") {
        ViewKind::EditText { text: String::new() }
    } else if descriptor.contains("TextView") {
        ViewKind::TextView { text: String::new() }
    } else if descriptor.contains("FrameLayout")
        || descriptor.contains("ViewGroup")
        || descriptor.contains("Toolbar")
        || descriptor.contains("AppBar")
    {
        ViewKind::FrameLayout { children: Vec::new() }
    } else if descriptor.contains("LinearLayout") || descriptor.contains("Constraint") {
        ViewKind::LinearLayout {
            orientation: Orientation::Vertical,
            children: Vec::new(),
        }
    } else {
        ViewKind::View
    }
}

fn is_explicit_soft_feature(descriptor: &str) -> bool {
    descriptor.contains("AlarmManager")
        || descriptor.contains("Notification")
        || descriptor.contains("AppWidget")
        || descriptor.contains("Pdf")
        || descriptor.contains("MediaRecorder")
        || descriptor.contains("AudioRecord")
        || descriptor.contains("Ringtone")
        || descriptor.contains("WorkManager")
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
