use std::error::Error;
use std::path::{Path, PathBuf};

use apksule_apk::{ApkLoader, ApkPackage};
use apksule_runtime::Runtime;
use tracing_subscriber::EnvFilter;

fn main() -> Result<(), Box<dyn Error>> {
    init_logging();

    match command_from_args()? {
        Command::Pick => {
            let Some(path) = rfd::FileDialog::new()
                .set_title("Choose an Android APK")
                .add_filter("Android package", &["apk"])
                .pick_file()
            else {
                tracing::info!("APK selection cancelled");
                return Ok(());
            };
            launch(&path)?;
        }
        Command::Launch(path) => launch(&path)?,
        Command::Inspect(path) => {
            let package = ApkLoader::open(path)?;
            print_package(&package);
        }
        Command::Help => print_help(),
    }
    Ok(())
}

fn launch(path: &Path) -> Result<(), Box<dyn Error>> {
    tracing::info!(apk = %path.display(), "opening APK");
    let package = ApkLoader::open(path)?;
    tracing::info!(
        package = %package.package_name,
        main_activity = ?package.main_activity,
        dex_files = package.resources.dex_entries.len(),
        "APK inspected"
    );
    Runtime::launch(package)?;
    Ok(())
}

fn print_package(package: &ApkPackage) {
    println!("APK: {}", package.source_path.display());
    println!("Package: {}", package.package_name);
    println!(
        "Version: {} ({})",
        package.version.name.as_deref().unwrap_or("unknown"),
        package.version.code.map_or_else(|| "unknown".to_owned(), |code| code.to_string())
    );
    println!("Main activity: {}", package.main_activity.as_deref().unwrap_or("not declared"));
    println!("Activities: {}", package.activities.len());
    println!("Permissions: {}", package.permissions.len());
    println!("DEX files: {}", package.resources.dex_entries.len());
    println!(
        "resources.arsc: {}",
        if package.resources.has_resource_table { "present" } else { "missing" }
    );
}

fn init_logging() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("apksule=info,apksule_runtime=info"));
    let _ =
        tracing_subscriber::fmt().with_env_filter(filter).with_target(false).compact().try_init();
}

enum Command {
    Pick,
    Launch(PathBuf),
    Inspect(PathBuf),
    Help,
}

fn command_from_args() -> Result<Command, Box<dyn Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let Some(first) = arguments.next() else {
        return Ok(Command::Pick);
    };

    if first == "--help" || first == "-h" {
        return Ok(Command::Help);
    }
    if first == "--inspect" {
        let path = arguments.next().ok_or("--inspect requires a path to an APK")?;
        if arguments.next().is_some() {
            return Err("--inspect accepts exactly one APK path".into());
        }
        return Ok(Command::Inspect(path.into()));
    }
    if arguments.next().is_some() {
        return Err("pass one APK path, --inspect <apk>, or no arguments".into());
    }
    Ok(Command::Launch(first.into()))
}

fn print_help() {
    println!("Apksule - lightweight APK compatibility runtime");
    println!();
    println!("USAGE:");
    println!("  apksule                 Choose an APK using the native picker");
    println!("  apksule <path.apk>      Inspect and open an APK runtime window");
    println!("  apksule --inspect <apk> Print manifest/runtime metadata and exit");
}
