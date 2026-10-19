//! 归档扫描：专题目录 → 内存索引；封面缩略图按需生成并缓存。
use crate::html;
use crate::util::{strip_tags, year_range};
use anyhow::Result;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Serialize, Clone, Debug)]
pub struct Item {
    pub aid: String,
    pub title: String,
    pub subtitle: String,
    pub channel: String,
    pub author: String,
    pub time: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct Series {
    pub sid: String,
    pub title: String,
    pub title_text: String,
    pub n: usize,
    pub cover: Option<String>,
    pub authors: Vec<String>,
    pub channel: String,
    pub y0: String,
    pub y1: String,
    pub items: Vec<Item>,
}

impl Series {
    /// 转成生成器需要的时间正序条目。
    pub fn raw_items(&self) -> Vec<html::RawItem> {
        self.items
            .iter()
            .map(|i| html::RawItem {
                aid: i.aid.clone(),
                title: i.title.clone(),
                subtitle: i.subtitle.clone(),
                channel: i.channel.clone(),
                author: i.author.clone(),
                time: i.time.clone(),
            })
            .collect()
    }
}

#[derive(Serialize, Clone)]
pub struct EpubFile {
    pub name: String,
    pub size: u64,
    pub mtime: u64,
    pub sid: Option<String>,
}

pub struct Archive {
    pub root: PathBuf,
    pub site: PathBuf,
    pub out: PathBuf,
    pub series: Vec<Series>,
    pub assets: usize,
    thumbs: Mutex<HashMap<String, Arc<Vec<u8>>>>,
}

fn numeric(name: &str) -> Option<i64> {
    let d: String = name.chars().filter(|c| c.is_ascii_digit()).collect();
    if d.is_empty() || d.len() != name.len() {
        None
    } else {
        d.parse().ok()
    }
}

impl Archive {
    pub fn load(root: &Path) -> Result<Arc<Archive>> {
        let site = root.join("Anitama-Backup");
        let out = root.join("epubs");
        let series = scan_series(&site)?;
        let assets = count_assets(&site);
        Ok(Arc::new(Archive {
            root: root.to_path_buf(),
            site,
            out,
            series,
            assets,
            thumbs: Mutex::new(HashMap::new()),
        }))
    }

    pub fn by_sid(&self, sid: &str) -> Option<&Series> {
        self.series.iter().find(|s| s.sid == sid)
    }

    pub fn cover_path(&self, sid: &str) -> Option<PathBuf> {
        let s = self.by_sid(sid)?;
        let url = s.cover.as_ref()?;
        let rest = url.strip_prefix("/Anitama-Backup/")?;
        let p = self.site.join(rest);
        if p.exists() {
            Some(p)
        } else {
            None
        }
    }

    /// 横幅缩略图（JPEG）。按 sid+宽度缓存。
    pub fn thumb(&self, sid: &str, w: u32) -> Option<Arc<Vec<u8>>> {
        let key = format!("{sid}:{w}");
        if let Some(v) = self.thumbs.lock().ok()?.get(&key) {
            return Some(v.clone());
        }
        let path = self.cover_path(sid)?;
        let data = std::fs::read(&path).ok()?;
        let img = image::load_from_memory(&data).ok()?;
        let scale = w as f64 / img.width() as f64;
        let (tw, th) = if scale < 1.0 {
            (w.max(1), ((img.height() as f64 * scale).round() as u32).max(1))
        } else {
            (img.width(), img.height())
        };
        let rgb = img.to_rgb8();
        let resized = image::imageops::resize(&rgb, tw, th, image::imageops::FilterType::Lanczos3);
        let mut buf = std::io::Cursor::new(Vec::new());
        let ok = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 80).encode_image(&resized);
        if ok.is_err() {
            return None;
        }
        let v = Arc::new(buf.into_inner());
        self.thumbs.lock().ok()?.insert(key, v.clone());
        Some(v)
    }

    pub fn epubs(&self) -> Vec<EpubFile> {
        let mut out = Vec::new();
        let rd = match std::fs::read_dir(&self.out) {
            Ok(r) => r,
            Err(_) => return out,
        };
        let mut names: HashMap<String, String> = HashMap::new();
        for s in &self.series {
            names.insert(crate::util::safe_name(&s.title, 80), s.sid.clone());
        }
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("epub") {
                continue;
            }
            let stem = p.file_stem().and_then(|x| x.to_str()).unwrap_or("").to_string();
            let md = e.metadata().ok();
            let size = md.as_ref().map(|m| m.len()).unwrap_or(0);
            let mtime = md
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            out.push(EpubFile {
                sid: names.get(&stem).cloned(),
                name: p.file_name().and_then(|x| x.to_str()).unwrap_or("").to_string(),
                size,
                mtime,
            });
        }
        out.sort_by(|a, b| b.mtime.cmp(&a.mtime));
        out
    }
}

fn scan_series(site: &Path) -> Result<Vec<Series>> {
    let dir = site.join("series");
    let mut sids: Vec<String> = std::fs::read_dir(&dir)?
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| numeric(n).is_some())
        .collect();
    sids.sort_by_key(|n| numeric(n).unwrap_or(0));
    let mut out = Vec::new();
    for sid in sids {
        let d = dir.join(&sid);
        let mut pages: Vec<PathBuf> = std::fs::read_dir(&d)?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("html"))
            .collect();
        pages.sort_by_key(|p| {
            p.file_name()
                .and_then(|x| x.to_str())
                .and_then(|n| numeric(&n.replace(".html", "")))
                .unwrap_or(0)
        });
        let mut title = String::new();
        let mut cover: Option<String> = None;
        let mut seen = std::collections::HashSet::new();
        let mut raw = Vec::new();
        for p in pages {
            let html_src = match std::fs::read_to_string(&p) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let (t, c, items) = html::parse_series_page(&html_src);
            if title.is_empty() {
                if let Some(t) = t {
                    title = t;
                }
            }
            if cover.is_none() {
                cover = c;
            }
            for it in items {
                if seen.insert(it.aid.clone()) {
                    raw.push(it);
                }
            }
        }
        if raw.is_empty() {
            continue;
        }
        html::order_items(&mut raw);
        let mut authors: Vec<String> = Vec::new();
        let mut chan_count: HashMap<String, usize> = HashMap::new();
        for it in &raw {
            let a = it.author.trim();
            if !a.is_empty() && !authors.iter().any(|x| x == a) && authors.len() < 4 {
                authors.push(a.to_string());
            }
            let c = it.channel.trim();
            if !c.is_empty() {
                *chan_count.entry(c.to_string()).or_insert(0) += 1;
            }
        }
        let channel = chan_count
            .into_iter()
            .max_by_key(|(_, v)| *v)
            .map(|(k, _)| k)
            .unwrap_or_default();
        let (y0, y1) = year_range(&raw.iter().map(|i| i.time.clone()).collect::<Vec<_>>());
        let items: Vec<Item> = raw
            .iter()
            .map(|i| Item {
                aid: i.aid.clone(),
                title: i.title.clone(),
                subtitle: i.subtitle.clone(),
                channel: i.channel.clone(),
                author: i.author.clone(),
                time: i.time.clone(),
            })
            .collect();
        let title_text = strip_tags(&title);
        out.push(Series {
            sid,
            title,
            title_text,
            n: items.len(),
            cover,
            authors,
            channel,
            y0,
            y1,
            items,
        });
    }
    Ok(out)
}

/// cdn 下的素材文件总数（cdn/<aid>/{img,video}/*）。
fn count_assets(site: &Path) -> usize {
    let mut n = 0usize;
    let rd = match std::fs::read_dir(site.join("cdn")) {
        Ok(r) => r,
        Err(_) => return 0,
    };
    for e in rd.flatten() {
        for kind in ["img", "video"] {
            let kd = e.path().join(kind);
            if let Ok(rd2) = std::fs::read_dir(kd) {
                n += rd2.filter_map(|x| x.ok()).filter(|x| x.path().is_file()).count();
            }
        }
    }
    n
}
