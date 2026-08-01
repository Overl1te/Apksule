#![allow(clippy::cast_precision_loss)]

use apksule_apk::ApkPackage;
use thiserror::Error;
use tiny_skia::{Color, Paint, Pixmap, Rect, Transform};

use crate::dex::DexStatus;

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("surface dimensions are too large: {width}x{height}")]
    InvalidDimensions { width: u32, height: u32 },
}

pub fn render_launch_surface(
    package: &ApkPackage,
    status: &DexStatus,
    width: u32,
    height: u32,
) -> Result<Vec<u32>, RenderError> {
    let mut pixmap =
        Pixmap::new(width, height).ok_or(RenderError::InvalidDimensions { width, height })?;
    pixmap.fill(Color::from_rgba8(13, 18, 28, 255));

    let mut accent = Paint::default();
    accent.set_color_rgba8(72, 201, 176, 255);
    fill_rect(&mut pixmap, 0.0, 0.0, width as f32, 8.0, &accent);

    let margin = 28.0;
    let mut y = 34.0;
    draw_text(&mut pixmap, "APKSULE", margin, y, 4.0, (72, 201, 176, 255));
    y += 48.0;
    draw_text(&mut pixmap, "СРЕДА СОВМЕСТИМОСТИ M2", margin, y, 2.0, (150, 164, 184, 255));
    y += 42.0;

    let mut panel = Paint::default();
    panel.set_color_rgba8(24, 32, 48, 255);
    fill_rect(
        &mut pixmap,
        margin - 12.0,
        y - 16.0,
        (width as f32 - (margin * 2.0) + 24.0).max(1.0),
        (height as f32 - y - 40.0).max(1.0),
        &panel,
    );

    let lines = [
        format!("ПАКЕТ: {}", package.package_name),
        format!("АКТИВНОСТЬ: {}", package.main_activity.as_deref().unwrap_or("НЕ ОБЪЯВЛЕНА")),
        format!(
            "ВЕРСИЯ: {} ({})",
            package.version.name.as_deref().unwrap_or("НЕИЗВЕСТНО"),
            package.version.code.map_or_else(|| "НЕИЗВЕСТНО".to_owned(), |code| code.to_string())
        ),
        format!("DEX-ФАЙЛЫ: {}", package.resources.dex_entries.len()),
        format!("РАЗРЕШЕНИЯ: {}", package.permissions.len()),
        format!(
            "ТАБЛИЦА РЕСУРСОВ: {}",
            if package.resources.has_resource_table { "ЕСТЬ" } else { "НЕТ" }
        ),
    ];

    for line in lines {
        draw_text(&mut pixmap, &line, margin, y, 2.0, (226, 232, 240, 255));
        y += 24.0;
    }

    y += 14.0;
    let status_line = match status {
        DexStatus::NotLoaded => "СТАТУС: DEX НЕ ЗАГРУЖЕН".to_owned(),
        DexStatus::Ready { dex_files, classes } => {
            format!("СТАТУС: ГОТОВО - {dex_files} DEX, {classes} КЛАССОВ")
        }
        DexStatus::Running { activity, method } => {
            format!("СТАТУС: ВЫПОЛНЕНО {activity}.{method}")
        }
        DexStatus::Failed { reason } => format!("СТАТУС: ОШИБКА - {reason}"),
        DexStatus::Unsupported { dex_files, reason } => {
            format!("СТАТУС: ЗАГЛУШКА ({dex_files} DEX) - {reason}")
        }
    };
    draw_text(&mut pixmap, &status_line, margin, y, 2.0, (255, 190, 92, 255));

    if height > 50 {
        draw_text(
            &mut pixmap,
            "M2: ИСПОЛНЕНИЕ DEX. ИНТЕРФЕЙС APK БУДЕТ В M3",
            margin,
            height as f32 - 34.0,
            2.0,
            (150, 164, 184, 255),
        );
    }

    Ok(pixmap
        .data()
        .chunks_exact(4)
        .map(|pixel| (u32::from(pixel[0]) << 16) | (u32::from(pixel[1]) << 8) | u32::from(pixel[2]))
        .collect())
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
    for character in text.to_uppercase().chars() {
        if x + (5.0 * scale) >= pixmap.width() as f32 {
            break;
        }
        let glyph = glyph(character);
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
#[allow(clippy::match_same_arms)] // Latin/Cyrillic aliases intentionally share bitmap glyphs.
fn glyph(character: char) -> [u8; 7] {
    match character {
        'A' => [0b01110,0b10001,0b10001,0b11111,0b10001,0b10001,0b10001],
        'B' => [0b11110,0b10001,0b10001,0b11110,0b10001,0b10001,0b11110],
        'C' => [0b01111,0b10000,0b10000,0b10000,0b10000,0b10000,0b01111],
        'D' => [0b11110,0b10001,0b10001,0b10001,0b10001,0b10001,0b11110],
        'E' => [0b11111,0b10000,0b10000,0b11110,0b10000,0b10000,0b11111],
        'F' => [0b11111,0b10000,0b10000,0b11110,0b10000,0b10000,0b10000],
        'G' => [0b01111,0b10000,0b10000,0b10111,0b10001,0b10001,0b01111],
        'H' => [0b10001,0b10001,0b10001,0b11111,0b10001,0b10001,0b10001],
        'I' => [0b11111,0b00100,0b00100,0b00100,0b00100,0b00100,0b11111],
        'J' => [0b00111,0b00010,0b00010,0b00010,0b10010,0b10010,0b01100],
        'K' => [0b10001,0b10010,0b10100,0b11000,0b10100,0b10010,0b10001],
        'L' => [0b10000,0b10000,0b10000,0b10000,0b10000,0b10000,0b11111],
        'M' => [0b10001,0b11011,0b10101,0b10101,0b10001,0b10001,0b10001],
        'N' => [0b10001,0b11001,0b10101,0b10011,0b10001,0b10001,0b10001],
        'O' => [0b01110,0b10001,0b10001,0b10001,0b10001,0b10001,0b01110],
        'P' => [0b11110,0b10001,0b10001,0b11110,0b10000,0b10000,0b10000],
        'Q' => [0b01110,0b10001,0b10001,0b10001,0b10101,0b10010,0b01101],
        'R' => [0b11110,0b10001,0b10001,0b11110,0b10100,0b10010,0b10001],
        'S' => [0b01111,0b10000,0b10000,0b01110,0b00001,0b00001,0b11110],
        'T' => [0b11111,0b00100,0b00100,0b00100,0b00100,0b00100,0b00100],
        'U' => [0b10001,0b10001,0b10001,0b10001,0b10001,0b10001,0b01110],
        'V' => [0b10001,0b10001,0b10001,0b10001,0b10001,0b01010,0b00100],
        'W' => [0b10001,0b10001,0b10001,0b10101,0b10101,0b10101,0b01010],
        'X' => [0b10001,0b10001,0b01010,0b00100,0b01010,0b10001,0b10001],
        'Y' => [0b10001,0b10001,0b01010,0b00100,0b00100,0b00100,0b00100],
        'Z' => [0b11111,0b00001,0b00010,0b00100,0b01000,0b10000,0b11111],
        'А' => [0b01110,0b10001,0b10001,0b11111,0b10001,0b10001,0b10001],
        'Б' => [0b11111,0b10000,0b10000,0b11110,0b10001,0b10001,0b11110],
        'В' => [0b11110,0b10001,0b10001,0b11110,0b10001,0b10001,0b11110],
        'Г' => [0b11111,0b10000,0b10000,0b10000,0b10000,0b10000,0b10000],
        'Д' => [0b00110,0b01010,0b01010,0b01010,0b01010,0b11111,0b10001],
        'Е' | 'Ё' => [0b11111,0b10000,0b10000,0b11110,0b10000,0b10000,0b11111],
        'Ж' => [0b10101,0b10101,0b01110,0b00100,0b01110,0b10101,0b10101],
        'З' => [0b11110,0b00001,0b00001,0b01110,0b00001,0b00001,0b11110],
        'И' => [0b10001,0b10011,0b10101,0b10101,0b10101,0b11001,0b10001],
        'Й' => [0b01010,0b00100,0b10001,0b10011,0b10101,0b11001,0b10001],
        'К' => [0b10001,0b10010,0b10100,0b11000,0b10100,0b10010,0b10001],
        'Л' => [0b00111,0b01001,0b01001,0b01001,0b01001,0b10001,0b10001],
        'М' => [0b10001,0b11011,0b10101,0b10101,0b10001,0b10001,0b10001],
        'Н' => [0b10001,0b10001,0b10001,0b11111,0b10001,0b10001,0b10001],
        'О' => [0b01110,0b10001,0b10001,0b10001,0b10001,0b10001,0b01110],
        'П' => [0b11111,0b10001,0b10001,0b10001,0b10001,0b10001,0b10001],
        'Р' => [0b11110,0b10001,0b10001,0b11110,0b10000,0b10000,0b10000],
        'С' => [0b01111,0b10000,0b10000,0b10000,0b10000,0b10000,0b01111],
        'Т' => [0b11111,0b00100,0b00100,0b00100,0b00100,0b00100,0b00100],
        'У' => [0b10001,0b10001,0b10001,0b01111,0b00001,0b00001,0b11110],
        'Ф' => [0b00100,0b01110,0b10101,0b10101,0b01110,0b00100,0b00100],
        'Х' => [0b10001,0b10001,0b01010,0b00100,0b01010,0b10001,0b10001],
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
        '-' => [0,0,0,0b11111,0,0,0],
        '_' => [0,0,0,0,0,0,0b11111],
        '/' => [0b00001,0b00010,0b00010,0b00100,0b01000,0b01000,0b10000],
        '(' => [0b00010,0b00100,0b01000,0b01000,0b01000,0b00100,0b00010],
        ')' => [0b01000,0b00100,0b00010,0b00010,0b00010,0b00100,0b01000],
        ' ' => [0; 7],
        _ => [0b01110,0b10001,0b00010,0b00100,0b00100,0,0b00100],
    }
}

#[cfg(test)]
mod tests {
    use super::glyph;

    #[test]
    fn russian_diagnostic_copy_has_glyphs() {
        let unknown = glyph('?');
        let text = "СРЕДА СОВМЕСТИМОСТИ ПАКЕТ ВЕРСИЯ ФАЙЛЫ РАЗРЕШЕНИЯ \
                    ТАБЛИЦА РЕСУРСОВ ЕСТЬ НЕТ СТАТУС ЗАГРУЖЕН ГОТОВО \
                    КЛАССОВ ВЫПОЛНЕНО ОШИБКА ЗАГЛУШКА ИНТЕРФЕЙС БУДЕТ";

        for character in text.chars().filter(|character| !character.is_whitespace()) {
            assert_ne!(glyph(character), unknown, "нет глифа для {character}");
        }
    }
}
