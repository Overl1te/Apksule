# Apksule compatibility roadmap

The roadmap is intentionally app-driven. Each milestone adds only the Android
surface required by the current reference APK and keeps unsupported behavior
observable.

## M1 — APK-to-window pipeline

- [x] Native APK picker and direct path launch
- [x] ZIP indexing without extraction
- [x] Binary AndroidManifest.xml parsing
- [x] Package, component, permission, SDK, DEX, and resource inventory
- [x] Dedicated software-rendered window
- [x] Activity lifecycle state machine
- [x] Pointer and keyboard input translation
- [x] Context, raw resources, and per-package storage
- [x] GMS detection and deterministic stubs
- [x] Unsupported API log
- [x] `DexRuntime` seam and explicit M1 stub

## M2 — Minimal Rust DEX interpreter

Goal: execute a purpose-built, dependency-free test APK before attempting
Notally.

- [ ] Parse DEX header, map, string/type/proto/field/method/class tables
- [ ] Validate checksums, section bounds, and instruction offsets
- [ ] Implement register frames and method invocation
- [ ] Implement the initial opcode set:
  - constants, moves, returns, branches, comparisons
  - instance/static fields
  - object and array allocation/access
  - invoke virtual/direct/static/interface
  - integer arithmetic and conversions
  - exception tables and `throw`
- [ ] Implement class loading, initialization, inheritance, and virtual dispatch
- [ ] Add deterministic instruction/time/heap limits
- [ ] Bridge selected Java core classes (`Object`, `String`, collections)
- [ ] Route unresolved native/framework methods through `ApiLogger`
- [ ] Run a Rust fixture suite generated from documented DEX byte sequences
- [ ] Launch a minimal open-source test APK and execute its `Activity.onCreate`

Exit criterion: a small APK executes its own DEX code and reaches a framework
method through the compatibility bridge.

## M3 — Android UI and resource subset

Goal: let a simple APK draw and interact with its own UI in the Apksule window.

- [ ] Decode the needed `resources.arsc` table chunks and configurations
- [ ] Resolve resource IDs, strings, colors, dimensions, styles, and drawables
- [ ] Inflate binary layout XML
- [ ] Implement a minimal View tree (`View`, `ViewGroup`, `TextView`,
      `EditText`, `Button`, linear/frame/constraint subset)
- [ ] Implement measure/layout/draw and invalidation scheduling
- [ ] Send translated MotionEvent/KeyEvent objects into View dispatch
- [ ] Add focus, text input, clipboard, and minimal IME delegation
- [ ] Implement Activity `setContentView` and Window/DecorView bridge
- [ ] Add bitmap and vector drawable subsets
- [ ] Replace the M1 diagnostic surface as soon as the APK submits its first frame

Exit criterion: the test APK owns every visible pixel after launch and supports
basic click and text-input interaction.

## M4 — Notally compatibility slice

Goal: render and use the basic note list/editor path of Notally; unsupported
features must fail gracefully.

- [ ] Inventory Notally's actually reached AndroidX/Material APIs from logs
- [ ] Implement the required AndroidX lifecycle/fragment subset
- [ ] Add RecyclerView behavior needed by the note list
- [ ] Add a Material widget/style subset only where reached
- [ ] Implement SharedPreferences
- [ ] Implement SQLite operations and the minimum Room-generated call surface
- [ ] Add ContentProvider/file URI handling for local attachments
- [ ] Add timers, Handler/Looper, and simple background task dispatch
- [ ] Stub reminders, widgets, PDF export, audio, and unsupported media paths
- [ ] Add golden screenshots and scripted note create/edit/delete tests

Exit criterion: Notally starts, displays its own list/editor UI, and persists a
basic text note across launches.

## Later milestones

- [ ] Broaden DEX opcodes and verifier behavior from real compatibility traces
- [ ] Multi-dex class loading and split APK metadata
- [ ] More resource qualifiers, locales, density handling, and themes
- [ ] Networking behind an explicit permission and policy boundary
- [ ] Optional JIT/AOT experiments only after interpreter correctness
- [ ] Crash reports containing lifecycle, API-miss, and bytecode traces
- [ ] Per-app compatibility profiles without coupling them to host UI

Non-goals remain unchanged: no virtual Android device, Linux kernel emulation,
Android system image, Google Play installation, or hidden delegation to an
existing Android emulator.
