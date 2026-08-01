# Apksule

Apksule is a Windows-first, Rust-only experiment for running Android APKs in a
small compatibility runtime. It is not an Android emulator: there is no virtual
device, Android system image, ADB server, or bundled third-party Android runtime.

The M1 MVP implements the launch pipeline around DEX execution:

1. choose an APK with the native Windows file picker;
2. inspect the ZIP without extracting it;
3. decode binary `AndroidManifest.xml`;
4. create an isolated per-package context;
5. open a dedicated software-rendered window;
6. deliver Activity lifecycle and translated input events to the DEX boundary;
7. record unsupported Android API calls.

M1 deliberately uses a `StubDexRuntime`. It proves the APK-to-window pipeline,
but it does **not** execute `classes.dex` and therefore cannot render Notally's
actual Android UI yet. A Rust DEX interpreter is the M2 milestone.

## Architecture

```text
apksule.exe
  native file picker
        |
        v
apksule-apk
  ZIP index -> AXML manifest -> package metadata/resources
        |
        v
apksule-runtime ----------------------+
  Activity lifecycle                   |
  winit event loop                     v
  softbuffer/tiny-skia surface   apksule-compat
  input translation              Context / Resources
  DexRuntime trait               storage / GMS shim
        |                        unsupported API log
        v
  StubDexRuntime (M1)
  Rust DEX interpreter (M2)
```

The launcher depends on `apksule-apk` and `apksule-runtime`, but never imports
the compatibility crate directly. Android-like APIs are isolated from host
logic. The runtime owns the target window and the launcher has no dashboard,
settings page, or shell screen.

## Workspace

- `crates/apksule` — the only executable; picker and launch handoff.
- `crates/apksule-apk` — APK ZIP index, AXML manifest parser, raw entry loader.
- `crates/apksule-runtime` — window, renderer, lifecycle, input, DEX boundary.
- `crates/apksule-compat` — Context, resources, storage, GMS stubs, API logging.
- `ROADMAP.md` — compatibility milestones and concrete TODOs.

## Requirements

- Windows 10 or 11
- Rust 1.96 or newer with the MSVC toolchain

No Android SDK, emulator, Java, C++, C#, Python, or Electron is required.

## Build and run

```powershell
cargo build --release
cargo run -p apksule
```

Running without arguments opens the native APK picker. For repeatable testing:

```powershell
cargo run -p apksule -- "C:\path\to\application.apk"
cargo run -p apksule -- --inspect "C:\path\to\application.apk"
```

The release executable is `target\release\apksule.exe`.

## Windows installer

Build the branded installer (requires [Inno Setup 6](https://jrsoftware.org/isinfo.php)):

```powershell
.\scripts\build-installer.ps1
```

Artifacts land in `dist\`:

- `Apksule-Setup-0.1.0.exe` — full installer
- `apksule.exe` — portable copy of the release binary

Installer options (enabled by default):

- associate `.apk` with Apksule so double-click opens the runtime window;
- add **Open with Apksule** to the `.apk` context menu.

## Auto-update

On startup Apksule checks GitHub Releases for a newer
`apksule-windows-x64.exe`, verifies `SHA256SUMS.txt` when present, replaces the
running binary, and relaunches with the same arguments.

```powershell
apksule --check-update
apksule --update
apksule --no-update .\app.apk
$env:APKSULE_NO_UPDATE = "1"
```

Update failures never block APK launch; they are logged and the current build
continues.

## CI / Releases

GitHub Actions:

- `CI` — Windows test/clippy/release build + Inno Setup packaging; Linux
  self-hosted runner validates library crates.
- `Auto Tag` — on every push to `master`/`main`, creates the next semver tag
  and dispatches `Release`.
- `Release` — on `v*` tags (or manual dispatch) publishes Windows assets to
  GitHub Releases with checksums and release notes.

### Auto versioning

Tags are computed from the previous `vX.Y.Z` and the size/intent of the change:

| Bump | Result | When |
|------|--------|------|
| patch | `v0.1.0` → `v0.1.1` | small / docs / CI / `fix:` |
| minor | `v0.1.1` → `v0.2.0` | large code churn / `feat:` |
| major | `v1.0.0` | **manual only** |

Overrides in the commit message:

- `[patch]` / `bump:patch` — force patch
- `[minor]` / `bump:minor` — force minor
- `[no-tag]` / `[skip-tag]` — skip tagging
- `[major]` / `bump:major` — ignored (create `v1.0.0` yourself)

Major stays manual on purpose: publish `v1.0.0` with
`git tag -a v1.0.0 -m "..." && git push origin v1.0.0` when you decide the
project is out of 0.x.

### First reference APK: Notally

Notally 6.2 from F-Droid is the M1 reference:

```powershell
$apk = "$env:TEMP\Notally.apk"
Invoke-WebRequest `
  "https://f-droid.org/repo/com.omgodse.notally_57.apk" `
  -OutFile $apk
cargo run -p apksule -- --inspect $apk
cargo run -p apksule -- $apk
```

Expected M1 inspection values include package
`com.omgodse.notally`, launcher activity
`com.omgodse.notally.activities.MainActivity`, one DEX file, and a compiled
resource table. The window then shows the temporary M1 launch surface. It is a
runtime diagnostic surface, not a reimplementation of Notally's UI.

## Storage and logs

Each package receives a sandbox:

```text
%APPDATA%\Apksule\apps\<package>\
  files\
  cache\
  databases\
  logs\unsupported-api.log
```

Relative storage paths reject root and parent traversal. Every compatibility
stub records class, method, timestamp, and fallback detail in
`unsupported-api.log`; the same event is emitted through `tracing`.

## Current compatibility surface

- APK ZIP metadata and on-demand raw entry loading
- binary AXML and plain XML manifest decoding
- launcher Activity, components, permissions, SDK and version metadata
- `Created -> Started -> Resumed -> Paused -> Stopped -> Destroyed` lifecycle
- pointer, wheel, and keyboard event translation
- raw `assets/`, `res/`, and `resources.arsc` access
- per-package files/cache/databases/log directories
- Google Play Services reference detection and deterministic stub responses
- software-rendered target window through `winit`, `softbuffer`, and `tiny-skia`

See [ROADMAP.md](ROADMAP.md) for the DEX and Android UI work required to render
the APK's own interface.

## License

Licensed under either Apache-2.0 or MIT, at your option.
