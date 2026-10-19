//! 竖版封面渲染（1200×1800）：横幅专题图 + 自动换行标题 + 金线 + 落款。
//! 对应 Python 版 render_cover()。
use anyhow::{Context, Result};
use image::codecs::jpeg::JpegEncoder;
use image::{imageops, Rgb, RgbImage};
use imageproc::drawing::{draw_filled_rect_mut, draw_text_mut, text_size};
use imageproc::rect::Rect;
use rusttype::{Font, Scale};
use std::io::Cursor;
use std::path::Path;

const FONTS: [&str; 3] = [
    "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
    "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
    "/usr/share/fonts/truetype/arphic/uming.ttc",
];

const PAPER: Rgb<u8> = Rgb([250, 248, 243]);
const INK: Rgb<u8> = Rgb([30, 30, 30]);
const GOLD: Rgb<u8> = Rgb([176, 141, 87]);
const RULE: Rgb<u8> = Rgb([228, 222, 210]);
const GREY: Rgb<u8> = Rgb([152, 152, 152]);

struct Face {
    font: Font<'static>,
    scale: Scale,
}

fn face(data: &[u8], size: f32) -> Result<Face> {
    let font =
        Font::try_from_vec_and_index(data.to_vec(), 0).context("加载封面字体失败")?;
    Ok(Face {
        font,
        scale: Scale::uniform(size),
    })
}

fn width(f: &Face, text: &str) -> f32 {
    text_size(f.scale, &f.font, text).0 as f32
}

fn wrap(f: &Face, maxw: f32, text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        let mut probe = cur.clone();
        probe.push(ch);
        if width(f, &probe) > maxw && !cur.is_empty() {
            lines.push(std::mem::take(&mut cur));
            cur.push(ch);
        } else {
            cur = probe;
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

fn centered(im: &mut RgbImage, f: &Face, text: &str, y: i32, color: Rgb<u8>) {
    let x = ((im.width() as f32 - width(f, text)) / 2.0).max(0.0) as i32;
    draw_text_mut(im, color, x, y, f.scale, &f.font, text);
}

fn rule(im: &mut RgbImage, x0: i32, y: i32, w: u32, h: u32, color: Rgb<u8>) {
    draw_filled_rect_mut(im, Rect::at(x0, y).of_size(w, h), color);
}

/// 生成封面 JPEG 字节；没字体可用时返回 Err（调用方跳过封面）。
pub fn render_cover(title: &str, n_pages: usize, topic: Option<&Path>) -> Result<Vec<u8>> {
    let data = FONTS
        .iter()
        .find(|p| Path::new(p).exists())
        .map(|p| std::fs::read(p))
        .transpose()?
        .context("找不到封面字体")?;

    let w = 1200u32;
    let h = 1800u32;
    let mut im = RgbImage::from_pixel(w, h, PAPER);
    let maxw = (w - 140) as f32;

    // rusttype 的 Scale 与 PIL 的 truetype(size) 口径不同：同一标称字号下 rusttype 的
    // em 只有 PIL 的 0.78 倍（实测 78→墨迹 60px、100→78px）。乘系数让封面与已交付的
    // PIL 版视觉一致；换行度量与行高仍按 PIL 的标称字号计算。
    let k = 100.0f32 / 78.0;
    let mut size = 78f32;
    let mut f = face(&data, size * k)?;
    let mut lines = wrap(&f, maxw, title);
    while lines.len() > 3 && size > 44.0 {
        size -= 6.0;
        f = face(&data, size * k)?;
        lines = wrap(&f, maxw, title);
    }
    lines.truncate(4);
    let lh = (size * 1.32) as i32;

    // 横幅专题图（宽 1040，最高 700）
    let banner: Option<RgbImage> = topic
        .filter(|p| p.exists())
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|d| image::load_from_memory(&d).ok())
        .map(|img| {
            let rgb = img.to_rgb8();
            let tw = w - 160;
            let th = (img.height() as f64 * tw as f64 / img.width() as f64).round() as u32;
            if th > 700 {
                imageops::resize(
                    &rgb,
                    (rgb.width() as f64 * 700.0 / rgb.height() as f64).round() as u32,
                    700,
                    imageops::FilterType::Lanczos3,
                )
            } else {
                imageops::resize(&rgb, tw, th.max(1), imageops::FilterType::Lanczos3)
            }
        });

    let tail_h = 34 + 6 + 50 + 58 * 2;
    let total = banner
        .as_ref()
        .map(|b| b.height() as i32 + 160)
        .unwrap_or(0)
        + lh * lines.len() as i32
        + tail_h;
    let mut y = ((h as i32 - total) / 2).max(130);

    if let Some(b) = banner.as_ref() {
        let bx = (w as i32 - b.width() as i32) / 2;
        imageops::overlay(&mut im, b, bx as i64, y as i64);        y += b.height() as i32 + 46;
        rule(&mut im, 70, y, w - 140, 2, RULE);
        y += 114;
    }

    for ln in &lines {
        centered(&mut im, &f, ln, y, INK);
        y += lh;
    }

    y += 34;
    rule(&mut im, w as i32 / 2 - 70, y, 140, 5, GOLD);
    y += 50;
    let fs = face(&data, 34.0 * k)?;
    for ln in [
        "Anitama 专题".to_string(),
        format!("共 {n_pages} 篇 · Anitama-Backup 归档"),
    ] {
        centered(&mut im, &fs, &ln, y, GREY);
        y += 58;
    }

    let mut buf = Cursor::new(Vec::new());
    JpegEncoder::new_with_quality(&mut buf, 88)
        .encode_image(&im)
        .context("封面编码失败")?;
    Ok(buf.into_inner())
}
