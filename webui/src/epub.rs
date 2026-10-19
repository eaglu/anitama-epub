//! EPUB3 生成（对应 Python 版 build_epub）。每专题一本，图片内嵌，文末附评论。
use crate::cover::render_cover;
use crate::html::{self, Assets, Comment, RawItem};
use crate::util::{now_iso, snifftype, strip_tags, xml_escape};
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use zip::write::FileOptions;
use zip::{CompressionMethod, ZipWriter};

const XML: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n";

pub const STYLE: &str = r#"@charset "utf-8";

body {
  font-family: "Noto Serif CJK SC", "Source Han Serif SC", "Songti SC", "STSong",
               "SimSun", "Noto Serif", Georgia, serif;
  line-height: 1.8;
  margin: 0 1.1em;
  color: #1c1c1c;
  word-wrap: break-word;
}

/* ---------- 封面 ---------- */
.cover { margin: 0; padding: 0; text-align: center; }
.cover img { max-width: 100%; max-height: 100%; }

/* ---------- 书名页 ---------- */
.titlepage { text-align: center; margin-top: 26%; }
.titlepage h1 { font-size: 1.85em; line-height: 1.45; margin: 0 .2em .5em; }
.titlepage .tp-sub { color: #8a8a8a; font-size: .95em; letter-spacing: .35em; }
.titlepage .tp-line { width: 3.2em; height: 2px; border: none; background: #b08d57; margin: 1.8em auto; }
.titlepage .tp-meta { color: #9a9a9a; font-size: .85em; line-height: 2; margin-top: 3.5em; }

/* ---------- 文章标题 ---------- */
.article { margin: 0; }
.chapter-head { margin: 1.4em 0 1.3em; }
.art-index { font-size: .78em; color: #b08d57; letter-spacing: .18em; margin-bottom: .6em; }
h1.art-title { font-size: 1.55em; line-height: 1.45; margin: 0 0 .4em; font-weight: bold; }
h2.art-subtitle { font-size: 1.02em; font-weight: normal; color: #777; line-height: 1.6; margin: 0 0 .8em; }
p.art-meta { font-size: .82em; color: #999; margin: 0; padding-bottom: .8em; border-bottom: 1px solid #e6e6e6; }
p.art-meta .sep { margin: 0 .55em; color: #ccc; }

/* ---------- 正文 ---------- */
.content { font-size: 1em; }
.content p { margin: 0 0 .9em; text-indent: 2em; text-align: left; }
.content img { display: block; max-width: 100%; height: auto; margin: .9em auto; }
.content hr { border: none; border-top: 1px solid #e2e2e2; margin: 1.7em auto; width: 62%; }
.content blockquote { margin: 1.1em 1.1em; padding: .7em 1em; border-left: 3px solid #d9caa8;
  background: #faf8f3; color: #565656; }
.content blockquote p { text-indent: 0; }
.content h4, .content h5, .content h6 {
  font-size: 1.08em; line-height: 1.5; margin: 1.7em 0 .7em; padding-left: .55em; border-left: 4px solid #b08d57; }
.content h5 { font-size: 1.02em; } .content h6 { font-size: .98em; border-left-width: 3px; }
.content h4 a, .content h5 a, .content h6 a { color: inherit; text-decoration: none; }
.content ul, .content ol { margin: .6em 0 .9em; padding-left: 1.7em; }
.content li { margin: .3em 0; }
.content a { color: #2a6099; text-decoration: none; }
.content table { border-collapse: collapse; margin: 1em auto; max-width: 100%; font-size: .86em; }
.content th, .content td { border: 1px solid #dcdcdc; padding: .35em .6em; line-height: 1.5; }
.content th { background: #f4f1ea; }
.content center { text-align: center; }
.content .video-note { text-indent: 0; text-align: center; color: #a33; font-size: .85em; }

/* ---------- 评论 ---------- */
.comments { margin-top: 2.4em; padding-top: .9em; border-top: 2px solid #ececec; }
.cmt-head { font-size: 1.05em; color: #555; margin: 0 0 .9em; font-weight: bold; }
.cmt-head .cnt { font-size: .72em; color: #aaa; font-weight: normal; margin-left: .6em; }
.comment { padding: .6em 0; border-bottom: 1px dashed #eaeaea; }
.comment:last-child { border-bottom: none; }
.cmt-meta { font-size: .78em; color: #a49d92; margin: 0 0 .3em; }
.cmt-name { color: #7a6a4d; font-weight: bold; }
.cmt-time { margin-left: .7em; }
.cmt-body { font-size: .92em; color: #444; line-height: 1.7; }
.cmt-body p { margin: 0 0 .5em; text-indent: 0; }
.cmt-body p:last-child { margin-bottom: 0; }

/* ---------- 目录 ---------- */
.toc-title { font-size: 1.5em; margin-bottom: 1em; }
.toc-sub { color: #999; font-size: .85em; font-weight: normal; margin-left: .6em; }
nav#toc ol { line-height: 2.1; padding-left: 1.4em; }
nav#toc a { color: #222; text-decoration: none; }
"#;

const CONTAINER: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#;

fn page(title: &str, attr: &str, body: &str) -> String {
    let body = &html::selfclose_voids(body);
    format!(
        "{XML}<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" xml:lang=\"zh\" lang=\"zh\">\n<head><meta charset=\"utf-8\"/><title>{title}</title>\n<link rel=\"stylesheet\" type=\"text/css\" href=\"../style.css\"/></head>\n<body{attr}>{body}</body></html>"
    )
}

#[derive(Debug)]
pub enum Progress {
    Article {
        i: usize,
        total: usize,
        title: String,
    },
    Comments {
        n: usize,
    },
    Images(usize),
}

pub struct BuildOpts {
    pub embed_images: bool,
    pub comments: bool,
}

pub struct Built {
    pub pages: usize,
    pub images: usize,
    pub bytes: u64,
}

/// 生成一本 epub。items 已经按发表时间正序排好。
pub fn build_epub(
    site: &Path,
    root: &Path,
    sid: &str,
    title_html: &str,
    cover_url: Option<&str>,
    items: &[RawItem],
    out_path: &Path,
    opts: &BuildOpts,
    report: &mut dyn FnMut(Progress),
) -> Result<Built> {
    let mut assets: Assets = BTreeMap::new();
    let mut bodies: Vec<(String, String, String, String, String)> = Vec::new();
    let total = items.len();
    for (i, it) in items.iter().enumerate() {
        let path = site.join("article").join(format!("{}.html", it.aid));
        let raw = std::fs::read_to_string(&path).unwrap_or_default();
        let art = html::parse_article(&raw);
        let body = {
            let rewritten = if opts.embed_images {
                html::rewrite_content(&art.content, site, &mut assets)
            } else {
                // lean-ctx: 不内嵌图片时不收集素材，但仍做 script/style 清理与链接整理
                let mut scratch: Assets = BTreeMap::new();
                html::rewrite_content(&art.content, site, &mut scratch)
            }
            .unwrap_or_else(|_| art.content.clone());
            let wrapped = format!("<div class=\"content\">{rewritten}</div>");
            if opts.embed_images {
                wrapped
            } else {
                html::strip_media(&wrapped)
            }
        };
        report(Progress::Article {
            i: i + 1,
            total,
            title: if art.title.is_empty() {
                it.title.clone()
            } else {
                art.title.clone()
            },
        });
        bodies.push((
            art.title.clone(),
            art.subtitle.clone(),
            art.author.clone(),
            art.time_txt.clone(),
            body,
        ));
    }

    // 评论
    let mut comments_by_aid: BTreeMap<String, Vec<Comment>> = BTreeMap::new();
    if opts.comments {
        for it in items {
            let p = root.join("originals").join("comment").join(format!("{}.json", it.aid));
            if let Ok(txt) = std::fs::read_to_string(&p) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) {
                    if let Some(h) = v.get("data").and_then(|d| d.get("html")).and_then(|h| h.as_str()) {
                        if !h.trim().is_empty() {
                            let cs = html::parse_comments(h);
                            if !cs.is_empty() {
                                report(Progress::Comments { n: cs.len() });
                                comments_by_aid.insert(it.aid.clone(), cs);
                            }
                        }
                    }
                }
            }
        }
    }

    let title_text = strip_tags(title_html);
    let book_uuid = uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_URL,
        format!("anitama:series:{sid}").as_bytes(),
    );
    let uuid_urn = format!("urn:uuid:{book_uuid}");
    let modified = now_iso();

    // 封面图
    let cover_local: Option<PathBuf> = cover_url
        .and_then(|u| u.strip_prefix("/Anitama-Backup/"))
        .map(|r| site.join(r));
    let cover_bytes = render_cover(&title_text, items.len(), cover_local.as_deref()).ok();

    // 每篇文章一页
    let mut pages: Vec<(String, String, String)> = Vec::new(); // (pid, filename, nav title)
    let mut page_xhtml: Vec<String> = Vec::new();
    for (idx, (it, art)) in items.iter().zip(bodies.iter()).enumerate() {
        let i = idx + 1;
        let (a_title, a_sub, a_author, a_time, body) = art;
        let t = if a_title.is_empty() {
            it.title.clone()
        } else {
            a_title.clone()
        };
        let subtitle = if a_sub.is_empty() { it.subtitle.clone() } else { a_sub.clone() };
        let mut parts: Vec<String> = Vec::new();
        let au = if a_author.is_empty() { it.author.clone() } else { a_author.clone() };
        let tm = if a_time.is_empty() { it.time.clone() } else { a_time.clone() };
        if !au.is_empty() {
            parts.push(xml_escape(&au));
        }
        if !tm.is_empty() {
            parts.push(xml_escape(&tm));
        }
        let meta_line = parts.join("<span class=\"sep\">·</span>");
        let mut head = format!(
            "<header class=\"chapter-head\"><div class=\"art-index\">第 {i} 篇</div><h1 class=\"art-title\">{}</h1>",
            xml_escape(&t)
        );
        if !subtitle.is_empty() {
            head.push_str(&format!("<h2 class=\"art-subtitle\">{}</h2>", xml_escape(&subtitle)));
        }
        if !meta_line.is_empty() {
            head.push_str(&format!("<p class=\"art-meta\">{meta_line}</p>"));
        }
        head.push_str("</header>");
        let cmts = comments_by_aid
            .get(&it.aid)
            .map(|c| html::comments_xhtml(c))
            .unwrap_or_default();
        let body_html = format!("<div class=\"article\" epub:type=\"chapter\">{head}{body}{cmts}</div>");
        let fn_name = format!("text/{:04}.xhtml", i);
        let pid = format!("p{:04}", i);
        page_xhtml.push(page(&xml_escape(&t), "", &body_html));
        pages.push((pid, fn_name, t));
    }

    // manifest / spine
    let mut manifest = vec![
        "<item id=\"css\" href=\"style.css\" media-type=\"text/css\"/>".to_string(),
        "<item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>".to_string(),
        "<item id=\"ncx\" href=\"toc.ncx\" media-type=\"application/x-dtbncx+xml\"/>".to_string(),
    ];
    let mut spine: Vec<String> = Vec::new();
    let has_cover = cover_bytes.is_some();
    if has_cover {
        manifest.push("<item id=\"cover-image\" href=\"img/cover.jpg\" media-type=\"image/jpeg\"/>".into());
        manifest.push("<item id=\"cover\" href=\"text/cover.xhtml\" media-type=\"application/xhtml+xml\"/>".into());
        spine.push("<itemref idref=\"cover\"/>".into());
    }
    manifest.push("<item id=\"titlepage\" href=\"text/titlepage.xhtml\" media-type=\"application/xhtml+xml\"/>".into());
    spine.push("<itemref idref=\"titlepage\"/>".into());
    spine.push("<itemref idref=\"nav\" linear=\"no\"/>".into());
    for (pid, fn_name, _) in &pages {
        manifest.push(format!("<item id=\"{pid}\" href=\"{fn_name}\" media-type=\"application/xhtml+xml\"/>"));
        spine.push(format!("<itemref idref=\"{pid}\"/>"));
    }
    for (arc, local) in &assets {
        let (mime, _) = snifftype(local);
        let iid = format!("img_{}", regex::Regex::new(r"\W").unwrap().replace_all(arc, "_"));
        manifest.push(format!("<item id=\"{iid}\" href=\"{arc}\" media-type=\"{mime}\"/>"));
    }

    // nav + ncx
    let nav_lis: String = pages
        .iter()
        .map(|(_, fn_name, t)| format!("<li><a href=\"{fn_name}\">{}</a></li>", xml_escape(t)))
        .collect();
    let nav = format!(
        "{XML}<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" xml:lang=\"zh\">\n<head><meta charset=\"utf-8\"/><title>目录</title>\n<link rel=\"stylesheet\" type=\"text/css\" href=\"style.css\"/></head>\n<body><nav epub:type=\"toc\" id=\"toc\"><h1 class=\"toc-title\">{}<span class=\"toc-sub\">共 {} 篇</span></h1>\n<ol class=\"toc\">{nav_lis}</ol></nav></body></html>",
        xml_escape(&title_text),
        pages.len()
    );
    let nav_points: String = pages
        .iter()
        .enumerate()
        .map(|(i, (_, fn_name, t))| {
            format!("<navPoint id=\"nav{}\" playOrder=\"{}\"><navLabel><text>{}</text></navLabel><content src=\"{}\"/></navPoint>", i + 1, i + 1, xml_escape(t), fn_name)
        })
        .collect();
    let ncx = format!(
        "{XML}<ncx xmlns=\"http://www.daisy.org/z3986/2005/ncx/\" version=\"2005-1\">\n<head><meta name=\"dtb:uid\" content=\"{uuid_urn}\"/><meta name=\"dtb:depth\" content=\"1\"/></head>\n<docTitle><text>{}</text></docTitle>\n<navMap>{nav_points}</navMap></ncx>",
        xml_escape(&title_text)
    );
    let opf = format!(
        "{XML}<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"bookid\" xml:lang=\"zh\">\n<metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n<dc:identifier id=\"bookid\">{uuid_urn}</dc:identifier>\n<dc:title>{}</dc:title>\n<dc:language>zh</dc:language>\n<dc:creator>Anitama</dc:creator>\n<dc:publisher>Anitama</dc:publisher>\n<dc:source>Anitama-Backup</dc:source>\n<dc:date>{modified}</dc:date>\n{}<meta property=\"dcterms:modified\">{modified}</meta>\n</metadata>\n<manifest>{}</manifest>\n<spine toc=\"ncx\">{}</spine>\n</package>",
        xml_escape(&title_text),
        if has_cover { "<meta name=\"cover\" content=\"cover-image\"/>\n" } else { "" },
        manifest.join(""),
        spine.join("")
    );

    let title_page = page(
        &xml_escape(&title_text),
        " epub:type=\"titlepage\"",
        &format!(
            "<div class=\"titlepage\"><h1>{}</h1><div class=\"tp-sub\">Anitama 专题</div><hr class=\"tp-line\"/><p class=\"tp-meta\">共 {} 篇<br/>Anitama-Backup 归档</p></div>",
            if title_html.trim().is_empty() { xml_escape(&title_text) } else { title_html.to_string() },
            pages.len()
        ),
    );
    let cover_page = if has_cover {
        Some(page(
            "封面",
            " epub:type=\"cover\"",
            "<div class=\"cover\"><img src=\"../img/cover.jpg\" alt=\"封面\"/></div>",
        ))
    } else {
        None
    };

    // 写包
    if let Some(dir) = out_path.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let file = File::create(out_path).with_context(|| format!("无法写入 {}", out_path.display()))?;
    let mut zip = ZipWriter::new(BufWriter::new(file));
    let stored = FileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = FileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file("mimetype", stored)?;
    zip.write_all(b"application/epub+zip")?;
    let put = |zip: &mut ZipWriter<BufWriter<File>>, name: &str, data: &[u8]| -> Result<()> {
        zip.start_file(name, deflated)?;
        zip.write_all(data)?;
        Ok(())
    };
    put(&mut zip, "META-INF/container.xml", CONTAINER.as_bytes())?;
    put(&mut zip, "OEBPS/content.opf", opf.as_bytes())?;
    put(&mut zip, "OEBPS/nav.xhtml", nav.as_bytes())?;
    put(&mut zip, "OEBPS/toc.ncx", ncx.as_bytes())?;
    put(&mut zip, "OEBPS/style.css", STYLE.as_bytes())?;
    put(&mut zip, "OEBPS/text/titlepage.xhtml", title_page.as_bytes())?;
    if let (Some(cp), Some(cb)) = (cover_page.as_ref(), cover_bytes.as_ref()) {
        put(&mut zip, "OEBPS/text/cover.xhtml", cp.as_bytes())?;
        put(&mut zip, "OEBPS/img/cover.jpg", cb)?;
    }
    for (i, (_, fn_name, _)) in pages.iter().enumerate() {
        put(&mut zip, &format!("OEBPS/{fn_name}"), page_xhtml[i].as_bytes())?;
    }
    let mut nimg = 0usize;
    for (arc, local) in &assets {
        let bytes = std::fs::read(local).unwrap_or_default();
        put(&mut zip, &format!("OEBPS/{arc}"), &bytes)?;
        nimg += 1;
    }
    zip.finish()?;
    report(Progress::Images(nimg));
    let bytes = std::fs::metadata(out_path).map(|m| m.len()).unwrap_or(0);
    Ok(Built {
        pages: pages.len(),
        images: nimg,
        bytes,
    })
}
