# Apksule

Apksule — эксперимент для Windows: лёгкий runtime совместимости Android APK
только на Rust. Это **не** эмулятор Android: нет виртуального устройства,
образа системы, ADB и стороннего Android runtime.

Этап M2 реализует конвейер запуска и минимальное исполнение DEX:

1. выбрать APK через нативный диалог Windows;
2. просмотреть ZIP без распаковки;
3. декодировать бинарный `AndroidManifest.xml`;
4. создать изолированный контекст на пакет;
5. открыть отдельное окно с программной отрисовкой;
6. разобрать `classes.dex` и исполнить lifecycle-методы Activity;
7. передать события ввода к границе DEX;
8. журналировать неподдерживаемые вызовы Android API.

M2 исполняет минимальный самогенерируемый APK вплоть до `Activity.onCreate`.
Это ещё **не** означает совместимость с Notally: Android UI, `resources.arsc`,
AndroidX/Material и Room входят в M3–M4. Поэтому настоящий интерфейс Notally
пока не рисуется.

## Архитектура

```text
apksule.exe
  нативный выбор файла
        |
        v
apksule-apk
  индекс ZIP -> манифест AXML -> метаданные пакета/ресурсы
        |
        v
apksule-runtime ---------------------+
  lifecycle Activity                  |
  цикл событий winit                  v
  поверхность softbuffer/tiny-skia  apksule-compat
  перевод ввода                     Context / Resources
  трейт DexRuntime                  хранилище / заглушки GMS
        |                           журнал неподдержанных API
        v
  apksule-dex
  парсер + интерпретатор DEX (M2)
```

Лаунчер зависит от `apksule-apk` и `apksule-runtime`, но **не** импортирует
crate совместимости напрямую. Android-подобные API отделены от логики хоста.
Окно цели принадлежит runtime; у лаунчера нет дашборда, настроек и оболочки.

## Workspace

- `crates/apksule` — единственный executable: выбор файла и запуск.
- `crates/apksule-apk` — индекс ZIP APK, парсер AXML, загрузка сырых entry.
- `crates/apksule-dex` — безопасный парсер DEX и регистровый интерпретатор.
- `crates/apksule-runtime` — окно, рендер, lifecycle, ввод, граница DEX.
- `crates/apksule-compat` — Context, ресурсы, хранилище, заглушки GMS, лог API.
- `ROADMAP.md` — вехи совместимости и конкретные TODO.

## Требования

- Windows 10 или 11
- Rust 1.96+ с toolchain MSVC

Android SDK, эмулятор, Java, C++, C#, Python и Electron не нужны.

## Сборка и запуск

```powershell
cargo build --release
cargo run -p apksule
```

Без аргументов открывается нативный выбор APK. Для повторяемых тестов:

```powershell
cargo run -p apksule -- "C:\path\to\application.apk"
cargo run -p apksule -- --inspect "C:\path\to\application.apk"
```

Release-сборка: `target\release\apksule.exe`.

## Установщик Windows

Сборка брендированного установщика (нужен [Inno Setup 6](https://jrsoftware.org/isinfo.php)):

```powershell
.\scripts\build-installer.ps1
```

Артефакты в `dist\`:

- `Apksule-Setup-0.1.0.exe` — полный установщик
- `apksule.exe` — портативная копия release-бинарника

Опции установщика (по умолчанию включены):

- ассоциация `.apk` с Apksule (двойной клик открывает окно runtime);
- пункт **Открыть в Apksule** в контекстном меню `.apk`.

## Автообновление

При старте Apksule тихо проверяет GitHub Releases и при наличии новой сборки
заменяет файлы **в папке установленного приложения** (каталог текущего
`apksule.exe`).

Скачивается только портативный `apksule-windows-x64.exe` (и `apksule.ico`,
если есть). Установщик Inno Setup для обновлений **не** используется.

```powershell
apksule --check-update
apksule --update
apksule --no-update .\app.apk
$env:APKSULE_NO_UPDATE = "1"
```

Если папка установки (например Program Files) недоступна для записи, Apksule
планирует отложенную замену на месте (один раз может появиться UAC) и
перезапускается. Сбой обновления не блокирует запуск APK.

## CI / релизы

GitHub Actions:

- `CI` — тесты/clippy/release-сборка Windows + Inno Setup; self-hosted Linux
  проверяет library-crate’ы.
- `Auto Tag` — при каждом пуше в `master`/`main` создаёт следующий semver-тег
  и запускает `Release`.
- `Release` — по тегам `v*` (или вручную) публикует Windows-артефакты в
  GitHub Releases с checksum и описанием.

### Автоверсия

Тег считается от предыдущего `vX.Y.Z` и размера/смысла изменений:

| Шаг | Результат | Когда |
|------|-----------|--------|
| patch | `v0.1.0` → `v0.1.1` | мелочи / доки / CI / `fix:` |
| minor | `v0.1.1` → `v0.2.0` | крупный код / `feat:` |
| major | `v1.0.0` | **только вручную** |

Переопределения в сообщении коммита:

- `[patch]` / `bump:patch` — принудительный patch
- `[minor]` / `bump:minor` — принудительный minor
- `[no-tag]` / `[skip-tag]` — без тега
- `[major]` / `bump:major` — игнорируется (создайте `v1.0.0` сами)

Major намеренно ручной: когда проект выходит из 0.x —
`git tag -a v1.0.0 -m "..." && git push origin v1.0.0`.

### Эталон совместимости: Notally

Целевой APK для M4 — Notally 6.2 с F-Droid:

```powershell
$apk = "$env:TEMP\Notally.apk"
Invoke-WebRequest `
  "https://f-droid.org/repo/com.omgodse.notally_57.apk" `
  -OutFile $apk
cargo run -p apksule -- --inspect $apk
cargo run -p apksule -- $apk
```

Ожидаемые значения inspection: пакет `com.omgodse.notally`, launcher activity
`com.omgodse.notally.activities.MainActivity`, один DEX и скомпилированная
таблица ресурсов. M2 пытается разобрать и запустить DEX, но останавливается на
ещё не реализованной поверхности Android/AndroidX. Окно остаётся диагностикой
runtime, а не UI Notally.

Минимальный критерий M2 проверяется без Android SDK:

```powershell
cargo test -p apksule-runtime --test m2_minimal_apk
```

Тест сам собирает APK в памяти, исполняет `Activity.onCreate` и проверяет
side effect native bridge в песочнице.

## Хранилище и логи

У каждого пакета своя песочница:

```text
%APPDATA%\Apksule\apps\<package>\
  files\
  cache\
  databases\
  logs\unsupported-api.log
```

Относительные пути хранилища отклоняют корень и выход через `..`. Каждая
заглушка совместимости пишет класс, метод, время и деталь fallback в
`unsupported-api.log`; то же событие уходит в `tracing`.

## Текущая поверхность совместимости

- метаданные ZIP APK и загрузка сырых entry по требованию
- декодирование бинарного AXML и обычного XML-манифеста
- launcher Activity, компоненты, permissions, SDK и версия
- lifecycle `Created -> Started -> Resumed -> Paused -> Stopped -> Destroyed`
- перевод pointer, wheel и клавиатуры
- сырой доступ к `assets/`, `res/` и `resources.arsc`
- каталоги files/cache/databases/log на пакет
- обнаружение Google Play Services и детерминированные заглушки
- окно цели через `winit`, `softbuffer` и `tiny-skia`
- проверенный парсер DEX 035–041 с checksum/SHA-1 и проверкой границ
- регистровая VM: базовые опкоды, invoke, объекты/массивы, поля и исключения
- class loading, `<clinit>`, наследование, virtual dispatch и лимиты VM
- запуск `Activity.onCreate` минимального APK через native/framework bridge

Android UI для отрисовки интерфейса самого APK — следующий этап в
[ROADMAP.md](ROADMAP.md).

## Лицензия

На выбор: Apache-2.0 или MIT.
