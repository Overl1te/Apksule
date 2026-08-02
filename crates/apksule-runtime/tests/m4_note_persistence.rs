//! M4 persistence proof: `SharedPreferences` + `SQLite` note survive reopen.

#![allow(clippy::cast_precision_loss)]

use std::time::{SystemTime, UNIX_EPOCH};

use apksule_compat::{
    AppStorage, PrefValue, SharedPreferencesStore, SqliteRegistry, SqliteValue, UiHost, ViewKind,
    build_minimal_layout_axml, inflate_axml,
};

fn temp_base(label: &str) -> std::path::PathBuf {
    let unique = format!(
        "apksule-m4-{label}-{}",
        SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_nanos()
    );
    std::env::temp_dir().join(unique)
}

#[test]
fn m4_note_persistence_across_reopen() {
    let base = temp_base("persist");
    let storage = AppStorage::for_package_at(&base, "com.omgodse.notally").expect("storage");

    let prefs = SharedPreferencesStore::open(&storage, "NotallyPreferences").expect("prefs");
    prefs.put("firstLaunch", PrefValue::Bool(false)).expect("put");
    prefs.put("theme", PrefValue::String("dark".into())).expect("put");
    prefs.commit().expect("commit");

    let sqlite = SqliteRegistry::new();
    let db = sqlite.open(&storage, "NotallyDatabase").expect("db");
    sqlite
        .exec_sql(
            db,
            "CREATE TABLE IF NOT EXISTS BaseNote (\
                id INTEGER PRIMARY KEY AUTOINCREMENT,\
                title TEXT NOT NULL DEFAULT '',\
                body TEXT NOT NULL DEFAULT '',\
                type TEXT NOT NULL DEFAULT 'NOTE');",
        )
        .expect("schema");
    sqlite
        .insert(
            db,
            "BaseNote",
            &["title".into(), "body".into()],
            &[
                SqliteValue::Text("M4 note".into()),
                SqliteValue::Text("hello persistence".into()),
            ],
        )
        .expect("insert");

    // Re-open storage/prefs/db as a second launch would.
    let storage2 = AppStorage::for_package_at(&base, "com.omgodse.notally").expect("reopen");
    let prefs2 = SharedPreferencesStore::open(&storage2, "NotallyPreferences").expect("prefs2");
    assert_eq!(prefs2.get("theme"), Some(PrefValue::String("dark".into())));
    assert_eq!(prefs2.get("firstLaunch"), Some(PrefValue::Bool(false)));

    let sqlite2 = SqliteRegistry::new();
    let db2 = sqlite2.open(&storage2, "NotallyDatabase").expect("db2");
    let title = sqlite2
        .query_scalar_string(db2, "SELECT title FROM BaseNote ORDER BY id DESC LIMIT 1", &[])
        .expect("query");
    let body = sqlite2
        .query_scalar_string(db2, "SELECT body FROM BaseNote ORDER BY id DESC LIMIT 1", &[])
        .expect("query body");
    assert_eq!(title.as_deref(), Some("M4 note"));
    assert_eq!(body.as_deref(), Some("hello persistence"));

    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn m4_recyclerview_inflates_in_layout_tree() {
    let host = UiHost::new();
    host.set_surface_size(640, 480);
    let root = host.create_view(ViewKind::LinearLayout {
        orientation: apksule_compat::Orientation::Vertical,
        children: Vec::new(),
    });
    let list = host.create_view(ViewKind::RecyclerView { children: Vec::new() });
    let row = host.create_view(ViewKind::TextView { text: "Saved note".into() });
    host.add_child(list, row);
    host.add_child(root, list);
    host.set_content_view(root);
    assert!(host.has_content());
    let snap = host.snapshot();
    assert!(snap.iter().any(|node| matches!(node.kind, ViewKind::RecyclerView { .. })));
    assert!(snap.iter().any(|node| node.kind.text() == Some("Saved note")));

    // Existing AXML path still works for content surfaces.
    let axml = build_minimal_layout_axml("Apksule M4", "Save");
    let host2 = UiHost::new();
    let inflated = inflate_axml(&host2, &axml).expect("inflate");
    host2.set_content_view(inflated);
    assert!(host2.has_content());
}
