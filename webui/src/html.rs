//! HTML 解析（scraper）与正文重写（lol_html）。
use crate::util::{collapse_ws, parse_cn_time, snifftype};
use anyhow::{Context, Result};
use lol_html::{element, rewrite_str, RewriteStrSettings};
use percent_encoding::percent_decode_str;
use scraper::{Html, Selector};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub type Assets = BTreeMap<String, PathBuf>;

#[derive(Debug, Clone)]
pub struct RawItem {
    pub aid: String,
    pub title: String,
    pub subtitle: String,
    pub channel: String,
    pub author: String,
    pub time: String,
}

pub struct ArticleDoc {
    pub title: String,
    pub subtitle: String,
    pub author: String,
    pub time_txt: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct Comment {
    pub n: u32,
    pub name: String,
    pub time: String,
    pub html: String,
}

fn sel(s: &str) -> Selector {
    Selector::parse(s).expect("selector")
}

fn text_of(el: &scraper::ElementRef) -> String {
    collapse_ws(&el.text().collect::<String>())
}

fn attr(el: &scraper::ElementRef, name: &str) -> String {
    el.value().attr(name).unwrap_or("").to_string()
}

/// 专题页 → (标题 HTML, 横幅封面路径, 文章条目)。
pub fn parse_series_page(html: &str) -> (Option<String>, Option<String>, Vec<RawItem>) {
    let doc = Html::parse_document(html);
    let mut title = None;
    if let Some(m) = doc.select(&sel("#mainer")).next() {
        let t = attr(&m, "data-title");
        if !t.trim().is_empty() {
            title = Some(t);
        }
    }
    let mut cover = None;
    if let Some(top) = doc.select(&sel("#area-top-series")).next() {
        let style = attr(&top, "style");
        if let Some(u) = extract_url(&style) {
            cover = Some(u);
        }
    }
    let item_sel = sel("a.item");
    let s_title = sel(".title");
    let s_sub = sel(".subtitle");
    let s_chan = sel(".channel");
    let s_auth = sel(".author");
    let s_time = sel(".time");
    let mut items = Vec::new();
    for a in doc.select(&item_sel) {
        let href = attr(&a, "href");
        let aid = match aid_from_href(&href) {
            Some(x) => x,
            None => continue,
        };
        let pick = |s: &Selector| a.select(s).next().map(|e| text_of(&e)).unwrap_or_default();
        items.push(RawItem {
            aid,
            title: pick(&s_title),
            subtitle: pick(&s_sub),
            channel: pick(&s_chan),
            author: pick(&s_auth),
            time: pick(&s_time),
        });
    }
    (title, cover, items)
}

/// 专题页是「新→旧」；能解析时间时按时间正序，否则整体倒序。
pub fn order_items(items: &mut Vec<RawItem>) {
    let times: Vec<Option<(i32, u32, u32, u32, u32)>> =
        items.iter().map(|i| parse_cn_time(&i.time)).collect();
    if times.iter().all(|t| t.is_some()) {
        items.sort_by_key(|i| parse_cn_time(&i.time).unwrap_or((0, 0, 0, 0, 0)));
    } else {
        items.reverse();
    }
}

fn aid_from_href(href: &str) -> Option<String> {
    let idx = href.find("/article/")? + "/article/".len();
    let rest = &href[idx..];
    let end = rest
        .find(|c: char| !c.is_ascii_alphanumeric())
        .unwrap_or(rest.len());
    let aid = &rest[..end];
    if aid.is_empty() {
        None
    } else {
        Some(aid.to_string())
    }
}

fn extract_url(style: &str) -> Option<String> {
    let i = style.find("url(")? + 4;
    let rest = &style[i..];
    let j = rest.find(')')?;
    let u = rest[..j].trim().trim_matches(|c| c == '\'' || c == '"').to_string();
    if u.is_empty() {
        None
    } else {
        Some(u)
    }
}

/// 文章页 → 标题 / 副标题 / 作者 / 时间 / 正文区 inner HTML。
pub fn parse_article(html: &str) -> ArticleDoc {
    let doc = Html::parse_document(html);
    let mainer = doc.select(&sel("#mainer")).next();
    let gt = |name: &str| -> String {
        mainer
            .as_ref()
            .map(|m| attr(m, name))
            .unwrap_or_default()
    };
    let mut time_txt = String::new();
    if let Some(info) = doc.select(&sel(".info-title-article")).next() {
        if let Some(t) = info.select(&sel(".time")).next() {
            time_txt = text_of(&t);
        }
    }
    let content = doc
        .select(&sel("#area-content-article"))
        .next()
        .map(|e| e.inner_html())
        .unwrap_or_else(|| "<p></p>".to_string());
    ArticleDoc {
        title: gt("data-title"),
        subtitle: gt("data-subtitle"),
        author: gt("data-author"),
        time_txt,
        content,
    }
}

/// 评论 JSON 里的 html 片段 → 评论列表。
pub fn parse_comments(fragment: &str) -> Vec<Comment> {
    let doc = Html::parse_fragment(fragment);
    let item_sel = sel("div.item-comment");
    let s_name = sel(".name");
    let s_time = sel(".time");
    let s_index = sel(".index");
    let s_body = sel(".content-comment");
    let mut out = Vec::new();
    for it in doc.select(&item_sel) {
        let body = match it.select(&s_body).next() {
            Some(b) => b,
            None => continue,
        };
        let get = |s: &Selector| it.select(s).next().map(|e| text_of(&e)).unwrap_or_default();
        let n: u32 = get(&s_index)
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse()
            .unwrap_or(0);
        let name = {
            let v = get(&s_name);
            if v.is_empty() {
                "匿名".to_string()
            } else {
                v
            }
        };
        let mut html = body.inner_html();
        // 评论里的图片（头像等）一律去掉
        html = strip_images(&html);
        out.push(Comment {
            n,
            name,
            time: get(&s_time),
            html,
        });
    }
    out.sort_by_key(|c| c.n);
    out
}

fn strip_images(html: &str) -> String {
    let out = rewrite_str(
        html,
        RewriteStrSettings {
            element_content_handlers: vec![element!("img", |el| {
                el.remove();
                Ok(())
            })],
            ..RewriteStrSettings::default()
        },
    );
    out.unwrap_or_else(|_| html.to_string())
}

/// 去掉正文里的图片与视频（“不内嵌图片”时用）。
/// 把 HTML5 的空元素写成 XHTML 自闭合形式（`<img …>` → `<img …/>`）。
/// lol_html 按 HTML5 序列化，产出的 void 标签在 EPUB 的 XHTML 里是非法的。
pub fn selfclose_voids(html: &str) -> String {
    let re = regex::Regex::new(
        r"<(img|br|hr|source|col|area|base|embed|param|track|wbr|input)((?:[^>]*?)?)\s*/?>",
    )
    .expect("void re");
    re.replace_all(html, "<$1$2/>").into_owned()
}

pub fn strip_media(html: &str) -> String {
    let out = rewrite_str(
        html,
        RewriteStrSettings {
            element_content_handlers: vec![element!("img, video, figure", |el| {
                el.remove();
                Ok(())
            })],
            ..RewriteStrSettings::default()
        },
    );
    out.unwrap_or_else(|_| html.to_string())
}

fn add_asset(assets: &mut Assets, site: &Path, local: &Path) -> String {
    let rel = local.strip_prefix(site).unwrap_or(local);
    let mut arc = format!("img/{}", rel.to_string_lossy().replace('/', "_"));
    let (_, ext) = snifftype(local);
    if !arc.to_lowercase().ends_with(ext) {
        arc.push_str(ext);
    }
    assets.insert(arc.clone(), local.to_path_buf());
    arc
}

fn local_from_site_url(site: &Path, url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("/Anitama-Backup/")?;
    Some(site.join(rest))
}

/// 正文重写：script/style 去掉；video→poster 图；img 换成内嵌图；链接清理。
/// 返回 XHTML 片段。
pub fn rewrite_content(inner: &str, site: &Path, assets: &mut Assets) -> Result<String> {
    let shared: Rc<RefCell<Assets>> = Rc::new(RefCell::new(std::mem::take(assets)));
    let site_owned = site.to_path_buf();
    let bare_domain = regex::Regex::new(r"(?i)^[\w.-]+\.[a-z]{2,}(/|$)").unwrap();

    let out = rewrite_str(
        inner,
        RewriteStrSettings {
            element_content_handlers: vec![
                element!("script, style", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("video", |el| {
                    let poster = el.get_attribute("poster").unwrap_or_default();
                    let mut repl = None;
                    if let Some(local) = local_from_site_url(&site_owned, &poster) {
                        if local.exists() {
                            let arc = {
                                let mut a = shared.borrow_mut();
                                add_asset(&mut a, &site_owned, &local)
                            };
                            repl = Some(format!(
                                r#"<img src="../{}" alt="视频"/>"#,
                                crate::util::attr_escape(&arc)
                            ));
                        }
                    }
                    match repl {
                        Some(h) => el.replace(&h, lol_html::html_content::ContentType::Html),
                        None => el.remove(),
                    }
                    Ok(())
                }),
                element!("img", |el| {
                    let src = el
                        .get_attribute("data-src")
                        .or_else(|| el.get_attribute("src"))
                        .unwrap_or_default();
                    el.remove_attribute("data-src");
                    el.remove_attribute("data-original");
                    el.remove_attribute("loading");
                    el.remove_attribute("onerror");
                    el.remove_attribute("onload");
                    if let Some(local) = local_from_site_url(&site_owned, &src) {
                        if local.exists() {
                            let arc = {
                                let mut a = shared.borrow_mut();
                                add_asset(&mut a, &site_owned, &local)
                            };
                            el.set_attribute("src", &format!("../{arc}"))?;
                        } else {
                            el.remove();
                        }
                    } else if src.starts_with("../") {
                        // video 替换进来的 poster，已是正确相对路径
                    } else if src.starts_with("http") {
                        // 少数外链图，保留原地址
                    } else {
                        el.remove();
                    }
                    Ok(())
                }),
                element!("a", |el| {
                    if let Some(href) = el.get_attribute("href") {
                        let low = href.trim().to_lowercase();
                        if low.starts_with("javascript") {
                            el.remove_attribute("href");
                        } else if bare_domain.is_match(href.trim()) {
                            el.set_attribute("href", &format!("http://{}", href.trim()))?;
                        }
                    }
                    el.remove_attribute("target");
                    el.remove_attribute("rel");
                    el.remove_attribute("onclick");
                    Ok(())
                }),
            ],
            ..RewriteStrSettings::default()
        },
    )
    .context("重写正文失败")?;

    *assets = shared.borrow().clone();
    Ok(out)
}

/// 归档内的资源地址（`/Anitama-Backup/<rest>`）→ 在线阅读用的 `/media/<rest>`。
/// 不存在、越界或含空段的地址返回 None（调用方据此删掉该元素）。
fn media_url(site: &Path, url: &str) -> Option<String> {
    let rest = url.trim().strip_prefix("/Anitama-Backup/")?;
    if rest.split('/').any(|s| s.is_empty() || s == "." || s == ".." || s.contains('\\')) {
        return None;
    }
    let decoded = percent_decode_str(rest).decode_utf8().ok()?;
    if !site.join(decoded.as_ref()).exists() {
        return None;
    }
    Some(format!("/media/{rest}"))
}

/// style 里的 `url(/Anitama-Backup/…)` → `/media/…`。
fn rewrite_style_urls(site: &Path, style: &str) -> String {
    let re = regex::Regex::new(r#"/Anitama-Backup/[^)"'\s]+"#).expect("style url re");
    re.replace_all(style, |c: &regex::Captures| {
        media_url(site, &c[0]).unwrap_or_else(|| c[0].to_string())
    })
    .into_owned()
}

/// 在线阅读用的正文重写：与 `rewrite_content` 同一套清理，但图片指向 `/media/<rest>`，不收集素材。
pub fn rewrite_web(inner: &str, site: &Path) -> Result<String> {
    let site_owned = site.to_path_buf();
    let bare_domain = regex::Regex::new(r"(?i)^[\w.-]+\.[a-z]{2,}(/|$)").unwrap();

    let out = rewrite_str(
        inner,
        RewriteStrSettings {
            element_content_handlers: vec![
                element!("script, style", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("video", |el| {
                    let poster = el.get_attribute("poster").unwrap_or_default();
                    match media_url(&site_owned, &poster) {
                        Some(u) => {
                            let h = format!(
                                r#"<img src="{}" alt="视频"/>"#,
                                crate::util::attr_escape(&u)
                            );
                            el.replace(&h, lol_html::html_content::ContentType::Html);
                        }
                        None => el.remove(),
                    }
                    Ok(())
                }),
                element!("img", |el| {
                    let src = el
                        .get_attribute("data-src")
                        .or_else(|| el.get_attribute("src"))
                        .unwrap_or_default();
                    for a in [
                        "data-src",
                        "data-original",
                        "loading",
                        "onerror",
                        "onload",
                        "srcset",
                    ] {
                        el.remove_attribute(a);
                    }
                    match media_url(&site_owned, &src) {
                        Some(u) => el.set_attribute("src", &u)?,
                        // 少数外链图保留原地址
                        None if src.trim().starts_with("http") => {
                            el.set_attribute("src", src.trim())?
                        }
                        None => el.remove(),
                    }
                    Ok(())
                }),
                element!("[style]", |el| {
                    if let Some(style) = el.get_attribute("style") {
                        if style.contains("/Anitama-Backup/") {
                            let new = rewrite_style_urls(&site_owned, &style);
                            el.set_attribute("style", &new)?;
                        }
                    }
                    Ok(())
                }),
                element!("a", |el| {
                    if let Some(href) = el.get_attribute("href") {
                        let low = href.trim().to_lowercase();
                        if low.starts_with("javascript") {
                            el.remove_attribute("href");
                        } else if bare_domain.is_match(href.trim()) {
                            el.set_attribute("href", &format!("http://{}", href.trim()))?;
                        }
                    }
                    el.remove_attribute("target");
                    el.remove_attribute("rel");
                    el.remove_attribute("onclick");
                    Ok(())
                }),
            ],
            ..RewriteStrSettings::default()
        },
    )
    .context("重写正文失败")?;

    Ok(out)
}

/// 评论 XHTML。与 Python 版输出一致。
pub fn comments_xhtml(items: &[Comment]) -> String {
    if items.is_empty() {
        return String::new();
    }
    let mut parts = vec![format!(
        r#"<div class="comments"><h2 class="cmt-head">评论<span class="cnt">{} 条</span></h2>"#,
        items.len()
    )];
    for c in items {
        parts.push(format!(
            r#"<div class="comment"><p class="cmt-meta"><span class="cmt-name">{}</span><span class="cmt-time">{}</span></p><div class="cmt-body"><div class="content-comment">{}</div></div></div>"#,
            crate::util::xml_escape(&c.name),
            crate::util::xml_escape(&c.time),
            c.html
        ));
    }
    parts.push("</div>".to_string());
    parts.join("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hrefs() {
        assert_eq!(
            aid_from_href("/Anitama-Backup/article/47fc3ef603b0dfff"),
            Some("47fc3ef603b0dfff".into())
        );
    }

    #[test]
    fn urls() {
        assert_eq!(
            extract_url("background-image: url(/Anitama-Backup/cdn/x/img/a-topic);").unwrap(),
            "/Anitama-Backup/cdn/x/img/a-topic"
        );
    }
}
