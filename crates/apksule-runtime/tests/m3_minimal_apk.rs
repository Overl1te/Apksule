mod support;

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use apksule_apk::{ApkLoader, ApkPackage};
use apksule_compat::{
    Context, InputEvent, KeyAction, KeyEvent, MotionAction, MotionEvent, PointerButton,
    ResourceSource, build_minimal_arsc, build_minimal_layout_axml,
};
use apksule_runtime::{
    ActivityState, DexRuntime, DexStatus, InterpretingDexRuntime, render_view_surface,
};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use support::dex_fixture::minimal_m3_activity_dex;

const MANIFEST: &str = r#"
<manifest xmlns:android="http://schemas.android.com/apk/res/android"
    package="dev.apksule.m3"
    android:versionCode="1"
    android:versionName="1.0">
    <uses-sdk android:minSdkVersion="21" android:targetSdkVersion="35" />
    <application android:label="Apksule M3 Test">
        <activity android:name=".MainActivity" android:exported="true">
            <intent-filter>
                <action android:name="android.intent.action.MAIN" />
                <category android:name="android.intent.category.LAUNCHER" />
            </intent-filter>
        </activity>
    </application>
</manifest>
"#;

#[test]
fn m3_runtime_inflates_ui_draws_and_handles_click() {
    let fixture = TestApk::new();
    let package = ApkLoader::open(&fixture.path).expect("тестовый APK должен разбираться");
    assert_eq!(package.package_name, "dev.apksule.m3");
    assert!(package.resources.has_resource_table);

    let storage_base = fixture.path.with_extension("storage");
    let context = Context::with_storage_base(
        package.package_name.clone(),
        Arc::new(PackageResources { package: package.clone() }),
        &[],
        &storage_base,
    )
    .expect("контекст M3");

    let mut runtime = InterpretingDexRuntime::new();
    runtime.load(&package, &context).expect("загрузка DEX");
    runtime.on_surface_changed(640, 480).expect("surface");
    runtime.on_lifecycle(ActivityState::Created).expect("onCreate");

    assert!(matches!(
        runtime.status(),
        DexStatus::Running { method, .. } if method == "onCreate()"
    ));
    assert_eq!(
        context.storage().read_file("m3-on-create.txt").expect("onCreate marker"),
        b"Activity.onCreate reached\n"
    );

    let host = runtime.ui();
    assert!(host.has_content(), "setContentView должен установить дерево View");
    let nodes = host.snapshot();
    assert!(nodes.len() >= 3, "layout: LinearLayout + children");
    assert!(nodes.iter().any(|node| node.kind.text() == Some("Hello M3")));
    assert!(nodes.iter().any(|node| node.kind.text() == Some("Save")));

    let frame = render_view_surface(host, 640, 480).expect("отрисовка UI");
    assert_eq!(frame.len(), 640 * 480);
    // Diagnostic splash uses dark navy ~ (13,18,28); content surface is light.
    let sample = frame[640 * 40 + 40];
    let r = (sample >> 16) & 0xff;
    let g = (sample >> 8) & 0xff;
    let b = sample & 0xff;
    assert!(r > 180 && g > 180 && b > 180, "UI surface should be light, got {r},{g},{b}");

    let button = nodes.iter().find(|node| node.kind.text() == Some("Save")).expect("button");
    let x = (button.bounds.left + button.bounds.right) / 2;
    let y = (button.bounds.top + button.bounds.bottom) / 2;
    runtime
        .on_input(&InputEvent::Motion(MotionEvent {
            action: MotionAction::Up,
            pointer_id: 0,
            x: x as f32,
            y: y as f32,
            button: Some(PointerButton::Primary),
            scroll_x: 0.0,
            scroll_y: 0.0,
            timestamp_ms: 0,
        }))
        .expect("click");
    assert_eq!(
        context.storage().read_file("m3-click.txt").expect("click marker"),
        b"m3-clicked\n"
    );

    // Focus edit text then type a character.
    let edit = nodes.iter().find(|node| matches!(node.kind, apksule_compat::ViewKind::EditText { .. }));
    if let Some(edit) = edit {
        let ex = (edit.bounds.left + edit.bounds.right) / 2;
        let ey = (edit.bounds.top + edit.bounds.bottom) / 2;
        runtime
            .on_input(&InputEvent::Motion(MotionEvent {
                action: MotionAction::Up,
                pointer_id: 0,
                x: ex as f32,
                y: ey as f32,
                button: Some(PointerButton::Primary),
                scroll_x: 0.0,
                scroll_y: 0.0,
                timestamp_ms: 1,
            }))
            .expect("focus");
        runtime
            .on_input(&InputEvent::Key(KeyEvent {
                action: KeyAction::Down,
                key_code: apksule_compat::AndroidKeyCode::Character("A".into()),
                text: Some("A".into()),
                repeat: false,
                timestamp_ms: 2,
            }))
            .expect("type");
        let typed = runtime.ui().snapshot().iter().any(|node| {
            matches!(&node.kind, apksule_compat::ViewKind::EditText { text } if text.contains('A'))
        });
        assert!(typed, "EditText должен принять ввод");
    }

    let _ = std::fs::remove_dir_all(storage_base);
}

#[derive(Debug, Clone)]
struct PackageResources {
    package: ApkPackage,
}

impl ResourceSource for PackageResources {
    fn contains(&self, path: &str) -> bool {
        self.package.contains_entry(path)
    }

    fn load(&self, path: &str) -> Result<Vec<u8>, String> {
        self.package.read_entry(path).map_err(|error| error.to_string())
    }
}

struct TestApk {
    path: PathBuf,
}

impl TestApk {
    fn new() -> Self {
        let unique =
            SystemTime::now().duration_since(UNIX_EPOCH).expect("системное время").as_nanos();
        let path = std::env::temp_dir().join(format!("apksule-m3-{unique}.apk"));
        write_apk(&path);
        Self { path }
    }
}

impl Drop for TestApk {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn write_apk(path: &Path) {
    let file = File::create(path).expect("создать тестовый APK");
    let mut archive = ZipWriter::new(file);
    let options = SimpleFileOptions::default();

    archive.start_file("AndroidManifest.xml", options).expect("manifest");
    archive.write_all(MANIFEST.as_bytes()).expect("manifest bytes");
    archive.start_file("classes.dex", options).expect("dex");
    archive.write_all(&minimal_m3_activity_dex()).expect("dex bytes");
    archive.start_file("resources.arsc", options).expect("arsc");
    archive
        .write_all(&build_minimal_arsc("dev.apksule.m3", "M3 Demo", "main"))
        .expect("arsc bytes");
    archive.start_file("res/layout/main.xml", options).expect("layout");
    archive
        .write_all(&build_minimal_layout_axml("Hello M3", "Save"))
        .expect("layout bytes");
    archive.finish().expect("закрыть APK");
}
