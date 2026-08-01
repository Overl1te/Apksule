mod support;

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use apksule_apk::ApkLoader;
use apksule_compat::{Context, ResourceSource};
use apksule_dex::DexFile;
use apksule_runtime::{ActivityState, DexRuntime, DexStatus, InterpretingDexRuntime};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use support::dex_fixture::minimal_activity_dex;

const MANIFEST: &str = r#"
<manifest xmlns:android="http://schemas.android.com/apk/res/android"
    package="dev.apksule.m2"
    android:versionCode="1"
    android:versionName="1.0">
    <uses-sdk android:minSdkVersion="21" android:targetSdkVersion="35" />
    <application android:label="Apksule M2 Test">
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
fn generated_apk_exposes_m2_activity_and_dex() {
    let fixture = TestApk::new();
    let package = ApkLoader::open(&fixture.path).expect("тестовый APK должен разбираться");

    assert_eq!(package.package_name, "dev.apksule.m2");
    assert_eq!(package.main_activity.as_deref(), Some("dev.apksule.m2.MainActivity"));
    assert_eq!(package.resources.dex_entries, ["classes.dex"]);
    assert_eq!(package.read_entry("classes.dex").expect("classes.dex"), minimal_activity_dex());

    let dex = DexFile::parse(package.read_entry("classes.dex").expect("classes.dex"))
        .expect("DEX тестового APK должен разбираться");
    let activity = dex
        .find_method("Ldev/apksule/m2/MainActivity;", "onCreate", Some("()V"))
        .expect("метод Activity.onCreate");
    assert!(activity.encoded.is_some(), "onCreate должен содержать DEX-код");
}

#[test]
fn m2_runtime_executes_activity_on_create_through_native_bridge() {
    let fixture = TestApk::new();
    let package = ApkLoader::open(&fixture.path).expect("тестовый APK должен разбираться");
    let storage_base = fixture.path.with_extension("storage");
    let context = Context::with_storage_base(
        package.package_name.clone(),
        Arc::new(EmptyResources),
        &[],
        &storage_base,
    )
    .expect("контекст M2");
    let mut runtime = InterpretingDexRuntime::new();

    runtime.load(&package, &context).expect("загрузка DEX");
    assert!(matches!(runtime.status(), DexStatus::Ready { classes: 1, .. }));
    runtime.on_lifecycle(ActivityState::Created).expect("исполнение Activity.onCreate");

    assert!(matches!(
        runtime.status(),
        DexStatus::Running { method, .. } if method == "onCreate()"
    ));
    assert_eq!(
        context.storage().read_file("m2-on-create.txt").expect("маркер bridge"),
        b"Activity.onCreate reached\n"
    );
    let _ = std::fs::remove_dir_all(storage_base);
}

#[derive(Debug)]
struct EmptyResources;

impl ResourceSource for EmptyResources {
    fn contains(&self, _path: &str) -> bool {
        false
    }

    fn load(&self, path: &str) -> Result<Vec<u8>, String> {
        Err(format!("ресурс отсутствует: {path}"))
    }
}

struct TestApk {
    path: PathBuf,
}

impl TestApk {
    fn new() -> Self {
        let unique =
            SystemTime::now().duration_since(UNIX_EPOCH).expect("системное время").as_nanos();
        let path = std::env::temp_dir().join(format!("apksule-m2-{unique}.apk"));
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

    archive.start_file("AndroidManifest.xml", options).expect("manifest entry");
    archive.write_all(MANIFEST.as_bytes()).expect("manifest bytes");
    archive.start_file("classes.dex", options).expect("dex entry");
    archive.write_all(&minimal_activity_dex()).expect("dex bytes");
    archive.finish().expect("закрыть тестовый APK");
}
