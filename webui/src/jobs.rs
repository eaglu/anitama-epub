//! 装订任务队列：串行执行，进度通过 broadcast 推给 SSE。
use crate::archive::Archive;
use crate::epub::{self, BuildOpts, Progress};
use crate::util::{now_iso, safe_name};
use serde::Serialize;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

/// 日志里用：按字符截断，超长补省略号（中英混排的行宽按列算，取小一点保险）。
pub fn trunc(s: &str, n: usize) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i >= n {
            out.push('…');
            break;
        }
        out.push(c);
    }
    out
}

#[derive(Serialize, Clone, Debug)]
pub struct OutFile {
    pub name: String,
    pub sid: String,
    pub pages: usize,
    pub images: usize,
    pub bytes: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct Job {
    pub id: String,
    pub sids: Vec<String>,
    pub titles: Vec<String>,
    pub state: String, // queued | running | done | failed
    pub embed_images: bool,
    pub comments: bool,
    pub done: usize,
    pub total: usize,
    pub current: String,
    pub current_index: usize,
    pub current_total: usize,
    pub files: Vec<OutFile>,
    pub log: Vec<String>,
    pub started: String,
    pub finished: Option<String>,
    pub error: Option<String>,
}

#[derive(Serialize, Clone)]
struct Ev {
    job: Job,
}

struct Inner {
    jobs: Mutex<Vec<Job>>,
    tx: broadcast::Sender<String>,
    seq: Mutex<u64>,
}

#[derive(Clone)]
pub struct Jobs {
    inner: Arc<Inner>,
}

impl Jobs {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(512);
        Jobs {
            inner: Arc::new(Inner {
                jobs: Mutex::new(Vec::new()),
                tx,
                seq: Mutex::new(0),
            }),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.inner.tx.subscribe()
    }

    pub fn list(&self) -> Vec<Job> {
        let mut v = self.inner.jobs.lock().unwrap().clone();
        v.reverse();
        v
    }

    pub fn get(&self, id: &str) -> Option<Job> {
        self.inner
            .jobs
            .lock()
            .unwrap()
            .iter()
            .find(|j| j.id == id)
            .cloned()
    }

    fn update(&self, id: &str, f: impl FnOnce(&mut Job)) {
        let job = {
            let mut jobs = self.inner.jobs.lock().unwrap();
            let Some(j) = jobs.iter_mut().find(|j| j.id == id) else {
                return;
            };
            f(j);
            j.clone()
        };
        let _ = self.inner.tx.send(serde_json::to_string(&Ev { job }).unwrap_or_default());
    }

    /// 启动一个装订任务（返回快照，执行在后台串行进行）。
    pub fn start(
        &self,
        arc: Arc<Archive>,
        sids: Vec<String>,
        embed_images: bool,
        comments: bool,
    ) -> Job {
        let id = {
            let mut s = self.inner.seq.lock().unwrap();
            *s += 1;
            format!("j{}", *s)
        };
        let titles: Vec<String> = sids
            .iter()
            .filter_map(|s| arc.by_sid(s).map(|x| x.title_text.clone()))
            .collect();
        let job = Job {
            id: id.clone(),
            sids: sids.clone(),
            titles,
            state: "running".into(),
            embed_images,
            comments,
            done: 0,
            total: sids.len(),
            current: String::new(),
            current_index: 0,
            current_total: 0,
            files: Vec::new(),
            log: vec![format!("任务 {} · 共 {} 个专题", id, sids.len())],
            started: now_iso(),
            finished: None,
            error: None,
        };
        self.inner.jobs.lock().unwrap().push(job.clone());
        let _ = self
            .inner
            .tx
            .send(serde_json::to_string(&Ev { job: job.clone() }).unwrap_or_default());

        let me = self.clone();
        let id2 = id.clone();
        tokio::task::spawn_blocking(move || {
            let opts = BuildOpts {
                embed_images,
                comments,
            };
            let mut used: HashSet<String> = HashSet::new();
            for (k, sid) in sids.iter().enumerate() {
                let Some(series) = arc.by_sid(sid) else {
                    me.update(&id2, |j| j.log.push(format!("跳过：找不到专题 {sid}")));
                    continue;
                };
                let mut stem = safe_name(&series.title, 80);
                if stem.is_empty() {
                    stem = format!("series-{sid}");
                }
                // 同名直接覆盖：一个专题固定对应一个 epub 文件名（和 Python 版 cli 一致）
                let cand = stem.clone();
                used.insert(cand.clone());
                let out = arc.out.join(format!("{cand}.epub"));

                me.update(&id2, |j| {
                    j.current = series.title_text.clone();
                    j.current_index = k + 1;
                    j.current_total = series.n;
                    j.log.push(format!(
                        "[{}] 开印 {} · {} 篇",
                        k + 1,
                        trunc(&series.title_text, 22),
                        series.n
                    ));
                });

                let mut report = |p: Progress| match p {
                    Progress::Article { i, total, title } => me.update(&id2, |j| {
                        j.current_index = k + 1;
                        j.current_total = total;
                        j.log.push(format!("  {i}/{total} 篇 · {}", trunc(&title, 20)));
                    }),
                    Progress::Comments { n } => me.update(&id2, |j| {
                        j.log.push(format!("  评论 {n} 条"));
                    }),
                    Progress::Images(n) => me.update(&id2, |j| {
                        j.log.push(format!("  内嵌素材 {n} 个"));
                    }),
                };

                match epub::build_epub(
                    &arc.site,
                    &arc.root,
                    sid,
                    &series.title,
                    series.cover.as_deref(),
                    &series.raw_items(),
                    &out,
                    &opts,
                    &mut report,
                ) {
                    Ok(b) => {
                        let f = OutFile {
                            name: format!("{cand}.epub"),
                            sid: sid.clone(),
                            pages: b.pages,
                            images: b.images,
                            bytes: b.bytes,
                        };
                        me.update(&id2, |j| {
                            j.log.push(format!(
                                "  完成 · {} 篇 · {} 图 · {:.2} MB",
                                b.pages,
                                b.images,
                                b.bytes as f64 / 1048576.0
                            ));
                            j.files.push(f);
                            j.done += 1;
                        });
                    }
                    Err(e) => {
                        me.update(&id2, |j| {
                            j.error = Some(format!("{}: {e:#}", series.title_text));
                            j.log.push(format!("  失败：{e:#}"));
                            j.done += 1;
                        });
                    }
                }
                // 日志只留最近 400 行
                me.update(&id2, |j| {
                    if j.log.len() > 400 {
                        let cut = j.log.len() - 400;
                        j.log.drain(0..cut);
                    }
                });
            }
            me.update(&id2, |j| {
                j.state = if j.error.is_some() { "failed".into() } else { "done".into() };
                j.current = String::new();
                j.finished = Some(now_iso());
                let done = j.done;
                j.log.push(format!("任务结束 · 完成 {done} 个"));
            });
        });
        job
    }
}
