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
            None => println!("Apksule {} уже актуален.", current_version()),
            Some(update) => {
                println!(
                    "Доступно обновление: {} → {} ({})",
                    update.current, update.latest, update.tag
                );
                println!("{}", update.html_url);
            }
        },
        Command::Update => {
            let args = relaunch_args_after_update();
            match check_and_update(&args)? {
                UpdateOutcome::UpToDate { current } => {
                    println!("Apksule {current} уже актуален.");
                }
                UpdateOutcome::Updated { from, to } => {
                    println!("Apksule обновлён: {from} → {to}.");
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
    println!("Пакет: {}", package.package_name);
    println!(
        "Версия: {} ({})",
        package.version.name.as_deref().unwrap_or("неизвестно"),
        package.version.code.map_or_else(|| "неизвестно".to_owned(), |code| code.to_string())
    );
    println!("Главная activity: {}", package.main_activity.as_deref().unwrap_or("не объявлена"));
    println!("Activities: {}", package.activities.len());
    println!("Разрешения: {}", package.permissions.len());
    println!("DEX-файлы: {}", package.resources.dex_entries.len());
    println!(
        "resources.arsc: {}",
        if package.resources.has_resource_table { "есть" } else { "нет" }
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
    println!("Apksule {} — лёгкий runtime совместимости APK", current_version());
    println!();
    println!("ИСПОЛЬЗОВАНИЕ:");
    println!("  apksule                    Выбрать APK через нативный диалог");
    println!("  apksule <path.apk>         Разобрать APK и открыть окно runtime");
    println!("  apksule --inspect <apk>    Вывести метаданные манифеста и выйти");
    println!("  apksule --check-update     Проверить GitHub Releases на новую сборку");
    println!("  apksule --update           Скачать и установить последнюю сборку сейчас");
    println!("  apksule --no-update ...    Пропустить автоматическую проверку обновлений");
    println!("  apksule --version          Показать встроенную версию");
    println!();
    println!("Автообновление заменяет apksule.exe в папке установки (без Inno).");
    println!("Отключить: --no-update, APKSULE_SKIP_UPDATE=1 или APKSULE_NO_UPDATE=1.");
}
