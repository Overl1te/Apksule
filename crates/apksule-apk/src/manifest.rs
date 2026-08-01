use libaxml::{Document, Element, Node};
use serde::{Deserialize, Serialize};

use crate::error::{ApkError, Result};

/// Version attributes declared by the APK.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApkVersion {
    pub name: Option<String>,
    pub code: Option<u32>,
}

/// SDK bounds declared in `<uses-sdk>`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SdkRequirements {
    pub min_sdk: Option<u32>,
    pub target_sdk: Option<u32>,
    pub max_sdk: Option<u32>,
}

/// One activity or activity-alias declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityInfo {
    pub name: String,
    pub target_activity: Option<String>,
    pub label: Option<String>,
    pub exported: Option<bool>,
    pub enabled: bool,
    pub launcher: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComponentKind {
    Service,
    Receiver,
    Provider,
}

/// Non-activity component metadata used by compatibility detection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentInfo {
    pub name: String,
    pub kind: ComponentKind,
    pub enabled: bool,
}

/// Parsed subset of AndroidManifest.xml needed to bootstrap the MVP runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestInfo {
    pub package_name: String,
    pub version: ApkVersion,
    pub sdk: SdkRequirements,
    pub application_label: Option<String>,
    pub main_activity: Option<String>,
    pub permissions: Vec<String>,
    pub activities: Vec<ActivityInfo>,
    pub components: Vec<ComponentInfo>,
}

/// Parse either compiled Android AXML or plain XML (the latter is useful for fixtures).
pub fn parse_manifest(bytes: &[u8]) -> Result<ManifestInfo> {
    let document = match Document::from_axml(bytes) {
        Ok(document) => document,
        Err(axml_error) => Document::from_xml(bytes).map_err(|xml_error| {
            ApkError::ManifestDecode { axml: axml_error.to_string(), xml: xml_error.to_string() }
        })?,
    };

    parse_document(&document)
}

fn parse_document(document: &Document) -> Result<ManifestInfo> {
    let manifest = document.find_element("manifest").ok_or(ApkError::MissingManifestElement)?;
    let package_name = manifest.attr_str("package").ok_or(ApkError::MissingPackageName)?;

    let version = ApkVersion {
        name: manifest.attr_str("versionName"),
        code: manifest.attr_str("versionCode").and_then(|value| parse_u32(&value)),
    };

    let sdk = manifest.find_element("uses-sdk").map_or_else(SdkRequirements::default, |uses_sdk| {
        SdkRequirements {
            min_sdk: attr_u32(uses_sdk, "minSdkVersion"),
            target_sdk: attr_u32(uses_sdk, "targetSdkVersion"),
            max_sdk: attr_u32(uses_sdk, "maxSdkVersion"),
        }
    });

    let permissions = collect_permissions(document);
    let application = manifest.find_element("application");
    let application_label = application.and_then(|element| element.attr_str("label"));

    let mut activities = Vec::new();
    if let Some(application) = application {
        collect_activities(application, &package_name, "activity", &mut activities);
        collect_activities(application, &package_name, "activity-alias", &mut activities);
    }
    let main_activity = activities
        .iter()
        .find(|activity| activity.launcher && activity.enabled)
        .map(|activity| activity.name.clone());

    let mut components = Vec::new();
    if let Some(application) = application {
        collect_components(
            application,
            &package_name,
            "service",
            ComponentKind::Service,
            &mut components,
        );
        collect_components(
            application,
            &package_name,
            "receiver",
            ComponentKind::Receiver,
            &mut components,
        );
        collect_components(
            application,
            &package_name,
            "provider",
            ComponentKind::Provider,
            &mut components,
        );
    }

    Ok(ManifestInfo {
        package_name,
        version,
        sdk,
        application_label,
        main_activity,
        permissions,
        activities,
        components,
    })
}

fn collect_permissions(document: &Document) -> Vec<String> {
    let mut permissions: Vec<_> = document
        .find_elements("uses-permission")
        .into_iter()
        .chain(document.find_elements("uses-permission-sdk-23"))
        .filter_map(|element| element.attr_str("name"))
        .collect();
    permissions.sort();
    permissions.dedup();
    permissions
}

fn collect_activities(
    application: &Element,
    package: &str,
    tag: &str,
    output: &mut Vec<ActivityInfo>,
) {
    for element in direct_children(application, tag) {
        let Some(raw_name) = element.attr_str("name") else {
            continue;
        };
        let target_activity =
            element.attr_str("targetActivity").map(|name| qualify_class_name(package, &name));
        output.push(ActivityInfo {
            name: qualify_class_name(package, &raw_name),
            target_activity,
            label: element.attr_str("label"),
            exported: element.attr_str("exported").and_then(|value| value.parse().ok()),
            enabled: element
                .attr_str("enabled")
                .and_then(|value| value.parse().ok())
                .unwrap_or(true),
            launcher: is_launcher_activity(element),
        });
    }
}

fn collect_components(
    application: &Element,
    package: &str,
    tag: &str,
    kind: ComponentKind,
    output: &mut Vec<ComponentInfo>,
) {
    output.extend(direct_children(application, tag).filter_map(|element| {
        let name = element.attr_str("name")?;
        Some(ComponentInfo {
            name: qualify_class_name(package, &name),
            kind,
            enabled: element
                .attr_str("enabled")
                .and_then(|value| value.parse().ok())
                .unwrap_or(true),
        })
    }));
}

fn direct_children<'a>(element: &'a Element, tag: &'a str) -> impl Iterator<Item = &'a Element> {
    element.children.iter().filter_map(move |node| match node {
        Node::Element(child) if child.name.as_ref() == tag => Some(child),
        _ => None,
    })
}

fn is_launcher_activity(activity: &Element) -> bool {
    direct_children(activity, "intent-filter").any(|filter| {
        let has_main = direct_children(filter, "action")
            .any(|action| action.attr_str("name").as_deref() == Some("android.intent.action.MAIN"));
        let has_launcher = direct_children(filter, "category").any(|category| {
            category.attr_str("name").as_deref() == Some("android.intent.category.LAUNCHER")
        });
        has_main && has_launcher
    })
}

fn qualify_class_name(package: &str, name: &str) -> String {
    if let Some(suffix) = name.strip_prefix('.') {
        format!("{package}.{suffix}")
    } else if name.contains('.') {
        name.to_owned()
    } else {
        format!("{package}.{name}")
    }
}

fn attr_u32(element: &Element, name: &str) -> Option<u32> {
    element.attr_str(name).and_then(|value| parse_u32(&value))
}

fn parse_u32(value: &str) -> Option<u32> {
    value
        .strip_prefix("0x")
        .map_or_else(|| value.parse().ok(), |hex| u32::from_str_radix(hex, 16).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"
        <manifest xmlns:android="http://schemas.android.com/apk/res/android"
            package="org.example.notes" android:versionCode="7" android:versionName="1.2">
            <uses-sdk android:minSdkVersion="23" android:targetSdkVersion="35" />
            <uses-permission android:name="android.permission.POST_NOTIFICATIONS" />
            <application android:label="Example Notes">
                <activity android:name=".MainActivity" android:exported="true">
                    <intent-filter>
                        <action android:name="android.intent.action.MAIN" />
                        <category android:name="android.intent.category.LAUNCHER" />
                    </intent-filter>
                </activity>
                <service android:name="SyncService" android:enabled="false" />
            </application>
        </manifest>
    "#;

    #[test]
    fn parses_launcher_and_qualifies_component_names() {
        let manifest = parse_manifest(MANIFEST.as_bytes()).expect("manifest should parse");

        assert_eq!(manifest.package_name, "org.example.notes");
        assert_eq!(manifest.version.code, Some(7));
        assert_eq!(manifest.sdk.min_sdk, Some(23));
        assert_eq!(manifest.main_activity.as_deref(), Some("org.example.notes.MainActivity"));
        assert_eq!(manifest.activities[0].exported, Some(true));
        assert_eq!(manifest.components[0].name, "org.example.notes.SyncService");
        assert!(!manifest.components[0].enabled);
    }
}
