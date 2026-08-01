mod update;

use std::error::Error;
use std::path::{Path, PathBuf};

use apksule_apk::{ApkLoader, ApkPackage};
use apksule_runtime::Runtime;
use tracing_subscriber::EnvFilter;

use crate::update::{UpdateOutcome, check_and_update, check_for_update, current_version};

fn main() -> Result<(), Box<dyn Error>> {
    init_logging();

    let options = Options::from_args()?;
    maybe_auto_update(&options);

    match options.command {
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
        Command::CheckUpdate => match check_for_update()? {
            None => println!("Apksule {} is up to date.", current_version()),
            Some(update) => {
                println!(
                    "Update available: {} -> {} ({})",
                    update.current, update.latest, update.tag
                );
                println!("{}", update.html_url);
            }
        },
        Command::Update => {
            let args = relaunch_args_after_update();
            match check_and_update(&args)? {
                UpdateOutcome::UpToDate { current } => {
                    println!("Apksule {current} is already up to date.");
                }
                UpdateOutcome::Updated { from, to } => {
                    println!("Updated Apksule {from} -> {to}.");
                }
            }
        }
        Command::Help => print_help(),
    }
    Ok(())
}

fn maybe_auto_update(options: &Options) {
    if options.skip_update || !options.command.allows_auto_update() || updates_disabled_by_env() {
        return;
    }

    let args = relaunch_args_preserving_command(&options.command);
    match check_and_update(&args) {
        Ok(UpdateOutcome::UpToDate { current }) => {
            tracing::debug!(%current, "no update available");
        }
        Ok(UpdateOutcome::Updated { from, to }) => {
            tracing::info!(%from, %to, "update applied");
        }
        Err(error) => {
            tracing::warn!(%error, "automatic update failed; continuing with current build");
        }
    }
}

fn updates_disabled_by_env() -> bool {
    fn truthy(name: &str) -> bool {
        matches!(std::env::var(name).as_deref(), Ok("1" | "true" | "TRUE" | "yes" | "YES"))
    }
    truthy("APKSULE_SKIP_UPDATE") || truthy("APKSULE_NO_UPDATE")
}

fn relaunch_args_after_update() -> Vec<String> {
    std::env::args().skip(1).filter(|arg| arg != "--update").collect()
}

fn relaunch_args_preserving_command(command: &Command) -> Vec<String> {
    match command {
        Command::Launch(path) => vec![path.display().to_string()],
        Command::Inspect(path) => vec!["--inspect".to_owned(), path.display().to_string()],
        Command::Pick | Command::CheckUpdate | Command::Update | Command::Help => Vec::new(),
    }
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

struct Options {
    skip_update: bool,
    command: Command,
}

enum Command {
    Pick,
    Launch(PathBuf),
    Inspect(PathBuf),
    CheckUpdate,
    Update,
    Help,
}

impl Command {
    fn allows_auto_update(&self) -> bool {
        matches!(self, Self::Pick | Self::Launch(_) | Self::Inspect(_))
    }
}

impl Options {
    fn from_args() -> Result<Self, Box<dyn Error>> {
        let mut skip_update = false;
        let mut positional = Vec::new();

        for arg in std::env::args_os().skip(1) {
            if arg == "--no-update" {
                skip_update = true;
                continue;
            }
            positional.push(arg);
        }

        let command = command_from_positional(positional)?;
        Ok(Self { skip_update, command })
    }
}

fn command_from_positional(positional: Vec<std::ffi::OsString>) -> Result<Command, Box<dyn Error>> {
    let mut arguments = positional.into_iter();
    let Some(first) = arguments.next() else {
        return Ok(Command::Pick);
    };

    if first == "--help" || first == "-h" {
        return Ok(Command::Help);
    }
    if first == "--version" || first == "-V" {
        println!("apksule {}", current_version());
        std::process::exit(0);
    }
    if first == "--check-update" {
        if arguments.next().is_some() {
            return Err("--check-update does not accept extra arguments".into());
        }
        return Ok(Command::CheckUpdate);
    }
    if first == "--update" {
        if arguments.next().is_some() {
            return Err("--update does not accept extra arguments".into());
        }
        return Ok(Command::Update);
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
    println!("Apksule {} - lightweight APK compatibility runtime", current_version());
    println!();
    println!("USAGE:");
    println!("  apksule                    Choose an APK using the native picker");
    println!("  apksule <path.apk>         Inspect and open an APK runtime window");
    println!("  apksule --inspect <apk>    Print manifest/runtime metadata and exit");
    println!("  apksule --check-update     Check GitHub Releases for a newer build");
    println!("  apksule --update           Download and install the latest build now");
    println!("  apksule --no-update ...    Skip the automatic update check");
    println!("  apksule --version          Print the embedded version");
    println!();
    println!("Auto-update replaces apksule.exe in the install folder (not via Inno).");
    println!("Disable with --no-update, APKSULE_SKIP_UPDATE=1, or APKSULE_NO_UPDATE=1.");
}
