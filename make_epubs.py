#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
把 Anitama 归档里的「专题」(series) 打包成 EPUB。

一个专题一本：正文图文内嵌（cdn/<aid>/img/*-preview），文末附评论
(originals/comment/<aid>.json)。

用法:
  python3 make_epubs.py              # 全部专题
  python3 make_epubs.py 10 199       # 只做指定专题 id
  python3 make_epubs.py --list       # 列出专题
输出:
  ./epubs/<专题名>.epub
"""
import os
import re
import sys
import json
import uuid
import zipfile
from datetime import datetime

import lxml.html

ROOT = os.path.dirname(os.path.abspath(__file__))
SITE = os.path.join(ROOT, 'Anitama-Backup')
OUT = os.path.join(ROOT, 'epubs')

EPUB_NS = 'http://www.idpf.org/2007/opf'
XML = '<?xml version="1.0" encoding="utf-8"?>\n'


# ---------------------------------------------------------------- 工具

def snifftype(path):
    """按魔数判断图片类型（cdn 文件没有扩展名）。"""
    try:
        with open(path, 'rb') as f:
            b = f.read(16)
    except OSError:
        return 'image/jpeg', '.jpg'
    if b[:3] == b'\xff\xd8\xff':
        return 'image/jpeg', '.jpg'
    if b[:8] == b'\x89PNG\r\n\x1a\n':
        return 'image/png', '.png'
    if b[:6] in (b'GIF87a', b'GIF89a'):
        return 'image/gif', '.gif'
    if b[:4] == b'RIFF' and b[8:12] == b'WEBP':
        return 'image/webp', '.webp'
    if b[:2] == b'BM':
        return 'image/bmp', '.bmp'
    return 'image/jpeg', '.jpg'


def strip_tags(s):
    return re.sub(r'\s+', ' ', re.sub(r'<[^>]+>', '', s or '')).strip()


def safe_name(s, limit=80):
    s = strip_tags(s)
    s = re.sub(r'[\\/:*?"<>|\x00-\x1f]', '_', s)
    s = re.sub(r'\s+', ' ', s).strip().strip('.')
    return s[:limit] or 'untitled'


def parse_cn_time(s):
    m = re.search(r'(\d{4})年(\d{1,2})月(\d{1,2})日\s*(\d{1,2})时(\d{1,2})分', s or '')
    if m:
        return datetime(*[int(x) for x in m.groups()])
    m = re.search(r'(\d{4})-(\d{2})-(\d{2})', s or '')
    if m:
        return datetime(*[int(x) for x in m.groups()])
    return None


def xml_escape(s):
    return (s or '').replace('&', '&amp;').replace('<', '&lt;').replace('>', '&gt;')


COVER_FONTS = (
    '/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc',
    '/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf',
    '/usr/share/fonts/truetype/arphic/uming.ttc',
)


def _cover_font(size):
    from PIL import ImageFont
    for p in COVER_FONTS:
        if os.path.exists(p):
            try:
                return ImageFont.truetype(p, size)
            except OSError:
                continue
    return ImageFont.load_default()


def render_cover(title, n_pages, topic_local):
    """生成竖版封面图（横幅专题图 + 标题），返回 JPEG 字节。"""
    import io
    from PIL import Image, ImageDraw

    W, H = 1200, 1800
    im = Image.new('RGB', (W, H), (250, 248, 243))
    d = ImageDraw.Draw(im)
    maxw = W - 140

    def wrap(text, font):
        lines, cur = [], ''
        for ch in text:
            if d.textlength(cur + ch, font=font) > maxw and cur:
                lines.append(cur); cur = ch
            else:
                cur += ch
        if cur:
            lines.append(cur)
        return lines

    size = 78
    f = _cover_font(size)
    lines = wrap(title, f)
    while len(lines) > 3 and size > 44:          # 长标题自动缩号
        size -= 6
        f = _cover_font(size)
        lines = wrap(title, f)
    lines = lines[:4]

    img = None
    if topic_local and os.path.exists(topic_local):
        try:
            t = Image.open(topic_local).convert('RGB')
            tw = W - 160
            th = max(1, int(t.height * tw / t.width))
            if th > 700:
                th, tw = 700, int(t.width * 700 / t.height)
            img = t.resize((tw, th), Image.LANCZOS)
        except Exception:
            img = None

    lh = int(size * 1.32)
    tail_h = 34 + 6 + 50 + 58 * 2
    total = (img.height + 160 if img else 0) + lh * len(lines) + tail_h
    y = max(130, (H - total) // 2)

    if img:
        im.paste(img, ((W - img.width) // 2, y))
        y += img.height + 46
        d.line((70, y, W - 70, y), fill=(228, 222, 210), width=2)
        y += 114

    for ln in lines:
        d.text(((W - d.textlength(ln, font=f)) / 2, y), ln, font=f, fill=(30, 30, 30))
        y += lh

    y += 34
    d.line((W // 2 - 70, y, W // 2 + 70, y), fill=(176, 141, 87), width=5)
    y += 50
    fs = _cover_font(34)
    for ln in ('Anitama 专题', '共 %d 篇 · Anitama-Backup 归档' % n_pages):
        d.text(((W - d.textlength(ln, font=fs)) / 2, y), ln, font=fs, fill=(152, 152, 152))
        y += 58

    buf = io.BytesIO()
    im.save(buf, 'JPEG', quality=88)
    return buf.getvalue()


# ---------------------------------------------------------------- 解析专题

def iter_series_pages(sid_dir):
    """按页码顺序返回专题目录下的 html 文件。"""
    files = [f for f in os.listdir(sid_dir) if f.endswith('.html')]
    files.sort(key=lambda f: int(re.sub(r'\D', '', f) or 0))
    return [os.path.join(sid_dir, f) for f in files]


def collect_series():
    """返回 {sid: {title, cover, items:[{aid,title,subtitle,author,time}]}}"""
    series = {}
    sdir = os.path.join(SITE, 'series')
    for sid in sorted(os.listdir(sdir), key=lambda x: int(x) if x.isdigit() else 0):
        d = os.path.join(sdir, sid)
        if not os.path.isdir(d):
            continue
        title, cover, order, seen = None, None, [], set()
        for page in iter_series_pages(d):
            with open(page, 'rb') as f:
                doc = lxml.html.document_fromstring(f.read())
            mainer = doc.get_element_by_id('mainer')
            if mainer is not None and not title:
                title = mainer.get('data-title') or ''
            top = doc.get_element_by_id('area-top-series')
            if top is not None and not cover:
                m = re.search(r'url\(([^)]+)\)', top.get('style') or '')
                if m:
                    cover = m.group(1).strip('\'"')
            for a in doc.xpath('//a[contains(@class,"item")]'):
                href = a.get('href') or ''
                m = re.search(r'/article/([0-9a-f]+)', href)
                if not m or m.group(1) in seen:
                    continue
                aid = m.group(1)
                seen.add(aid)
                h2 = a.find_class('title')
                h4 = a.find_class('subtitle')
                au = a.find_class('author')
                tm = a.find_class('time')
                order.append({
                    'aid': aid,
                    'title': h2[0].text_content().strip() if h2 else '',
                    'subtitle': h4[0].text_content().strip() if h4 else '',
                    'author': au[0].text_content().strip() if au else '',
                    'time': tm[0].text_content().strip() if tm else '',
                })
        if not order:
            continue
        # 专题页是「新→旧」，正文按发表时间正序读更顺；时间缺失则整体倒序
        times = [parse_cn_time(it['time']) for it in order]
        if all(times):
            order.sort(key=lambda it: parse_cn_time(it['time']))
        else:
            order.reverse()
        series[sid] = {'title': title or ('专题 ' + sid), 'cover': cover, 'items': order}
    return series


# ---------------------------------------------------------------- 解析文章

class Assets:
    """收集一本 epub 用到的图片，负责去重与命名。"""

    def __init__(self):
        self.files = {}   # arcname -> local path

    def add(self, local):
        arc = 'img/' + os.path.relpath(local, SITE).replace(os.sep, '_')
        mime, ext = snifftype(local)
        if not arc.lower().endswith(ext):
            arc += ext
        self.files[arc] = local
        return arc, mime


def fix_content(area, assets):
    """把正文区整理成 XHTML 片段，顺便收集图片。"""
    area.set('class', 'content')           # 统一成样式钩子 .content
    area.attrib.pop('id', None)
    # 去掉脚本/样式
    for t in area.xpath('.//script|.//style'):
        t.getparent().remove(t)

    # <video> -> poster 图
    for v in list(area.iter('video')):
        poster = v.get('poster') or ''
        repl = None
        if poster.startswith('/Anitama-Backup/'):
            local = os.path.join(SITE, poster.split('/Anitama-Backup/', 1)[1])
            if os.path.exists(local):
                arc, _ = assets.add(local)
                repl = lxml.html.fragment_fromstring(
                    '<img src="../%s" alt="视频"/>' % arc, create_parent='span')
        if repl is not None:
            v.getparent().replace(v, repl[0] if len(repl) else repl)
        else:
            v.getparent().remove(v)

    # <img data-src> -> <img src>
    for img in area.iter('img'):
        src = img.get('data-src') or img.get('src') or ''
        img.attrib.pop('data-src', None)
        for k in ('data-original', 'loading', 'onerror', 'onload'):
            img.attrib.pop(k, None)
        if src.startswith('/Anitama-Backup/'):
            local = os.path.join(SITE, src.split('/Anitama-Backup/', 1)[1])
            if os.path.exists(local):
                arc, _ = assets.add(local)
                img.set('src', '../' + arc)
            else:
                img.getparent().remove(img)
        elif src.startswith('../'):
            pass                          # video 替换进来的 poster，已是正确相对路径
        elif src.startswith('http'):
            img.set('src', src)          # 少数外链图，保留原地址
        else:
            img.getparent().remove(img)

    # 去掉空链接 / javascript
    for a in area.iter('a'):
        href = a.get('href') or ''
        if href.strip().lower().startswith('javascript'):
            del a.attrib['href']
        elif re.match(r'^[\w.-]+\.[a-z]{2,}(/|$)', href, re.I):
            a.set('href', 'http://' + href)   # 原文里裸域名的外链补上协议
        for k in ('target', 'rel', 'onclick'):
            a.attrib.pop(k, None)

    return lxml.html.tostring(area, method='xml', encoding='unicode')


def load_article(aid, assets):
    path = os.path.join(SITE, 'article', aid + '.html')
    with open(path, 'rb') as f:
        doc = lxml.html.document_fromstring(f.read())
    mainer = doc.get_element_by_id('mainer')
    title = mainer.get('data-title', '') if mainer is not None else ''
    subtitle = mainer.get('data-subtitle', '') if mainer is not None else ''
    author = mainer.get('data-author', '') if mainer is not None else ''
    tm = doc.xpath('//*[contains(@class,"info-title-article")]//*[contains(@class,"time")]/text()')
    time_txt = tm[0].strip() if tm else ''
    area = doc.get_element_by_id('area-content-article')
    body = fix_content(area, assets) if area is not None else '<p></p>'
    return {'title': title, 'subtitle': subtitle, 'author': author,
            'time_txt': time_txt, 'time': parse_cn_time(time_txt), 'body': body}


def load_comments(aid):
    path = os.path.join(ROOT, 'originals', 'comment', aid + '.json')
    if not os.path.exists(path):
        return []
    try:
        with open(path, encoding='utf-8') as f:
            data = json.load(f)
        html = data.get('data', {}).get('html') or ''
    except (ValueError, OSError):
        return []
    if not html.strip():
        return []
    root = lxml.html.fragment_fromstring(html, create_parent='div')
    out = []
    for item in root.xpath('.//div[contains(@class,"item-comment")]'):
        name = item.xpath('.//*[contains(@class,"name")]/text()')
        time = item.xpath('.//*[contains(@class,"time")]/text()')
        idx = item.xpath('.//*[contains(@class,"index")]/text()')
        body = item.xpath('.//*[contains(@class,"content-comment")]')
        if not body:
            continue
        for img in body[0].iter('img'):          # 评论头像等一律去掉
            img.getparent().remove(img)
        n = int(re.sub(r'\D', '', idx[0]) or 0) if idx else 0
        out.append({
            'n': n,
            'name': (name[0].strip() if name else '匿名'),
            'time': (time[0].strip() if time else ''),
            'html': lxml.html.tostring(body[0], method='xml', encoding='unicode'),
        })
    out.sort(key=lambda c: c['n'])
    return out


def comments_xhtml(items):
    if not items:
        return ''
    parts = ['<div class="comments"><h2 class="cmt-head">评论%s</h2>'
             % ('<span class="cnt">%d 条</span>' % len(items))]
    for c in items:
        parts.append(
            '<div class="comment">'
            '<p class="cmt-meta"><span class="cmt-name">%s</span>'
            '<span class="cmt-time">%s</span></p>'
            '<div class="cmt-body">%s</div></div>'
            % (xml_escape(c['name']), xml_escape(c['time']), c['html']))
    parts.append('</div>')
    return ''.join(parts)


# ---------------------------------------------------------------- 生成 EPUB

PAGE = '''<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" xml:lang="zh" lang="zh">
<head><meta charset="utf-8"/><title>{title}</title>
<link rel="stylesheet" type="text/css" href="../style.css"/></head>
<body{attr}>{body}</body></html>'''

STYLE = '''@charset "utf-8";

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
'''

CONTAINER = XML + '''<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>'''


def build_epub(sid, meta, articles, comments_by_aid, path):
    assets = Assets()
    # 先处理正文以收集图片
    bodies = []
    for a in articles:
        art = load_article(a['aid'], assets)
        bodies.append((a, art))

    book_uuid = str(uuid.uuid5(uuid.NAMESPACE_URL, 'anitama:series:' + sid))
    modified = datetime.utcnow().strftime('%Y-%m-%dT%H:%M:%SZ')
    title_text = strip_tags(meta['title'])

    cover_local = None
    cover = meta.get('cover')
    if cover and cover.startswith('/Anitama-Backup/'):
        p = os.path.join(SITE, cover.split('/Anitama-Backup/', 1)[1])
        if os.path.exists(p):
            cover_local = p
    cover_arc = None
    cover_bytes = None

    # 每篇文章一个 xhtml
    pages = []           # (id, filename, nav_title, content)
    for i, (a, art) in enumerate(bodies, 1):
        t = art['title'] or a['title']
        subtitle = art['subtitle'] or a['subtitle']
        parts = [x for x in (art['author'] or a['author'], art['time_txt'] or a['time']) if x]
        meta_line = '<span class="sep">·</span>'.join(xml_escape(x) for x in parts)
        head = ('<header class="chapter-head"><div class="art-index">第 %d 篇</div>'
                '<h1 class="art-title">%s</h1>' % (i, xml_escape(t)))
        if subtitle:
            head += '<h2 class="art-subtitle">%s</h2>' % xml_escape(subtitle)
        if meta_line:
            head += '<p class="art-meta">%s</p>' % meta_line
        head += '</header>'
        body = ('<div class="article" epub:type="chapter">%s%s%s</div>'
                % (head, art['body'], comments_xhtml(comments_by_aid.get(a['aid'], []))))
        fn = 'text/%04d.xhtml' % i
        pages.append(('p%04d' % i, fn, t, PAGE.format(title=xml_escape(t), attr='', body=body)))

    # 封面图：竖版，横幅专题图 + 标题
    try:
        cover_bytes = render_cover(title_text, len(pages), cover_local)
        cover_arc = 'img/cover.jpg'
    except Exception as e:
        print('  cover skip: %s' % e, file=sys.stderr)

    # manifest
    manifest = [
        '<item id="css" href="style.css" media-type="text/css"/>',
        '<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>',
        '<item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>',
    ]
    spine = []
    if cover_arc:
        manifest.append('<item id="cover-image" href="%s" media-type="image/jpeg"/>' % cover_arc)
        manifest.append('<item id="cover" href="text/cover.xhtml" media-type="application/xhtml+xml"/>')
        spine.append('<itemref idref="cover"/>')
    manifest.append('<item id="titlepage" href="text/titlepage.xhtml" media-type="application/xhtml+xml"/>')
    spine.append('<itemref idref="titlepage"/>')
    spine.append('<itemref idref="nav" linear="no"/>')
    for pid, fn, t, _ in pages:
        manifest.append('<item id="%s" href="%s" media-type="application/xhtml+xml"/>' % (pid, fn))
        spine.append('<itemref idref="%s"/>' % pid)
    for arc, local in sorted(assets.files.items()):
        mime, _ = snifftype(local)
        iid = 'img_' + re.sub(r'\W', '_', arc)
        manifest.append('<item id="%s" href="%s" media-type="%s"/>' % (iid, arc, mime))

    # nav + ncx
    nav_lis = ''.join('<li><a href="%s">%s</a></li>' % (fn, xml_escape(t)) for _, fn, t, _ in pages)
    nav = XML + ('''<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" xml:lang="zh">
<head><meta charset="utf-8"/><title>目录</title>
<link rel="stylesheet" type="text/css" href="style.css"/></head>
<body><nav epub:type="toc" id="toc"><h1 class="toc-title">%s<span class="toc-sub">共 %d 篇</span></h1>
<ol class="toc">%s</ol></nav></body></html>''' % (xml_escape(title_text), len(pages), nav_lis))

    nav_points = []
    for i, (_, fn, t, _) in enumerate(pages, 1):
        nav_points.append('''<navPoint id="nav%d" playOrder="%d"><navLabel><text>%s</text></navLabel><content src="%s"/></navPoint>'''
                           % (i, i, xml_escape(t), fn))
    ncx = XML + ('''<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
<head><meta name="dtb:uid" content="urn:uuid:%s"/><meta name="dtb:depth" content="1"/></head>
<docTitle><text>%s</text></docTitle>
<navMap>%s</navMap></ncx>''' % (book_uuid, xml_escape(title_text), ''.join(nav_points)))

    opf = XML + ('''<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="bookid" xml:lang="zh">
<metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
<dc:identifier id="bookid">urn:uuid:%s</dc:identifier>
<dc:title>%s</dc:title>
<dc:language>zh</dc:language>
<dc:creator>Anitama</dc:creator>
<dc:publisher>Anitama</dc:publisher>
<dc:source>Anitama-Backup</dc:source>
<dc:date>%s</dc:date>
%s<meta property="dcterms:modified">%s</meta>
</metadata>
<manifest>%s</manifest>
<spine toc="ncx">%s</spine>
</package>''' % (book_uuid, xml_escape(title_text), modified,
                 '<meta name="cover" content="cover-image"/>\n' if cover_arc else '',
                 modified, ''.join(manifest), ''.join(spine)))

    cover_page = None
    if cover_arc:
        cover_page = PAGE.format(title='封面', attr=' epub:type="cover"',
                                 body='<div class="cover"><img src="../%s" alt="封面"/></div>' % cover_arc)

    title_page = PAGE.format(
        title=xml_escape(title_text), attr=' epub:type="titlepage"',
        body=('<div class="titlepage"><h1>%s</h1>'
              '<div class="tp-sub">Anitama 专题</div>'
              '<hr class="tp-line"/>'
              '<p class="tp-meta">共 %d 篇<br/>Anitama-Backup 归档</p></div>'
              % (meta['title'] or xml_escape(title_text), len(pages))))

    # 写包
    with zipfile.ZipFile(path, 'w', zipfile.ZIP_DEFLATED) as z:
        zi = zipfile.ZipInfo('mimetype')
        z.writestr(zi, 'application/epub+zip', compress_type=zipfile.ZIP_STORED)
        z.writestr('META-INF/container.xml', CONTAINER)
        z.writestr('OEBPS/content.opf', opf)
        z.writestr('OEBPS/nav.xhtml', nav)
        z.writestr('OEBPS/toc.ncx', ncx)
        z.writestr('OEBPS/style.css', STYLE)
        z.writestr('OEBPS/text/titlepage.xhtml', title_page)
        if cover_page:
            z.writestr('OEBPS/text/cover.xhtml', cover_page)
            z.writestr('OEBPS/' + cover_arc, cover_bytes)
        for _, fn, _, content in pages:
            z.writestr('OEBPS/' + fn, content)
        for arc, local in assets.files.items():
            z.writestr('OEBPS/' + arc, open(local, 'rb').read())
    return len(pages), len(assets.files)


def main():
    args = [a for a in sys.argv[1:]]
    series = collect_series()
    if '--list' in args:
        for sid, s in series.items():
            print('%s\t%s\t%d 篇' % (sid, strip_tags(s['title']), len(s['items'])))
        return
    want = [a for a in args if a.isdigit()]
    os.makedirs(OUT, exist_ok=True)
    used = set()
    todo = [(sid, s) for sid, s in series.items() if not want or sid in want]
    for sid, s in todo:
        name = safe_name(s['title'])
        base, k = name, 2
        while base in used:
            base = '%s (%d)' % (name, k); k += 1
        used.add(base)
        cba = {it['aid']: load_comments(it['aid']) for it in s['items']}
        path = os.path.join(OUT, base + '.epub')
        npage, nimg = build_epub(sid, s, s['items'], cba, path)
        size = os.path.getsize(path) / 1024 / 1024
        print('%-4s %-40s %2d 篇 %3d 图  %5.1f MB' % (sid, base[:40], npage, nimg, size))


if __name__ == '__main__':
    main()
