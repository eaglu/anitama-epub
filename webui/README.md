# Anitama 装订台（WebUI）

浏览归档里的 330 个专题，勾选若干，装订成 EPUB（正文图片内嵌、文末附评论区、PIL 同款封面），在线看进度、下载成品。

- 本工具（Rust / axum）：默认 `http://127.0.0.1:8095`
- 归档站点（原阅读站，Python）：`/usr/local/bin/anitama-serve.py <root> 8090`，两者互不影响
- 输出目录：`<root>/epubs/`（一本专题一个文件，同名直接覆盖）

## 构建与运行

`cargo` 不在 PATH 里（rustup 装在 `~/.cargo`），用绝对路径或先补 PATH：

```bash
cd webui
/home/faye/.cargo/bin/cargo build --release     # 或 export PATH="$HOME/.cargo/bin:$PATH"
./start.sh                                      # 前台等就绪；已在跑就直接返回
./start.sh /other/archive 8096                  # 换归档目录/端口
```

`start.sh` 优先用 `target/release/anitama-epub-web`，没有就退回 debug；日志默认 `/tmp/anitama-epub-web.log`（可用 `LOG=` 改）。
参数是**归档根目录**（默认 `..`）。启动时会扫一遍 `Anitama-Backup/series/*/*.html` 与 `cdn/`，约 4~7 秒后才监听端口。

生产构建：`/home/faye/.cargo/bin/cargo build --release` → `target/release/anitama-epub-web`。

### 开机自启（systemd 用户服务）

仓库里带了 `webui/anitama-epub.service`，已安装并启用：

```bash
cp webui/anitama-epub.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now anitama-epub.service
systemctl --user status anitama-epub.service      # 看状态
journalctl --user -u anitama-epub.service -f      # 看日志
loginctl enable-linger "$USER"                    # 不用登录也能开机起（已设 Linger=yes）
```

环境变量：

| 变量 | 默认 | 说明 |
| --- | --- | --- |
| `BIND` | `0.0.0.0` | 监听地址（`start.sh` 与 systemd 服务都按此设置；只给本机用就 `BIND=127.0.0.1 ./start.sh`） |
| `PORT` | `8095` | 端口（8091 被 Motrix 占用） |
| `STATIC_DIR` | `<root>/webui/static` | 前端静态目录 |
| `RUST_LOG` | `info,anitama_epub_web=debug` | 日志级别 |

## 源码

| 文件 | 职责 |
| --- | --- |
| `src/main.rs` | axum 路由、SSE、封面/下载响应、启动 |
| `src/archive.rs` | 扫描专题页 → `Series{title,n,authors,channel,y0,y1,items}`；缩略图缓存；已装订列表 |
| `src/html.rs` | 解析专题页/文章页/评论；重写正文里的 `img/video/a`（本地路径、`javascript:`、裸域名） |
| `src/epub.rs` | EPUB3 打包：OPF + nav + NCX + 正文页 + 评论 + 样式常量 |
| `src/cover.rs` | 用 image + rusttype 画 1200×1800 封面（对齐 Python/PIL 版的字号与布局） |
| `src/jobs.rs` | 装订队列（串行）、进度广播、产出清单 |
| `src/util.rs` | 转义、时间解析、文件名清洗、图片类型嗅探 |
| `static/index.html`、`static/app.js` | 前端（书房·索引卡，无构建工具、无外部依赖） |

## HTTP 接口

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| GET | `/api/state` | 统计、330 个专题、任务、已装订 |
| GET | `/api/series/:sid` | 单个专题（含收录文章） |
| GET | `/api/cover/:sid?w=660` | 专题横幅缩略图（JPEG，带缓存头） |
| GET | `/api/epubs` | `epubs/` 里的成品 |
| GET | `/api/jobs` | 任务列表（最新在前） |
| GET | `/api/jobs/:id` | 单个任务 |
| POST | `/api/jobs` | 开始装订：`{sids:[...], embed_images:bool, comments:bool}` |
| GET | `/api/jobs/:id/stream` | SSE 进度（先发当前快照，再推增量） |
| POST | `/api/rescan` | 重新扫描归档 |
| GET | `/epub/:name` | 下载成品（`filename*=UTF-8''`） |

## 备注

- 改 `static/` 下的 HTML/JS **不需要重新编译**（`ServeDir` 直接读磁盘）；改 Rust 需要 `cargo build` 并重启。
- 「图片内嵌」不勾选时：正文里的图片元素被去掉，`epubs` 里也**不会**写入任何图片文件（体积很小）；封面仍会生成。
- 重启时别用 `pkill -f 'target/debug/anitama-epub-web'`——那条命令的模式会匹配到执行它的 shell 自身；用 `pkill -f 'anitama-epub-[w]eb'` 或按 PID 结束。
