//! 小工具：转义、文件名清洗、中文时间解析、图片类型嗅探。
use std::path::Path;

/// 与 Python 版 xml_escape 等价（& < > 转义）。
pub fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

/// 属性值转义（含引号）。
pub fn attr_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// 去掉 HTML 标签并把连续空白压成一个空格。
pub fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0usize;
    for c in s.chars() {
        match c {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    collapse_ws(&out)
}

pub fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut sp = false;
    for c in s.trim().chars() {
        if c.is_whitespace() {
            sp = true;
        } else {
            if sp && !out.is_empty() {
                out.push(' ');
            }
            sp = false;
            out.push(c);
        }
    }
    out
}

/// 输出文件名清洗：去标签、非法字符换下划线、限长。与 Python 版保持一致。
pub fn safe_name(s: &str, limit: usize) -> String {
    let t = strip_tags(s);
    let mut out = String::with_capacity(t.len());
    for c in t.chars() {
        let bad = matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
            || (c as u32) < 0x20;
        out.push(if bad { '_' } else { c });
    }
    let out = collapse_ws(&out);
    let out = out.trim_matches('.').trim().to_string();
    let out: String = out.chars().take(limit).collect();
    if out.is_empty() {
        "untitled".to_string()
    } else {
        out
    }
}

/// 解析「2016年3月6日 8时20分」或「2016-03-06」。返回可排序的 (年,月,日,时,分)。
pub fn parse_cn_time(s: &str) -> Option<(i32, u32, u32, u32, u32)> {
    let cs: Vec<char> = s.chars().collect();
    // 中文格式
    if let Some((y, m, d, hh, mm)) = cn_parts(&cs) {
        return Some((y, m, d, hh, mm));
    }
    // 2016-03-06
    let digits: Vec<&str> = s.split(|c: char| !c.is_ascii_digit()).filter(|x| !x.is_empty()).collect();
    if digits.len() >= 3 {
        let y = digits[0].parse::<i32>().ok()?;
        if y < 1900 || y > 2200 {
            return None;
        }
        let m = digits[1].parse::<u32>().ok()?;
        let d = digits[2].parse::<u32>().ok()?;
        return Some((y, m, d, 0, 0));
    }
    None
}

fn cn_parts(cs: &[char]) -> Option<(i32, u32, u32, u32, u32)> {
    let s: String = cs.iter().collect();
    let pos_nian = s.find('年')?;
    let y: i32 = s.get(..pos_nian)?.trim().parse().ok()?;
    let rest = s.get(pos_nian + 3..)?;
    let pos_yue = rest.find('月')?;
    let m: u32 = rest.get(..pos_yue)?.trim().parse().ok()?;
    let rest = rest.get(pos_yue + 3..)?;
    let pos_ri = rest.find('日')?;
    let d: u32 = rest.get(..pos_ri)?.trim().parse().ok()?;
    let rest = rest.get(pos_ri + 3..)?;
    let mut hh = 0u32;
    let mut mm = 0u32;
    if let Some(pos_shi) = rest.find('时') {
        hh = rest.get(..pos_shi)?.trim().parse().unwrap_or(0);
        let r2 = rest.get(pos_shi + 3..)?;
        if let Some(pos_fen) = r2.find('分') {
            mm = r2.get(..pos_fen)?.trim().parse().unwrap_or(0);
        }
    }
    Some((y, m, d, hh, mm))
}

/// 把 (年,月,日) 显示成 2015–2016 这种区间。
pub fn year_range(items: &[String]) -> (String, String) {
    let mut ys: Vec<i32> = items
        .iter()
        .filter_map(|t| parse_cn_time(t).map(|p| p.0))
        .collect();
    ys.sort_unstable();
    if ys.is_empty() {
        (String::new(), String::new())
    } else {
        (ys[0].to_string(), ys[ys.len() - 1].to_string())
    }
}

/// 按魔数判断图片类型（cdn 文件没有扩展名）。
pub fn snifftype(path: &Path) -> (&'static str, &'static str) {
    use std::io::Read;
    let mut buf = [0u8; 16];
    let n = std::fs::File::open(path)
        .and_then(|mut f| f.read(&mut buf))
        .unwrap_or(0);
    let b = &buf[..n];
    if b.starts_with(&[0xff, 0xd8, 0xff]) {
        ("image/jpeg", ".jpg")
    } else if b.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        ("image/png", ".png")
    } else if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        ("image/gif", ".gif")
    } else if b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        ("image/webp", ".webp")
    } else if b.starts_with(b"BM") {
        ("image/bmp", ".bmp")
    } else {
        ("image/jpeg", ".jpg")
    }
}

/// ISO 风格 UTC 时间戳，形如 2026-10-18T09:31:00Z。
pub fn now_iso() -> String {
    let now = time::OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        now.year(),
        now.month() as u8,
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times() {
        assert_eq!(parse_cn_time("2016年3月6日 8时20分"), Some((2016, 3, 6, 8, 20)));
        assert_eq!(parse_cn_time("2016年3月6日"), Some((2016, 3, 6, 0, 0)));
        assert_eq!(parse_cn_time("2019-02-15"), Some((2019, 2, 15, 0, 0)));
        assert_eq!(parse_cn_time("无"), None);
    }

    #[test]
    fn names() {
        assert_eq!(safe_name("<span style='color:Gold'>a/b:c</span>", 80), "a_b_c");
    }
}
