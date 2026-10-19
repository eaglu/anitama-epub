Anitama-Backup-2020  (archive.org: Anitama-Backup-2020, CC BY-NC-SA 4.0)
============================================================
目录说明
  Anitama-Backup/     阅读用站点（article/ channel/ series/ static/ cdn/ index.html）
                      html 里的链接是绝对路径 /Anitama-Backup/...，
                      所以必须挂在 web 根目录下用 HTTP 打开（服务见下），file:// 直接点开会缺样式和图片。
  originals/          原始版本与评论数据（item 说明里说阅读不需要）：article_o/ channel_o/ series_o/ comment/
  cdn-7z-volumes/     CDN 媒体归档的分卷（.001-.039）。注意：archive.org 上第一卷被上传者包成了 001.zip，
                      里面 Anitama-Backup-CDN.data 就是 .001（7z 魔数已核对），这里已改名补齐。
  Articles/           原始归档：article(.o).7z channel(.o).7z series(.o).7z
  Media Files/        原始归档：001.zip + Anitama-Backup-CDN.7z.002-.039 + Anitama-CDN-Backup-Accp.7z（未改动）
  Miscellaneous/      原始归档：comment.7z static.7z index.html
  _metadata/          archive.org 官方元数据（_files.xml/_meta.xml/_archive.torrent），对账用

完整性
  51/52 文件 md5 与官方 metadata 一致；唯一不符的 _files.xml 是 archive.org 每次请求重新生成的索引（非内容）。

字符集（重要）
  归档 html 是 UTF-8，但部分页面（含 index.html）没有写 <meta charset>。
  所以静态服务必须显式发 `text/html; charset=utf-8`，否则中文环境浏览器会按 GBK 猜 → 全页乱码。
  /usr/local/bin/anitama-serve.py 已处理（text/*、js/json/svg 都带 charset）。

手机端适配
  归档是 2018 年的桌面布局（固定 850/1120px），且 index.html 没有 viewport meta，
  手机上会按 980px 渲染（字只有 10px）、正文要横向拖。
  /usr/local/bin/anitama-serve.py 在响应 html 时动态注入 viewport + 一段只对 ≤820px 生效的 CSS
  （宽度流体化、正文 17px、图片自适应、导航条可横滑、页面不横向滚动、宽表格自己滚），
  并按魔数给无扩展名的 CDN 资源发正确的 Content-Type（否则是 octet-stream，Safari 可能不渲染）。
  归档文件本身没动，桌面端视图不受影响。

浏览
  http://192.168.100.6:8090/Anitama-Backup/index.html   （systemd: anitama-archive.service）
