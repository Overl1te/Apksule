#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]

//! Graphical host shell: open APK, settings, update check — without a console.

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::Arc;

use softbuffer::{Context as SoftContext, Surface};
use tiny_skia::{Color, Paint, Pixmap, Rect, Transform};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use crate::settings::{HostSettings, logs_dir};
use crate::update::{check_for_update, current_version};

pub fn run_host_shell() -> Result<Option<PathBuf>, Box<dyn std::error::Error>> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);

    let soft = SoftContext::new(event_loop.owned_display_handle())?;
    let mut app = HostApp {
        soft,
        window: None,
        surface: None,
        settings: HostSettings::load(),
        status: format!("Версия {}", current_version()),
        cursor: PhysicalPosition::new(0.0, 0.0),
        pending_apk: None,
        quit: false,
        layout: Layout::default(),
    };
    event_loop.run_app(&mut app)?;
    Ok(app.pending_apk.take())
}

struct HostApp {
    soft: SoftContext<winit::event_loop::OwnedDisplayHandle>,
    window: Option<Arc<Window>>,
    surface: Option<Surface<winit::event_loop::OwnedDisplayHandle, Arc<Window>>>,
    settings: HostSettings,
    status: String,
    cursor: PhysicalPosition<f64>,
    pending_apk: Option<PathBuf>,
    quit: bool,
    layout: Layout,
}

#[derive(Debug, Clone, Copy, Default)]
struct RectF {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl RectF {
    const fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x <= self.x + self.w && y <= self.y + self.h
    }
}

#[derive(Debug, Clone, Default)]
struct Layout {
    open_apk: RectF,
    toggle_update: RectF,
    check_update: RectF,
    open_logs: RectF,
}

impl HostApp {
    fn ensure_window(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        if self.window.is_none() {
            let attributes = Window::default_attributes()
                .with_title("Apksule")
                .with_inner_size(LogicalSize::new(720.0, 480.0))
                .with_min_inner_size(LogicalSize::new(560.0, 400.0));
            let window =
                Arc::new(event_loop.create_window(attributes).map_err(|error| error.to_string())?);
            self.surface =
                Some(Surface::new(&self.soft, window.clone()).map_err(|error| error.to_string())?);
            self.window = Some(window);
        }
        Ok(())
    }

    fn rebuild_layout(&mut self, width: u32, height: u32) {
        let margin = 36.0_f32;
        let content_w = (width as f32 - margin * 2.0).max(200.0);
        let mut y = 150.0;
        self.layout.open_apk = RectF { x: margin, y, w: content_w.min(320.0), h: 44.0 };
        y += 72.0;
        self.layout.toggle_update = RectF { x: margin, y, w: 44.0, h: 28.0 };
        y += 56.0;
        self.layout.check_update = RectF { x: margin, y, w: 260.0, h: 40.0 };
        y += 52.0;
        self.layout.open_logs = RectF { x: margin, y, w: 260.0, h: 40.0 };
        let _ = height;
    }

    fn render(&mut self) -> Result<(), String> {
        let Some(window) = self.window.as_ref() else {
            return Ok(());
        };
        let size = window.inner_size();
        let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        else {
            return Ok(());
        };
        self.rebuild_layout(size.width, size.height);
        let frame =
            paint_host(size.width, size.height, &self.settings, &self.status, &self.layout)?;
        let surface = self.surface.as_mut().ok_or("нет surface")?;
        surface.resize(width, height).map_err(|error| error.to_string())?;
        let mut buffer = surface.buffer_mut().map_err(|error| error.to_string())?;
        buffer.copy_from_slice(&frame);
        buffer.present().map_err(|error| error.to_string())
    }

    fn handle_click(&mut self, x: f32, y: f32) {
        if self.layout.open_apk.contains(x, y) {
            if let Some(path) = rfd::FileDialog::new()
                .set_title("Выберите Android APK")
                .add_filter("Пакет Android", &["apk"])
                .pick_file()
            {
                self.pending_apk = Some(path);
                self.quit = true;
                if let Some(window) = &self.window {
                    window.set_visible(false);
                }
            }
            return;
        }

        if self.layout.toggle_update.contains(x, y) {
            self.settings.auto_update = !self.settings.auto_update;
            if let Err(error) = self.settings.save() {
                self.status = format!("Не удалось сохранить настройки: {error}");
            } else if self.settings.auto_update {
                "Автообновление включено".clone_into(&mut self.status);
            } else {
                "Автообновление выключено".clone_into(&mut self.status);
            }
            if let Some(window) = &self.window {
                window.request_redraw();
            }
            return;
        }

        if self.layout.check_update.contains(x, y) {
            "Проверка обновлений...".clone_into(&mut self.status);
            if let Some(window) = &self.window {
                window.request_redraw();
            }
            self.status = match check_for_update() {
                Ok(None) => format!("Apksule {} уже актуален", current_version()),
                Ok(Some(update)) => {
                    format!("Доступно {} → {} ({})", update.current, update.latest, update.tag)
                }
                Err(error) => format!("Проверка не удалась: {error}"),
            };
            if let Some(window) = &self.window {
                window.request_redraw();
            }
            return;
        }

        if self.layout.open_logs.contains(x, y) {
            let path = logs_dir();
            let _ = std::fs::create_dir_all(&path);
            self.status = match open_in_explorer(&path) {
                Ok(()) => format!("Открыта папка {}", path.display()),
                Err(error) => format!("Не удалось открыть логи: {error}"),
            };
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }
}

impl ApplicationHandler for HostApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(error) = self.ensure_window(event_loop) {
            tracing::error!(%error, "host window failed");
            event_loop.exit();
            return;
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        if window.id() != window_id {
            return;
        }

        match event {
            WindowEvent::CloseRequested => {
                self.quit = true;
                event_loop.exit();
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = position;
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                let logical: LogicalPosition<f32> = self.cursor.to_logical(window.scale_factor());
                self.handle_click(logical.x, logical.y);
                if self.quit {
                    event_loop.exit();
                }
            }
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.render() {
                    tracing::error!(%error, "host redraw failed");
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }
}

fn open_in_explorer(path: &std::path::Path) -> Result<(), String> {
    std::process::Command::new("explorer")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn paint_host(
    width: u32,
    height: u32,
    settings: &HostSettings,
    status: &str,
    layout: &Layout,
) -> Result<Vec<u32>, String> {
    let mut pixmap =
        Pixmap::new(width, height).ok_or_else(|| format!("bad size {width}x{height}"))?;
    pixmap.fill(Color::from_rgba8(14, 18, 24, 255));

    // Subtle vertical atmosphere — not flat white.
    let mut wash = Paint::default();
    wash.set_color_rgba8(28, 46, 48, 255);
    fill_rect(&mut pixmap, 0.0, 0.0, width as f32, height as f32 * 0.42, &wash);

    let mut accent = Paint::default();
    accent.set_color_rgba8(72, 201, 176, 255);
    fill_rect(&mut pixmap, 0.0, 0.0, width as f32, 6.0, &accent);

    let margin = 36.0;
    draw_text(&mut pixmap, "APKSULE", margin, 34.0, 5.0, (72, 201, 176, 255));
    draw_text(&mut pixmap, "ХОСТ СОВМЕСТИМОСТИ APK", margin, 78.0, 2.0, (168, 180, 196, 255));
    draw_text(
        &mut pixmap,
        "Откройте APK или настройте автообновление.",
        margin,
        108.0,
        2.0,
        (210, 218, 228, 255),
    );

    draw_button(&mut pixmap, layout.open_apk, "ОТКРЫТЬ APK", true);
    draw_toggle(&mut pixmap, layout.toggle_update, settings.auto_update);
    draw_text(
        &mut pixmap,
        if settings.auto_update {
            "АВТООБНОВЛЕНИЕ: ВКЛ"
        } else {
            "АВТООБНОВЛЕНИЕ: ВЫКЛ"
        },
        layout.toggle_update.x + 58.0,
        layout.toggle_update.y + 6.0,
        2.0,
        (226, 232, 240, 255),
    );
    draw_button(&mut pixmap, layout.check_update, "ПРОВЕРИТЬ ОБНОВЛЕНИЯ", false);
    draw_button(&mut pixmap, layout.open_logs, "ОТКРЫТЬ ПАПКУ ЛОГОВ", false);

    draw_text(
        &mut pixmap,
        &status.to_uppercase(),
        margin,
        height as f32 - 48.0,
        2.0,
        (150, 164, 184, 255),
    );

    Ok(pixmap
        .data()
        .chunks_exact(4)
        .map(|pixel| (u32::from(pixel[0]) << 16) | (u32::from(pixel[1]) << 8) | u32::from(pixel[2]))
        .collect())
}

fn draw_button(pixmap: &mut Pixmap, rect: RectF, label: &str, primary: bool) {
    let mut fill = Paint::default();
    if primary {
        fill.set_color_rgba8(42, 120, 108, 255);
    } else {
        fill.set_color_rgba8(32, 40, 54, 255);
    }
    fill_rect(pixmap, rect.x, rect.y, rect.w, rect.h, &fill);
    let mut border = Paint::default();
    border.set_color_rgba8(72, 201, 176, 255);
    fill_rect(pixmap, rect.x, rect.y, rect.w, 2.0, &border);
    draw_text(pixmap, label, rect.x + 16.0, rect.y + 14.0, 2.0, (236, 242, 248, 255));
}

fn draw_toggle(pixmap: &mut Pixmap, rect: RectF, on: bool) {
    let mut track = Paint::default();
    track.set_color_rgba8(
        if on { 42 } else { 48 },
        if on { 120 } else { 54 },
        if on { 108 } else { 64 },
        255,
    );
    fill_rect(pixmap, rect.x, rect.y, rect.w, rect.h, &track);
    let mut knob = Paint::default();
    knob.set_color_rgba8(236, 242, 248, 255);
    let knob_x = if on { rect.x + rect.w - 22.0 } else { rect.x + 4.0 };
    fill_rect(pixmap, knob_x, rect.y + 4.0, 18.0, rect.h - 8.0, &knob);
}

fn fill_rect(pixmap: &mut Pixmap, x: f32, y: f32, width: f32, height: f32, paint: &Paint<'_>) {
    if let Some(rect) = Rect::from_xywh(x, y, width, height) {
        pixmap.fill_rect(rect, paint, Transform::identity(), None);
    }
}

fn draw_text(
    pixmap: &mut Pixmap,
    text: &str,
    start_x: f32,
    y: f32,
    scale: f32,
    color: (u8, u8, u8, u8),
) {
    let mut paint = Paint::default();
    paint.set_color_rgba8(color.0, color.1, color.2, color.3);
    let mut x = start_x;
    let advance = 6.0 * scale;
    for character in text.chars() {
        let mapped = match character {
            'а'..='я' | 'ё' => {
                // Host labels are intentionally uppercase; map lowercase Cyrillic.
                char::from_u32(u32::from(character) - 32).unwrap_or(character)
            }
            other => other.to_ascii_uppercase(),
        };
        if x + (5.0 * scale) >= pixmap.width() as f32 {
            break;
        }
        let glyph = glyph(mapped);
        for (row, bits) in glyph.into_iter().enumerate() {
            for column in 0..5 {
                if bits & (1 << (4 - column)) != 0 {
                    fill_rect(
                        pixmap,
                        x + (column as f32 * scale),
                        y + (row as f32 * scale),
                        scale,
                        scale,
                        &paint,
                    );
                }
            }
        }
        x += advance;
    }
}

#[rustfmt::skip]
#[allow(clippy::match_same_arms)]
fn glyph(character: char) -> [u8; 7] {
    match character {
        'A' | 'А' => [0b01110,0b10001,0b10001,0b11111,0b10001,0b10001,0b10001],
        'B' | 'В' => [0b11110,0b10001,0b10001,0b11110,0b10001,0b10001,0b11110],
        'C' | 'С' => [0b01111,0b10000,0b10000,0b10000,0b10000,0b10000,0b01111],
        'D' => [0b11110,0b10001,0b10001,0b10001,0b10001,0b10001,0b11110],
        'E' | 'Е' | 'Ё' => [0b11111,0b10000,0b10000,0b11110,0b10000,0b10000,0b11111],
        'F' => [0b11111,0b10000,0b10000,0b11110,0b10000,0b10000,0b10000],
        'G' => [0b01111,0b10000,0b10000,0b10111,0b10001,0b10001,0b01111],
        'H' | 'Н' => [0b10001,0b10001,0b10001,0b11111,0b10001,0b10001,0b10001],
        'I' => [0b11111,0b00100,0b00100,0b00100,0b00100,0b00100,0b11111],
        'J' => [0b00111,0b00010,0b00010,0b00010,0b10010,0b10010,0b01100],
        'K' | 'К' => [0b10001,0b10010,0b10100,0b11000,0b10100,0b10010,0b10001],
        'L' => [0b10000,0b10000,0b10000,0b10000,0b10000,0b10000,0b11111],
        'M' | 'М' => [0b10001,0b11011,0b10101,0b10101,0b10001,0b10001,0b10001],
        'N' => [0b10001,0b11001,0b10101,0b10011,0b10001,0b10001,0b10001],
        'O' | 'О' => [0b01110,0b10001,0b10001,0b10001,0b10001,0b10001,0b01110],
        'P' | 'Р' => [0b11110,0b10001,0b10001,0b11110,0b10000,0b10000,0b10000],
        'Q' => [0b01110,0b10001,0b10001,0b10001,0b10101,0b10010,0b01101],
        'R' => [0b11110,0b10001,0b10001,0b11110,0b10100,0b10010,0b10001],
        'S' => [0b01111,0b10000,0b10000,0b01110,0b00001,0b00001,0b11110],
        'T' | 'Т' => [0b11111,0b00100,0b00100,0b00100,0b00100,0b00100,0b00100],
        'U' => [0b10001,0b10001,0b10001,0b10001,0b10001,0b10001,0b01110],
        'V' => [0b10001,0b10001,0b10001,0b10001,0b10001,0b01010,0b00100],
        'W' => [0b10001,0b10001,0b10001,0b10101,0b10101,0b10101,0b01010],
        'X' | 'Х' => [0b10001,0b10001,0b01010,0b00100,0b01010,0b10001,0b10001],
        'Y' => [0b10001,0b10001,0b01010,0b00100,0b00100,0b00100,0b00100],
        'Z' => [0b11111,0b00001,0b00010,0b00100,0b01000,0b10000,0b11111],
        'Б' => [0b11111,0b10000,0b10000,0b11110,0b10001,0b10001,0b11110],
        'Г' => [0b11111,0b10000,0b10000,0b10000,0b10000,0b10000,0b10000],
        'Д' => [0b00110,0b01010,0b01010,0b01010,0b01010,0b11111,0b10001],
        'Ж' => [0b10101,0b10101,0b01110,0b00100,0b01110,0b10101,0b10101],
        'З' => [0b11110,0b00001,0b00001,0b01110,0b00001,0b00001,0b11110],
        'И' => [0b10001,0b10011,0b10101,0b10101,0b10101,0b11001,0b10001],
        'Й' => [0b01010,0b00100,0b10001,0b10011,0b10101,0b11001,0b10001],
        'Л' => [0b00111,0b01001,0b01001,0b01001,0b01001,0b10001,0b10001],
        'П' => [0b11111,0b10001,0b10001,0b10001,0b10001,0b10001,0b10001],
        'У' => [0b10001,0b10001,0b10001,0b01111,0b00001,0b00001,0b11110],
        'Ф' => [0b00100,0b01110,0b10101,0b10101,0b01110,0b00100,0b00100],
        'Ц' => [0b10010,0b10010,0b10010,0b10010,0b10010,0b11111,0b00001],
        'Ч' => [0b10001,0b10001,0b10001,0b01111,0b00001,0b00001,0b00001],
        'Ш' => [0b10101,0b10101,0b10101,0b10101,0b10101,0b10101,0b11111],
        'Щ' => [0b10101,0b10101,0b10101,0b10101,0b10101,0b11111,0b00001],
        'Ъ' => [0b11000,0b01000,0b01000,0b01110,0b01001,0b01001,0b01110],
        'Ы' => [0b10001,0b10001,0b10001,0b11101,0b10011,0b10011,0b11101],
        'Ь' => [0b10000,0b10000,0b10000,0b11110,0b10001,0b10001,0b11110],
        'Э' => [0b11110,0b00001,0b00001,0b01111,0b00001,0b00001,0b11110],
        'Ю' => [0b10010,0b10101,0b10101,0b11101,0b10101,0b10101,0b10010],
        'Я' => [0b01111,0b10001,0b10001,0b01111,0b00101,0b01001,0b10001],
        '0' => [0b01110,0b10001,0b10011,0b10101,0b11001,0b10001,0b01110],
        '1' => [0b00100,0b01100,0b00100,0b00100,0b00100,0b00100,0b01110],
        '2' => [0b01110,0b10001,0b00001,0b00010,0b00100,0b01000,0b11111],
        '3' => [0b11110,0b00001,0b00001,0b01110,0b00001,0b00001,0b11110],
        '4' => [0b00010,0b00110,0b01010,0b10010,0b11111,0b00010,0b00010],
        '5' => [0b11111,0b10000,0b10000,0b11110,0b00001,0b00001,0b11110],
        '6' => [0b01110,0b10000,0b10000,0b11110,0b10001,0b10001,0b01110],
        '7' => [0b11111,0b00001,0b00010,0b00100,0b01000,0b01000,0b01000],
        '8' => [0b01110,0b10001,0b10001,0b01110,0b10001,0b10001,0b01110],
        '9' => [0b01110,0b10001,0b10001,0b01111,0b00001,0b00001,0b01110],
        '.' => [0,0,0,0,0,0b00110,0b00110],
        ':' => [0,0b00110,0b00110,0,0b00110,0b00110,0],
        '-' | '—' => [0,0,0,0b11111,0,0,0],
        '_' => [0,0,0,0,0,0,0b11111],
        '/' => [0b00001,0b00010,0b00010,0b00100,0b01000,0b01000,0b10000],
        ' ' => [0; 7],
        '…' => [0,0,0,0,0b10101,0,0],
        _ => [0b01110,0b10001,0b00010,0b00100,0b00100,0,0b00100],
    }
}
