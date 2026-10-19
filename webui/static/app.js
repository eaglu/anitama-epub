// Anitama 工坊 · 专题 EPUB 装订台（前端）
// 数据全部来自本地 axum 服务：/api/state、/api/series/:sid、/api/cover/:sid、/api/jobs、/api/epubs
'use strict';

const $ = (s) => document.querySelector(s);
const esc = (s) => String(s == null ? '' : s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));

// 预估口径：取自已生成的 4 本真实样本（约 0.7 MB/篇、22 图/篇、1.3 秒/篇）
const PER_MB = 0.7, PER_IMG = 22, PER_SEC = 1.3, PER_MB_TEXT = 0.015, PER_SEC_TEXT = 0.4;

const S = {
  stats: null, series: [], epubs: [], jobs: [], sel: new Set(), built: new Set(),
  view: 'grid', page: 1, per: 12, q: '', f: 'all', channel: '', author: '', year: '',
  streams: new Map(), read: null, dir: 1, lastToggled: null,
};

const fmtNum = (n) => (n == null ? '—' : Number(n).toLocaleString('en-US'));
const fmtSize = (b) => (!b ? '0 B' : b < 1024 ? b + ' B' : b < 1048576 ? (b / 1024).toFixed(0) + ' KB' : (b / 1048576).toFixed(2) + ' MB');
const fmtEst = (mb) => (mb < 1 ? `${Math.max(1, Math.round(mb * 1024))} KB` : `${Math.round(mb)} MB`);

// 动画小工具：加一次类名、播完就摘掉；重播前强制重排（可被打断、可重放）
function animOnce(el, cls, ms = 400) {
  if (!el) return;
  el.classList.remove(cls);
  void el.offsetWidth;
  el.classList.add(cls);
  setTimeout(() => el.classList.remove(cls), ms);
}
// 数字变了才滚一次，重复点不重播
function bump(el, val) {
  if (!el) return;
  const txt = String(val);
  if (el.textContent === txt) return;
  el.textContent = txt;
  animOnce(el, 'bump', 300);
}
let firstGrid = true;             // 首屏卡片入场只播一次
let lastView = null;              // 视图切换的来路
let shownTickets = new Set();     // 书签：区分新加入 / 已离开
let seenEpubs = new Set();
function fmtDur(s) {
  s = Math.round(s);
  if (s < 60) return s + ' 秒';
  const m = Math.floor(s / 60), r = s % 60;
  return m + ' 分' + (r ? ' ' + r + ' 秒' : '');
}
function fmtDate(v) {
  if (v == null || v === '') return '';
  let d;
  if (typeof v === 'number') d = new Date(v * 1000);
  else if (/^\d+$/.test(String(v))) d = new Date(Number(v) * 1000);
  else d = new Date(v);
  if (isNaN(d)) return String(v);
  const p = (n) => String(n).padStart(2, '0');
  return d.getFullYear() + '-' + p(d.getMonth() + 1) + '-' + p(d.getDate());
}

async function load() {
  const d = await (await fetch('/api/state')).json();
  S.stats = d.stats;
  S.series = d.series || [];
  S.epubs = d.epubs || [];
  S.jobs = d.jobs || [];
  S.built = new Set(S.epubs.map((e) => e.sid).filter(Boolean));
  fillStatic();
  if (S.jobs.some((j) => j.state === 'running' || j.state === 'queued')) S.view = 'jobs';
  renderAll();
  const m = location.hash.match(/^#read\/([^/]+)\/([A-Za-z0-9]+)$/);
  if (m) openReader(m[1] === '-' ? null : m[1], m[2]);
  S.jobs.forEach((j) => { if (j.state === 'running' || j.state === 'queued') follow(j.id); });
}

function fillStatic() {
  const st = S.stats || {};
  $('#s-articles').textContent = fmtNum(st.articles);
  $('#s-assets').textContent = fmtNum(st.assets);
  bump($('#s-built'), fmtNum(S.epubs.length));
  $('#brand-sub').textContent = '专题归档 · ' + fmtNum(st.series) + ' 个专题';
  const chans = new Map(), auths = new Map(), years = new Map();
  for (const s of S.series) {
    if (s.channel) chans.set(s.channel, (chans.get(s.channel) || 0) + 1);
    for (const a of s.authors || []) auths.set(a, (auths.get(a) || 0) + 1);
    for (let y = Number(s.y0) || 0; y && y <= Number(s.y1 || s.y0); y++) years.set(y, (years.get(y) || 0) + 1);
  }
  const opt = (map, sel) => [...map.entries()].sort((a, b) => b[1] - a[1] || String(a[0]).localeCompare(String(b[0])))
    .map(([k, v]) => `<option value="${esc(k)}">${esc(k)} · ${v}</option>`).join('');
  const fc = $('#f-channel'), fa = $('#f-author'), fy = $('#f-year');
  fc.innerHTML = '<option value="">栏目 ' + chans.size + ' ▾</option>' + opt(chans);
  fa.innerHTML = '<option value="">作者 ' + auths.size + ' ▾</option>' + opt(auths);
  fy.innerHTML = '<option value="">年份 ▾</option>' + [...years.keys()].sort((a, b) => b - a).map((y) => `<option value="${y}">${y} · ${years.get(y)}</option>`).join('');
}

const visible = () => {
  const q = S.q.trim().toLowerCase();
  return S.series.filter((s) => {
    if (S.f === 'done' && !S.built.has(s.sid)) return false;
    if (S.f === 'todo' && S.built.has(s.sid)) return false;
    if (S.channel && s.channel !== S.channel) return false;
    if (S.author && !(s.authors || []).includes(S.author)) return false;
    if (S.year) { const y = Number(S.year); if (!(Number(s.y0) <= y && y <= Number(s.y1 || s.y0))) return false; }
    if (q) {
      const hay = (s.title_text + ' ' + (s.authors || []).join(' ') + ' ' + (s.channel || '') + ' ' + s.sid).toLowerCase();
      if (!hay.includes(q)) return false;
    }
    return true;
  });
};

function renderAll() {
  renderTitle();
  renderNav();
  renderFilters();
  renderGrid();
  renderCart();
  renderJobs();
  renderEpubs();
}

function renderTitle() {
  const n = visible().length;
  const st = S.stats || {};
  const R = S.read;
  $('#view-title').textContent = S.view === 'grid' ? '专题'
    : S.view === 'epubs' ? '已装订'
      : S.view === 'read' ? ((R && R.series && R.series.title) || '在线阅读')
        : '装订中';
  if (S.view === 'grid') {
    $('#view-sub').textContent = n === S.series.length
      ? `${fmtNum(st.series)} 个专题 · 收录 ${fmtNum(st.articles)} 篇文章 · ${fmtNum(st.covers)} 个有横幅图`
      : `筛出 ${fmtNum(n)} 个专题`;
    const allOn = n > 0 && visible().every((s) => S.sel.has(s.sid));
    const sb = $('#selall');
    sb.disabled = !n;
    sb.textContent = allOn ? `取消全选 ${fmtNum(n)} 个` : `全选筛出的 ${fmtNum(n)} 个`;
  } else if (S.view === 'epubs') {
    $('#view-sub').textContent = `${S.epubs.length} 本 · 点右侧下载 · 输出到 ${st.out || ''}`;
  } else if (S.view === 'read') {
    const s = R && R.series;
    $('#view-sub').textContent = s
      ? `${fmtNum(s.n)} 篇 · ${(s.authors || []).slice(0, 3).join('、')}${s.y0 ? ' · ' + s.y0 + (s.y1 && s.y1 !== s.y0 ? '–' + s.y1 : '') : ''} · 在线阅读`
      : '在线阅读 · 正文里的文章链接会继续在这里打开';
  } else {
    $('#view-sub').textContent = S.jobs.length ? `最近 ${S.jobs.length} 个任务` : '还没有装订任务';
  }
  for (const v of ['grid', 'epubs', 'jobs', 'read']) $('#view-' + v).hidden = v !== S.view;
  // 视图切换：新视图流淌进来（首次进入交给卡片入场，不叠一层）
  if (lastView !== null && lastView !== S.view) animOnce($('#view-' + S.view), 'enter', 260);
  lastView = S.view;
}

function renderNav() {
  $('#n-series').textContent = fmtNum(S.series.length);
  bump($('#n-built'), fmtNum(S.epubs.length));
  const run = S.jobs.filter((j) => j.state === 'running' || j.state === 'queued').length;
  $('#n-jobs').textContent = fmtNum(run);
  for (const a of document.querySelectorAll('nav a')) a.classList.toggle('on', a.dataset.view === S.view);
}

function renderFilters() {
  $('#c-all').textContent = fmtNum(S.series.length);
  $('#c-done').textContent = fmtNum(S.series.filter((s) => S.built.has(s.sid)).length);
  $('#c-todo').textContent = fmtNum(S.series.filter((s) => !S.built.has(s.sid)).length);
  for (const c of document.querySelectorAll('.chip[data-f]')) c.classList.toggle('on', c.dataset.f === S.f);
}

function renderGrid() {
  const list = visible();
  const pages = Math.max(1, Math.ceil(list.length / S.per));
  if (S.page > pages) S.page = pages;
  const slice = list.slice((S.page - 1) * S.per, S.page * S.per);
  const grid = $('#grid');
  if (!slice.length) {
    grid.innerHTML = '';
    $('#pager').innerHTML = '<span class="n">没有匹配的专题</span>';
    return;
  }
  grid.innerHTML = slice.map((s) => {
    const on = S.sel.has(s.sid);
    const meta = [s.n ? s.n + ' 篇' : '', (s.authors || []).slice(0, 2).join('、'),
      s.y0 ? s.y0 + (s.y1 && s.y1 !== s.y0 ? '–' + s.y1 : '') : ''].filter(Boolean).join('<i> · </i>');
    const rd = s.first ? `<span class="rd" data-aid="${esc(s.first)}" title="在线阅读">阅读</span>` : '';
    const img = s.cover
      ? `<div class="thumb"><img src="/api/cover/${encodeURIComponent(s.sid)}?w=660" alt="">${rd}</div>`
      : `<div class="thumb empty">№&nbsp;${esc(s.sid)}${rd}</div>`;
    const just = on && s.sid === S.lastToggled;
    return `<div class="card${on ? ' sel' : ''}${just ? ' just' : ''}" role="checkbox" aria-checked="${on}" tabindex="0" data-sid="${esc(s.sid)}">
      ${S.built.has(s.sid) ? '<div class="stamp">已装订</div>' : ''}${img}
      <div class="pick">${on ? '✓' : ''}</div>
      <h2>${s.title || esc(s.title_text)}</h2><p>${meta}</p></div>`;
  }).join('');
  S.lastToggled = null;
  grid.querySelectorAll('.card.just').forEach((c) => setTimeout(() => c.classList.remove('just'), 200));  // 播完就摘掉，避免下次重渲染又播一次
  if (firstGrid) {   // 首次进入：主角先到，其余按阅读顺序跟上
    firstGrid = false;
    grid.dataset.enter = '';
    grid.querySelectorAll('.card').forEach((c, i) => c.style.setProperty('--i', i));
    setTimeout(() => { delete grid.dataset.enter; }, 900);
  }
  const from = (S.page - 1) * S.per + 1, to = Math.min(list.length, S.page * S.per);
  const btns = [];
  const win = [];
  for (let p = 1; p <= pages; p++) {
    if (p <= 3 || p > pages - 2 || Math.abs(p - S.page) <= 1) win.push(p);
  }
  let last = 0;
  for (const p of win) {
    if (p - last > 1) btns.push('<span class="gap">…</span>');
    btns.push(p === S.page ? `<button class="cur">${p}</button>` : `<button data-p="${p}">${p}</button>`);
    last = p;
  }
  $('#pager').innerHTML = `<span class="n">显示 ${fmtNum(from)}–${fmtNum(to)} / 共 ${fmtNum(list.length)} 个专题</span>${btns.join('')}`;
}

function cartSum() {
  const list = S.series.filter((s) => S.sel.has(s.sid));
  const arts = list.reduce((a, s) => a + (s.n || 0), 0);
  const withImg = $('#opt-images').checked;
  return {
    list, arts,
    imgs: withImg ? Math.round(arts * PER_IMG) : 0,
    mb: arts * (withImg ? PER_MB : PER_MB_TEXT),
    sec: arts * (withImg ? PER_SEC : PER_SEC_TEXT),
  };
}

function renderCart() {
  const { list, arts, imgs, mb, sec } = cartSum();
  const running = S.jobs.find((j) => j.state === 'running' || j.state === 'queued');
  const opts = [$('#opt-images').checked ? '图片内嵌' : '不嵌图片', $('#opt-comments').checked ? '附评论' : '不带评论'];
  if (running) {
    const pct = running.total ? Math.round((running.done / running.total) * 100) : 0;
    $('#cart-sum').textContent = `装订中 ${running.done}/${running.total} · ${running.current || '准备中'}`;
    $('#cart-msum').textContent = `装订中 ${running.done}/${running.total}`;
    $('#est').innerHTML = `<b>${pct}%</b>${running.current_total ? `第 ${running.current_index}/${running.current_total} 篇` : '准备中'}`;
    $('#bar').style.width = pct + '%';
    $('#go').disabled = true;
    $('#go').textContent = '装订中…';
  } else {
    $('#go').disabled = !S.sel.size;
    $('#go').textContent = '装订成 EPUB';
    $('#bar').style.width = '0';
    if (!S.sel.size) {
      $('#cart-sum').textContent = '还没有选专题 · 点卡片加入';
      $('#cart-msum').textContent = '';
      $('#est').innerHTML = '<b>—</b>&nbsp;';
    } else {
      $('#cart-sum').textContent = `已选 ${S.sel.size} 个专题 · 共 ${fmtNum(arts)} 篇 · ${imgs ? '约 ' + fmtNum(imgs) + ' 张图' : '不嵌图片'}`;
      $('#cart-msum').textContent = `${S.sel.size} 个专题 · ${fmtNum(arts)} 篇 · 约 ${fmtEst(mb)}`;
      $('#est').innerHTML = `<b>预计 ${fmtDur(sec)} · 约 ${fmtEst(mb)}</b>${opts.join(' · ')}`;
    }
  }
  const shown = list.length > 2 ? list.slice(0, 1) : list;
  const rest = list.length - shown.length;
  const box = $('#tickets');
  const now = new Set(list.map((s) => s.sid));
  const leaving = [...box.querySelectorAll('.ticket[data-sid]')].filter((t) => !now.has(t.dataset.sid));
  box.classList.toggle('many', rest > 0);
  box.innerHTML = shown.map((s) => `<div class="ticket" data-sid="${esc(s.sid)}" title="${esc(s.title_text)}"><span>${esc(s.title_text)}</span><u data-rm="${esc(s.sid)}" role="button" aria-label="移出书车">✕</u></div>`).join('')
    + (rest > 0 ? `<div class="ticket more">还有 ${fmtNum(rest)} 个</div>` : '');
  // 走的先留在原地播完退场，新加入的滑进来
  for (const t of leaving) { t.classList.add('out'); box.appendChild(t); setTimeout(() => t.remove(), 140); }
  for (const t of box.querySelectorAll('.ticket[data-sid]')) if (!shownTickets.has(t.dataset.sid)) animOnce(t, 'in', 240);
  shownTickets = now;
}

function renderJobs() {
  const box = $('#job-list');
  if (!S.jobs.length) { box.innerHTML = '<div class="empty-note">还没有装订任务。在「专题」里选几个，然后点右下角的「装订成 EPUB」。</div>'; return; }
  box.innerHTML = S.jobs.map((j) => {
    const pct = j.total ? Math.round((j.done / j.total) * 100) : 0;
    const state = { queued: '排队中', running: '装订中', done: '完成', failed: '失败' }[j.state] || j.state;
    const files = (j.files || []).map((f) => `<div class="epub-row"><span class="nm">${esc(f.name)}</span>
      <span class="sz">${f.pages} 篇 · ${fmtNum(f.images)} 图</span><span class="sz">${fmtSize(f.bytes)}</span>
      <a class="dl" href="/epub/${encodeURIComponent(f.name)}">下载</a></div>`).join('');
    return `<div class="jobbox" data-job="${j.id}">
      <div class="jh"><b>${esc((j.titles || []).join('、') || j.id)}</b><em>${state} · ${j.done}/${j.total}${j.total ? ' · ' + pct + '%' : ''}</em>
        <em>${esc(fmtDate(j.started))}${j.finished ? ' → ' + esc(fmtDate(j.finished)) : ''}</em></div>
      ${j.state === 'running' && j.current ? `<div class="jt">正在装订：${esc(j.current)}${j.current_total ? `（第 ${j.current_index}/${j.current_total} 篇）` : ''}</div>` : ''}
      ${j.error ? `<div class="jt">错误：${esc(j.error)}</div>` : ''}
      <div class="log">${(j.log || []).map((l) => `<span class="ln">${esc(l).replace(/cdn\/[0-9a-f]{6,}\//g, '')}</span>`).join('')}</div>
      ${files ? `<div class="epub-list">${files}</div>` : ''}
    </div>`;
  }).join('');
}

function renderEpubs() {
  const box = $('#epub-list');
  if (!S.epubs.length) { box.innerHTML = '<div class="empty-note">epubs 目录里还没有文件。</div>'; return; }
  box.innerHTML = S.epubs.map((e) => `<div class="epub-row">
    <span class="nm">${esc(e.name)}</span>
    <span class="sz">${e.sid ? '№ ' + esc(e.sid) : ''}</span>
    <span class="size">${fmtSize(e.size)}</span>
    <span class="dt">${esc(fmtDate(e.mtime))}</span>
    <a class="dl" href="/epub/${encodeURIComponent(e.name)}">下载</a></div>`).join('');
  // 刚装订出来的那几行，淡入上移一次
  const now = new Set(S.epubs.map((e) => e.name));
  box.querySelectorAll('.epub-row').forEach((r, i) => {
    if (S.epubs[i] && !seenEpubs.has(S.epubs[i].name)) animOnce(r, 'in', 300);
  });
  seenEpubs = now;
}

function toggle(sid, on) {
  if (on === undefined) on = !S.sel.has(sid);
  if (on) S.sel.add(sid); else S.sel.delete(sid);
  S.lastToggled = sid;
  renderGrid(); renderCart();
}

async function refreshOutputs() {
  try {
    const e = await (await fetch('/api/epubs')).json();
    S.epubs = e.epubs || [];
    S.built = new Set(S.epubs.map((x) => x.sid).filter(Boolean));
    renderNav(); renderFilters(); renderGrid(); renderEpubs(); renderTitle();
  } catch (_) { }
}

function follow(id) {
  if (S.streams.has(id)) return;
  const es = new EventSource('/api/jobs/' + encodeURIComponent(id) + '/stream');
  S.streams.set(id, es);
  es.onmessage = (ev) => {
    let d; try { d = JSON.parse(ev.data); } catch (_) { return; }
    const j = d.job; if (!j) return;
    const i = S.jobs.findIndex((x) => x.id === j.id);
    if (i >= 0) S.jobs[i] = j; else S.jobs.unshift(j);
    renderJobs(); renderCart(); renderNav();
    if (j.state === 'done' || j.state === 'failed') {
      es.close(); S.streams.delete(id); refreshOutputs();
    }
  };
  es.onerror = () => { /* SSE 断了就等下一次 load() */ };
}

async function start() {
  const sids = [...S.sel];
  if (!sids.length) return;
  $('#go').disabled = true;
  const res = await fetch('/api/jobs', {
    method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ sids, embed_images: $('#opt-images').checked, comments: $('#opt-comments').checked }),
  });
  if (!res.ok) { $('#est').innerHTML = '<b>启动失败</b>' + (await res.text()); $('#go').disabled = false; return; }
  const { job } = await res.json();
  S.jobs.unshift(job);
  S.sel.clear();
  S.view = 'jobs';
  renderAll();
  follow(job.id);
}

// ---------- 在线阅读 ----------
let routing = false;
function setHash(h) {
  if (location.hash === h) return;
  routing = true;
  if (h) location.hash = h;
  else history.replaceState(null, '', location.pathname + location.search);
  setTimeout(() => { routing = false; }, 0);
}

async function openReader(sid, aid) {
  const series = sid ? S.series.find((s) => s.sid === sid) : null;
  if (series && !series.items) {
    try {
      const d = await (await fetch('/api/series/' + encodeURIComponent(sid))).json();
      series.items = d.items || [];
      series.title = d.title || series.title;
      series.title_text = d.title_text || series.title_text;
    } catch (_) { series.items = []; }
  }
  const items = (series && series.items) || [];
  let i = aid ? items.findIndex((x) => x.aid === aid) : -1;
  if (i < 0 && items.length) i = 0;
  const cur = i >= 0 ? items[i] : { aid };
  S.read = { sid: sid || null, series: series || null, items, i, aid: cur.aid };
  S.view = 'read';
  setHash('#read/' + (sid || '-') + '/' + cur.aid);
  renderAll();
  renderRead();
}

function closeReader() {
  S.read = null;
  S.view = 'grid';
  setHash('');
  renderAll();
}

function goChapter(k) {
  const R = S.read;
  if (!R) return;
  if (!R.items[k]) { animOnce($('#rd-body'), k < 0 ? 'nudge-r' : 'nudge-l', 220); return; }  // 到头：轻轻顶一下
  S.dir = k > R.i ? 1 : -1;
  R.i = k;
  R.aid = R.items[k].aid;
  setHash('#read/' + (R.sid || '-') + '/' + R.aid);
  renderRead();
}

async function renderRead() {
  const R = S.read; if (!R) return;
  if (R.items.length) {
    $('#rd-chapters').innerHTML = `<div class="hd">共 ${R.items.length} 篇</div>` + R.items.map((x, k) =>
      `<a class="${k === R.i ? 'on' : ''}" data-k="${k}" href="#read/${R.sid || '-'}/${esc(x.aid)}">${esc(x.title || x.aid)}<span class="m">${esc(x.author || '')}${x.time ? ' · ' + esc(x.time) : ''}</span></a>`).join('');
    $('#rd-pick').innerHTML = R.items.map((x, k) => `<option value="${k}"${k === R.i ? ' selected' : ''}>${k + 1}：${esc(x.title || x.aid)}</option>`).join('');
    $('#rd-pos').textContent = `第 ${R.i + 1} / ${R.items.length} 篇 · 点左侧目录切换，← → 翻篇`;
  } else {
    $('#rd-chapters').innerHTML = '';
    $('#rd-pick').innerHTML = '';
    $('#rd-pos').textContent = '← 回到专题列表';
  }
  const body = $('#rd-body');
  body.innerHTML = '<p class="meta">正在读取…</p>';
  try {
    const d = await (await fetch('/api/read/' + encodeURIComponent(R.aid))).json();
    const prev = R.i > 0 ? R.items[R.i - 1] : null;
    const next = R.i >= 0 && R.i < R.items.length - 1 ? R.items[R.i + 1] : null;
    const src = R.series ? ` · 收录于《${esc(R.series.title_text || R.series.title)}》` : '';
    body.innerHTML = `<h1>${esc(d.title || R.aid)}</h1>
      <p class="meta">${esc(d.author || '')}${d.time ? ' · ' + esc(d.time) : ''}${src}</p>
      ${d.html || ''}
      <div class="rd-nav">${prev ? `<a data-k="${R.i - 1}" href="#read/${R.sid || '-'}/${esc(prev.aid)}">← ${esc(prev.title || '上一篇')}</a>` : ''}${next ? `<a data-k="${R.i + 1}" href="#read/${R.sid || '-'}/${esc(next.aid)}">${esc(next.title || '下一篇')} →</a>` : ''}</div>`;
    animOnce(body, S.dir < 0 ? 'in-l' : 'in-r', 260);   // 翻篇方向一致：往后从右来，往前从左来
    // 归档里没有的外链图（如 bilibili 缩略图）加载失败时直接隐藏，别留破图标
    body.querySelectorAll('img').forEach((im) => {
      const dead = () => { im.remove(); };
      im.addEventListener('error', dead);
      if (im.complete && im.naturalWidth === 0) dead();
    });
    const v = $('#view-read'); if (v) v.scrollTop = 0;
    if (!v || v.scrollHeight <= v.clientHeight) window.scrollTo({ top: 0 });
  } catch (e) {
    body.innerHTML = '<p class="meta">读取失败：' + esc(String(e)) + '</p>';
  }
}

// ---------- 事件 ----------
$('#grid').addEventListener('click', (e) => {
  const rd = e.target.closest('.rd');
  if (rd) {
    const c = e.target.closest('.card');
    if (c) openReader(c.dataset.sid, rd.dataset.aid);
    return;
  }
  const c = e.target.closest('.card'); if (!c) return;
  toggle(c.dataset.sid);
});
$('#grid').addEventListener('keydown', (e) => {
  if (e.key !== ' ' && e.key !== 'Enter') return;
  const c = e.target.closest('.card'); if (!c) return;
  e.preventDefault(); toggle(c.dataset.sid);
});
$('#pager').addEventListener('click', (e) => {
  const b = e.target.closest('button[data-p]'); if (!b) return;
  S.page = Number(b.dataset.p); renderGrid();
  document.querySelector('main').scrollTo({ top: 0 });
});
$('#tickets').addEventListener('click', (e) => {
  const u = e.target.closest('u[data-rm]'); if (!u) return;
  toggle(u.dataset.rm, false);
});
for (const c of document.querySelectorAll('.chip[data-f]')) {
  c.addEventListener('click', () => { S.f = c.dataset.f; S.page = 1; renderFilters(); renderGrid(); renderTitle(); });
}
$('#f-channel').addEventListener('change', (e) => { S.channel = e.target.value; e.target.classList.toggle('on', !!e.target.value); S.page = 1; renderGrid(); renderTitle(); });
$('#f-author').addEventListener('change', (e) => { S.author = e.target.value; e.target.classList.toggle('on', !!e.target.value); S.page = 1; renderGrid(); renderTitle(); });
$('#f-year').addEventListener('change', (e) => { S.year = e.target.value; e.target.classList.toggle('on', !!e.target.value); S.page = 1; renderGrid(); renderTitle(); });
$('#q').addEventListener('input', (e) => { S.q = e.target.value; S.page = 1; renderGrid(); renderTitle(); });
$('#clear').addEventListener('click', () => { S.sel.clear(); renderGrid(); renderCart(); });
$('#selall').addEventListener('click', () => {
  const v = visible();
  const all = v.length > 0 && v.every((s) => S.sel.has(s.sid));
  for (const s of v) { if (all) S.sel.delete(s.sid); else S.sel.add(s.sid); }
  renderTitle(); renderGrid(); renderCart();
});
$('#go').addEventListener('click', start);
$('#opt-images').addEventListener('change', renderCart);
$('#opt-comments').addEventListener('change', renderCart);
for (const a of document.querySelectorAll('nav a')) {
  a.addEventListener('click', (e) => { e.preventDefault(); S.view = a.dataset.view; renderAll(); if (S.view === 'epubs') refreshOutputs(); });
}
$('#rd-back').addEventListener('click', closeReader);
$('#rd-chapters').addEventListener('click', (e) => {
  const a = e.target.closest('a[data-k]'); if (!a) return;
  e.preventDefault();
  goChapter(Number(a.dataset.k));
});
$('#rd-pick').addEventListener('change', (e) => goChapter(Number(e.target.value)));
$('#rd-body').addEventListener('click', (e) => {
  const nav = e.target.closest('.rd-nav a[data-k]');
  if (nav) { e.preventDefault(); goChapter(Number(nav.dataset.k)); return; }
  const a = e.target.closest('a'); if (!a) return;
  const m = (a.getAttribute('href') || '').match(/\/article\/([A-Za-z0-9]+)/);
  if (!m) return;
  e.preventDefault();
  const R = S.read || {};
  const k = (R.items || []).findIndex((x) => x.aid === m[1]);
  if (k >= 0) goChapter(k); else openReader(null, m[1]);
});
window.addEventListener('hashchange', () => {
  if (routing) return;
  const m = location.hash.match(/^#read\/([^/]+)\/([A-Za-z0-9]+)$/);
  if (m) {
    const sid = m[1] === '-' ? null : m[1];
    const R = S.read;
    if (!R || R.aid !== m[2] || (R.sid || null) !== sid) openReader(sid, m[2]);
  } else if (S.view === 'read') closeReader();
});
document.addEventListener('keydown', (e) => {
  if (S.view === 'read' && S.read) {
    if (e.key === 'Escape') { closeReader(); return; }
    if (e.key === 'ArrowLeft') { goChapter(S.read.i - 1); return; }
    if (e.key === 'ArrowRight') { goChapter(S.read.i + 1); return; }
  }
  if (e.key === 'Escape') { S.sel.clear(); renderGrid(); renderCart(); }
});

load().catch((err) => { $('#view-sub').textContent = '读取归档失败：' + err; });
