# 性能目标与测量

面向 10,000 首 / 约 500–800 张专辑的本地库，目标平台为 Linux；Windows/macOS 只在 CI 编译，未实测。
下表的“目标”是发布门槛，“现状”是代码审阅结论。`fb-library` 已带可复跑的后端计时 example（见“本机实测”和“测量方法”），但 UI 帧时间、启动和内存仍无内置计时，标“未测”的项需要在发布前补测。

## 目标

| 场景 | 指标 | 目标 | 现状 |
| --- | --- | --- | --- |
| 冷扫描 10k 首（SSD、标签完整） | 墙钟时间 | ≤ 90 s | 未测；单线程逐目录扫描 + 逐文件 lofty 解析 |
| 重扫未变化目录 | 墙钟时间 | ≤ 90 s | 未测；没有 mtime/size 跳过，仍全量解析标签 |
| `Library::snapshot()` 10k 首 | 单次耗时 | ≤ 100 ms | 合成素材后端实测约 4.7–5.1 ms；真实标签/封面库未测 |
| 启动到可交互（约 500 张封面） | 墙钟时间 | ≤ 1.5 s | 未测；所有封面在首帧前同步解码（见下） |
| 滚动 10k 曲目列表 | 帧时间 p95 | ≤ 16.7 ms（60 Hz） | 未测；列表已虚拟化，静态模型按版本缓存，动态字段与 `playback_view` 查找仍逐帧 |
| 搜索输入（10k 首） | 每次按键 p95 | ≤ 16 ms | 未测；每帧对全库做小写化 + `contains` |
| 封面解码卡顿 | 单次主线程停顿 | ≤ 50 ms | 未测；重扫/启动时一次性解码全部缺失封面 |
| EQ 参数生效（本地文件） | 可听间隙 | ≤ 150 ms | 未测；停旧源 → 重开文件 → seek |
| 远程缓存命中后起播 | 点击到出声 | ≤ 100 ms | 未测；缓存命中路径只做文件存在性检查 |
| 远程首次播放 | — | 受网络限制 | 整首下载完成后才起播，不支持边下边播 |
| 常驻内存（10k 库 + 500 封面） | RSS | ≤ 400 MB | 按现实现大概率不达标：封面缓存无上限，512² RGBA 约 1 MiB/张，500 张约 500 MiB（524 MB），需 LRU/缩略图（优化项 3）。未测 |

这些数字是门槛，不是承诺；改动后用同一套素材复测并更新本表。

## 本机实测（2026-11-21）

M6 性能验收，只覆盖 `fb-library` 后端；环境与素材：

- Linux 7.3.4-1-cachyos (x86_64)，AMD Ryzen 5 9600X（6 核 12 线程），16 GiB 内存。
- rustc/cargo 1.99.0，`--release`（`target/release/examples/bench`）。
- 合成素材：10,000 个 46 字节 WAV（44 字节 header + 2 个 8 kHz / 8-bit / 单声道样本，data chunk
  2 字节，满足 RIFF 偶数对齐），分 10 个 `dir-NNNN/` 子目录，文件名 `song-00001.wav`…
  `song-10000.wav`；无标签、无封面。
- 页缓存未清，属于热缓存；每项单次计时，tmpfs 行为 4 次运行的范围。
- 命令：`cargo run --release -p fb-library --example bench -- 10000`（默认 `N = 10000`）。
  默认落在 `/tmp`（本机为 tmpfs）；把 `TMPDIR` 指向磁盘可看真实文件系统开销。

| 场景 | tmpfs（`/tmp`） | ZFS（`target/`） | 当前目标 | 结论 |
| --- | --- | --- | --- | --- |
| 生成 10k 文件 | 18.5–24.4 ms | 94.7 ms | — | — |
| `scan_dir_with_options` | 20.2–21.2 ms（10,000 曲目，0 错误） | 45.7 ms | 冷扫描 ≤ 90 s | 本素材远低于目标* |
| `Library::open` | 0.58–0.76 ms | 27.4 ms | — | — |
| `replace_folder_with_covers` | 13.9–14.6 ms | 38.6 ms | — | — |
| `snapshot()` | 4.72–4.94 ms（10,000 曲目，1 专辑，0 播放列表） | 5.64 ms | ≤ 100 ms | 达成* |
| `search("song-05001")` | 0.78–0.83 ms（1 条结果） | 1.35 ms | 每键 p95 ≤ 16 ms（UI，未测） | 后端单次 search < 16 ms；UI 每键 p95 未测 |

\* 合成 WAV 极小且无标签/封面，扫描几乎只测到目录遍历和 `lofty` 最小解析；10000 首共用一个
“Unknown Album”分组，所以 albums = 1。真实 10k 库（大文件、FLAC/MP3 标签、目录封面、冷缓存）
的扫描与入库会明显更慢，不能把上表当成真实素材的验收结果。启动、滚动帧时间、搜索输入 p95、
封面解码停顿、RSS、EQ/远程缓存本机均未测，见“目标”表的“未测”项与“优化项”。

后端在这套素材上没有出现瓶颈：扫描/入库为几十毫秒、snapshot 约 5 ms、SQL 搜索约 1 ms，
均低于当前门槛。因此本机没有新增待优化项；已知风险仍是 UI 侧的 `playback_view` 每帧线性查找、
客户端搜索和同步封面解码，以及真实标签解析与冷缓存下的扫描耗时。

## 测量方法

1. **统一构建与素材**：`cargo build --release -p fb-app`；用独立的 `FOOBAR_RS_DATA_DIR` 和
   `FOOBAR_RS_CONFIG_DIR`，避免旧库/旧缓存污染。素材建议 10,000 个真实编码文件、500+ 专辑、
   每首带 title/artist/album/track/duration 标签；把素材放在本地 SSD，并记录是否清过页缓存
   （`sync; echo 3 | sudo tee /proc/sys/vm/drop_caches` 为可选冷缓存步骤）。
2. **扫描/启动**：纯后端扫描/入库/snapshot/SQL 搜索用 `cargo run --release -p fb-library --example
   bench -- 10000` 复测，输出已分别计时生成、`scan_dir_with_options`、`Library::open`、
   `replace_folder_with_covers`、`snapshot` 和 `search`。UI 端到端扫描耗时仍只能从点击“扫描全部”
   到状态栏出现结果用秒表/录屏计时；`hyperfine --warmup 1 './target/release/fb-app'` 只能量进程
   启动开销，GUI 首帧/可交互时间需日志或录屏。
3. **帧时间**：滚动/输入时用 `perf record -g -p $(pgrep -x fb-app)` 或 `samply record` 采 10 s，
   看主线程热点是否落在 `rebuild_static_model`、`playback_view`、`visible_tracks`、封面解码或 SQLite；条件允许时临时用
   `ctx.input(|i| i.stable_dt)` 打点，确认 p95 而不是平均值。
4. **数据库**：对 `library.sqlite` 执行 `sqlite3 library.sqlite '.timer on'` 加 `EXPLAIN QUERY PLAN`
   复核搜索/快照 SQL；当前搜索是 `LIKE '%q%'`，无法用索引（见下）。
5. **内存**：`/usr/bin/time -v ./target/release/fb-app` 或 `heaptrack` 看峰值 RSS；封面缓存是
   `HashMap<String, Arc<ColorImage>>`，按数量 × 1 MiB 估算。
6. **体积**：`cargo bloat --release -p fb-app`；随包 Noto Sans SC 约 10.6 MB，属于已知常量。

## 现状与瓶颈

### 主线程上的同步工作

- **模型重建**：`App::sync_model` 每帧把模型送进 UI 并收回上一份；待发送缓冲的 snapshot/covers
  版本落后时原地重建静态部分（曲目/专辑/播放列表/封面），动态字段仍逐帧更新，不再每帧产生 3N 次
  字符串克隆。一次快照变更最多触发两次重建（两份缓冲各一次），之后稳定。列表虚拟化进一步减少
  排版/绘制。剩余热点是 `playback_view` 每帧线性查找当前播放曲目、客户端搜索每帧
  `to_lowercase()` + `contains`，以及同步封面解码。
- **同步 SQLite 调用**：选择/创建/加入播放列表、评分、标签编辑、`reload_snapshot`、`refresh_covers`
  里的 `library.cover(hash)` 都在 GUI 线程执行。库在 10k 规模通常低于一帧，但机械盘、WAL
  检查点或标签写回时会直接卡界面。
- **封面解码**：`refresh_covers` 在启动和每次扫描/重载后调用 `load_cover_cache`，对**所有**缺失封面
  依次读库、`image::load_from_memory`、缩到 512 px、转 RGBA。500 张就是 500 次同步解码，
  首帧可能被推迟数百毫秒到数秒；`cover_cache` 只增不删，500 张约占 500 MiB。
- **搜索**：`fb-ui` 的搜索框是纯客户端过滤（`matches_query`），每帧对全库 title/artist/album 做
  `to_lowercase()` 再 `contains`，每次按键都会产生 3N 次临时字符串。`fb-library` 有 SQL `LIKE`
  搜索，但宿主没有走它。
- **虚拟化**：专辑网格和曲目列表使用 `ScrollArea::show_rows` 固定行高，只排版可见行；配合上面的
  模型缓存，大列表的重建与绘制都按变化/可见范围收敛，但客户端过滤和封面解码仍未覆盖。
- **重绘节奏**：空闲时事件驱动；播放、扫描、远程下载、插件任务、转换或托盘监听期间每 200 ms
  请求一帧。位置每秒最多同步一次到系统媒体控制，开销可忽略。

### 优化项（按影响排序）

1. ~~**模型缓存/共享**~~（已完成）：静态模型（曲目/专辑/播放列表/封面）在待发送缓冲版本落后时
   原地重建（一次快照变更最多重建两份缓冲各一次），动态字段仍逐帧更新，不再每帧做 3N 次字符串
   分配。收益未量化：帧时间与 RSS 尚未实测。
2. **搜索预计算**：模型重建时为每首曲目生成一份小写 `title\0artist\0album` 哈希字段，过滤时只做
   `contains`；输入加 100–150 ms 去抖，或改走 SQLite `LIKE`/FTS5 并限制返回条数。当前 SQL
   `LIKE '%q%'` 也不能用索引，若走数据库需同时加 FTS5 或前缀索引。
3. **封面异步 + 限量**：解码放到 worker（`image` 解码可并行），主线程只做纹理上传；每帧上传
   上限 1–2 张，只解码当前视口/最近专辑；缓存改成有上限的 LRU（例如 200 张 ≈ 200 MiB），
   或存缩略图（256 px ≈ 256 KiB/张）。远程封面接入时同样走这条路。
4. **DB 读写下放**：复用现有 `fb-library-scan` / `plugin_worker` 的“请求 + 事件”模式，把播放列表
   写操作和标签编辑移到 worker，结果通过事件回主线程；快照只在完成后替换。
5. **扫描并行与增量**：目录遍历和 lofty 解析按 2–4 线程并行，DB 批量提交；表中已有 `modified`/`size`，
   可先 stat 再决定是否重新解析，使重扫接近 O(文件数) 而非 O(标签解析数)。
6. **EQ 旁路**：中性设置（全 0 dB 或关闭）直接跳过 `EqSource`，避免切换开关时无意义重建；进一步
   可让 rodio 源支持在原解码器上更新系数（当前 API 只能重建）。
7. **远程缓存**：边下边播（HTTP Range/206 已由流模块支持）、可取消、断点续传、启动清理 `.part`、
   把 2 GiB “尽力淘汰”改成硬配额。首次播放延迟的根本解法是流式播放，而不是更快下载。

### EQ 重建间隙

`fb-audio` 的 worker 收到 `SetEq` 后：`player.stop()` → `clear()` → `open_file()` 重新打开当前文件 →
`append(EqSource::new(..))` → `try_seek(原位置)`。间隙来自重新打开 + 解码 + seek；本地文件通常
几十毫秒，网络盘/大文件更久。UI 已经做了“拖动时只记脏、指针松开才发送”，所以拖滑块期间不会
连续重建。注意：**远程/电台流不会重建**（无法在流上重开并 seek），参数只影响之后打开的本地文件；
中性设置目前也会包一层 `EqSource`，没有旁路。

### 远程缓存

- 位置：`<data>/cache/remote/<server>/<track-hash>.<ext>`，缓存键与服务器、曲目 ID 绑定。
- 策略：整首下载完成后插入内存索引并播放；每次下载完成后按文件 mtime 从旧到新淘汰，上限
  `REMOTE_CACHE_LIMIT = 2 GiB`。这是软上限：正在写入的 `.part` 被跳过，进程中断留下的 `.part`
  不会在启动时清理。
- 代价：缓存命中只需一次 `is_file()`；未命中要等整首下载，下载命令在远程 worker 串行执行，
  期间浏览/搜索请求会排队。缓存淘汰只扫描目录、不读文件，通常远低于一帧，但目录很大时仍值得
  改成持久化索引。

## 复测记录模板

发布前把结果记在这里（或链接到 CI artifact）：

| 日期 | 构建 | 素材（曲目/专辑） | 冷扫描 | snapshot | 搜索 p95 | 滚动 p95 | 启动 | RSS |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 待测 | | | | | | | | |
