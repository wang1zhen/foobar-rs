# WASM 插件 SDK（API v0.1）

foobar-rs 只用 WASM 作为第三方扩展机制：不加载原生 DLL，不提供原版 foobar2000 插件 ABI。本文描述当前已实现的 **实验性 API v0.1**，1.0 前允许破坏性变更。

> 当前边界：manifest 命令/面板/导入器/远程源声明、命令执行、面板正文声明、导入器与远程源宿主 ABI、资源限制和权限校验。远程源接入应用 UI、设置 API、DSP/可视化、网络代理/TLS 策略和插件签名都**尚未实现**，不要在插件里依赖它们。

## 插件目录布局

每个插件一个目录，至少包含：

```text
my-plugin/
├── plugin.json    # manifest，UTF-8 JSON
└── plugin.wasm    # core wasm 模块（开发期也可用 .wat，宿主能解析）
```

`manifest.entry` 是相对插件目录的路径，例如 `plugin.wasm`。入口路径必须是相对路径、不能包含 `..`，canonicalize 后必须仍在插件目录内；符号链接指向目录外会被拒绝。发布包应只带 `.wasm`，`.wat` 仅用于本地开发/示例。

安装位置：`${FOOBAR_RS_DATA_DIR}/plugins/<插件目录>/`。未设置 `FOOBAR_RS_DATA_DIR` 时使用平台数据目录（Linux 为 `~/.local/share/foobar-rs/plugins`）。

## manifest

```json
{
  "id": "com.example.template-rust",
  "name": "Template Rust",
  "version": "0.1.0",
  "api_version": "0.1.0",
  "permissions": ["register_commands", "declare_panels"],
  "entry": "plugin.wasm",
  "commands": [
    { "id": "com.example.template-rust.hello", "title": "Template Hello" },
    { "id": "com.example.template-rust.world", "title": "Template World" }
  ],
  "panels": [
    {
      "id": "com.example.template-rust.panel",
      "title": "Template Panel",
      "text": "Hello from the Rust plugin template.\nUse the buttons to run a command.",
      "commands": [
        "com.example.template-rust.hello",
        "com.example.template-rust.world"
      ]
    }
  ]
}
```

字段与校验：

- `id` 非空；`version`、`api_version` 是 semver 字符串。
- `integrity` 可选：entry 文件字节的 SHA-256，64 位 hex（大小写均可）；格式非法会在加载前拒绝。省略表示跳过完整性校验。
- `commands`、`panels`、`importers`、`sources` 可省略（旧 manifest 仍可加载）；存在时 id/title 不能为空，各自列表内 id 必须唯一。
- `commands` 非空时 permissions 必须包含 `register_commands`；`panels` 非空时必须包含 `declare_panels`；`importers` 非空时必须包含 `read_files`；`sources` 非空时必须包含 `network_access`。
- 面板 `text` 是纯文本：不解释 HTML、Markdown 或脚本，换行按文本渲染。
- 面板 `commands` 引用本 manifest `commands` 中的 id；空引用或未知 id 会被拒绝。
- 导入器 `extensions` 不能为空、不能带 `.` 前缀，解析时规范化为小写。
- 远程源 `allowed_hosts` 不能为空，解析时 trim/转小写/去重；支持精确域名和 `*.example.com` 子域通配，不能带 scheme、端口、路径或 `*` 裸通配。
- `register_commands`/`declare_panels` 仍是声明级校验；`read_files` 用于导入器与 `host.read_file`，`network_access` 用于远程源与 `host.http_get`。元数据读取仍无运行时执行器。

当前权限枚举：`read_metadata`、`register_commands`、`declare_panels`、`network_access`、`read_files`。

### 完整性校验（integrity）

```json
"integrity": "<64 位 sha256 hex>"
```

- 哈希对象是宿主 `load` 收到的 entry 文件字节。发布包中 `entry` 指向 `plugin.wasm`，因此就是对 `.wasm` 文件内容做 SHA-256；开发期若 entry 指向 `.wat`，则对 `.wat` 文本字节计算，而不是编译后的 wasm。
- 大小写 hex 都接受；必须正好 64 位 hex，否则 manifest 校验失败。
- 省略 `integrity` 时跳过校验，兼容旧插件；这是当前默认。填写后会启用校验，若 wasm 被修改则加载失败且不会实例化。
- 计算示例：

```sh
sha256sum "$plugins_dir/plugin.wasm"
# 把输出的 64 位 hex（不含文件名）填进 plugin.json 的 integrity
```

模板 `plugin.json` 故意省略 `integrity`，避免仓库里写死构建产物哈希；发布时再按实际 `.wasm` 填写。注意这不等于签名：没有信任链/密钥校验。

### API 版本兼容规则

宿主实现 `0.1.0`，兼容规则：

- major 必须相同。
- major 为 0 时 minor 必须相同（pre-1.0 的 minor 是破坏边界）；patch 不参与判断，所以 `0.1.x` 都兼容 `0.1.0` 宿主。
- major ≥ 1 时，插件 minor 必须小于等于宿主 minor。
- major 不同直接拒绝。

manifest 在编译 wasm 前校验，版本/权限/声明错误会让插件加载失败，而不是运行期崩溃。

## 命令 ABI

插件是 `wasm32-unknown-unknown` **core wasm** 模块。宿主**不链接 WASI**，导入 `wasi_snapshot_preview1` 会实例化失败。宿主链接的导入只有：

```text
host.ping() -> i32   // 返回 42，仅用于连通性验证
host.read_file(path_ptr: i32, path_len: i32, buf_ptr: i32, buf_cap: i32) -> i32
host.http_get(url_ptr: i32, url_len: i32, buf_ptr: i32, buf_cap: i32) -> i32
```

`host.read_file` 只给声明了 `read_files` 的插件使用；`host.http_get` 只给声明了 `network_access` 且处于某个 source 调用上下文的插件使用。命令本身不需要任何宿主导入。

命令通过四个固定导出访问，无参数结构体、无指针传递：

| 导出 | 签名 | 含义 |
|---|---|---|
| `fb_command_count` | `() -> i32` | 命令数量 |
| `fb_command_name_len` | `(index: i32) -> i32` | 命令名 UTF-8 字节长度；负数表示 guest 错误 |
| `fb_command_name_byte` | `(index: i32, offset: i32) -> i32` | 返回 0..=255 的单字节；负数表示 guest 错误 |
| `fb_command_run` | `(index: i32) -> i32` | 执行命令，返回值由插件定义 |

宿主的约束：

- 命令名最长 4096 字节；负长度、越界字节、非法字节值都会变成宿主错误。
- 命令名必须是严格 UTF-8；宿主不会用 lossy replacement 掩盖坏数据。
- 缺失导出或签名不符返回明确错误。
- 加载后应用会调用 `validate_command_declarations`，要求 `fb_command_count` 与 manifest `commands.len()` 一致。
- fuel 是单个 `Invocation` 生命周期内累计的；应用在每次执行命令前会重置到配置值，插件不能依赖跨命令剩余预算。

## 面板

面板目前是**声明式**的：manifest 提供 `id`、`title`、纯文本 `text` 和可引用命令 id 列表 `commands`。宿主负责布局、主题、字体、无障碍和按钮渲染；插件不能提交 egui/Rust 对象、HTML 或自定义绘制代码。面板 GUI 渲染由宿主 UI 决定，插件只声明内容。

## 导入器 ABI（实验）

导入器声明示例：

```json
"permissions": ["read_files"],
"importers": [
  {
    "id": "com.example.import.flac",
    "title": "FLAC importer",
    "extensions": ["flac", "wav"]
  }
]
```

- `extensions` 在反序列化时 trim 并转小写，不能为空、不能以 `.` 开头；导入器 id 与命令/面板一样不能重复。
- 非空 `importers` 强制要求 `read_files` 权限，否则 manifest 校验失败。
- 当前只是宿主层 ABI spike：应用还没有把导入器接到“导入文件”流程或 UI，也没有 `network_access`/远程源实现。

guest 必须导出：

| 导出 | 签名 | 含义 |
|---|---|---|
| `memory` | linear memory | 输入/输出 JSON 与文件缓冲 |
| `fb_alloc` | `(len: i32) -> i32` | 返回至少 `len` 字节的 guest 指针；负数表示失败 |
| `fb_import` | `(ptr: i32, len: i32) -> i64` | 处理输入 JSON，返回 `(out_ptr << 32) \| out_len` |

宿主调用流程（`Invocation::import(path)`）：

1. 检查 manifest 声明了 importers 且插件有 `read_files`。
2. 序列化 `{"path":"..."}`（UTF-8），上限 1 MiB；调用 `fb_alloc` 取缓冲并写入 guest memory。
3. 调用 `fb_import(ptr, len)`，高 32 位是输出指针、低 32 位是输出长度。
4. 检查输出长度不超过 1 MiB、指针区间在 memory 内，读取并严格校验 UTF-8，再校验是合法 JSON；返回原 JSON 字符串。

文件访问通过宿主函数，不允许 guest 直接读文件：

```text
host.read_file(path_ptr, path_len, buf_ptr, buf_cap) -> i32
```

- 路径从 guest memory 读取，必须是 UTF-8；指针/长度越界返回负错误码。
- 没有 `read_files` 返回 `-1`；文件超过 10 MiB 返回 `-3`；guest 缓冲不够返回 `-4`；路径非法/越界返回 `-2`；打开或读取失败返回 `-5`；成功返回写入字节数。
- 当前没有目录 allowlist：`read_files` 等价于“可读宿主进程有权读取的任意文件”。这是实验性 ABI，接入正式导入流程前需要收紧路径策略。

## 远程源 ABI（实验）

远程源负责把查询转换为曲目列表。manifest 示例：

```json
"permissions": ["network_access"],
"sources": [
  {
    "id": "com.example.source.remote",
    "title": "Example Remote",
    "allowed_hosts": ["api.example.com", "*.example.com"]
  }
]
```

`allowed_hosts` 规则：

- 解析时 trim、转小写、去重、去掉末尾点；列表不能为空。
- 精确项只匹配同名 host；`*.example.com` 只匹配至少一级子域（`a.example.com`、`a.b.example.com`），不匹配 apex `example.com`，也不匹配 `evil-example.com`。
- 不允许 scheme、端口、路径、查询串或裸 `*`；host 只按 URL 的 host 部分匹配，URL 仍必须是 `http`/`https`。

guest 必须导出：

| 导出 | 签名 | 含义 |
|---|---|---|
| `memory` | linear memory | 输入/输出 JSON 与 HTTP 缓冲 |
| `fb_alloc` | `(len: i32) -> i32` | 返回至少 `len` 字节的 guest 指针；负数失败 |
| `fb_source` | `(ptr: i32, len: i32) -> i64` | 处理输入 JSON，返回 `(out_ptr << 32) \| out_len` |

`Invocation::source_list(source_id, query)`：

1. 校验 `manifest.sources` 含该 id、插件有 `network_access`，否则返回 `UnknownSource` / `NetworkAccessDenied`。
2. 把该 source 的 `allowed_hosts` 写入当前 Store；每个 source 调用只允许自己的 allowlist，不能复用上一个 source 的权限。
3. 序列化 `{"source":"...", "query":"..."}`（上限 1 MiB），通过 `fb_alloc` 写入 guest memory，调用 `fb_source`。
4. 校验输出指针/长度（上限 1 MiB、必须在 memory 内）、严格 UTF-8 和合法 JSON，返回原字符串。

输出约定为曲目列表，宿主目前不解析字段，但推荐格式：

```json
{
  "tracks": [
    {
      "title": "Track",
      "artist": "Artist",
      "album": "Album",
      "url": "https://api.example.com/stream/1"
    }
  ]
}
```

### `host.http_get`

```text
host.http_get(url_ptr, url_len, buf_ptr, buf_cap) -> i32
```

- URL 从 guest memory 读取，必须是 UTF-8、`http`/`https`，host 必须命中当前 source 的 allowlist。
- 连接超时 15 秒、总超时 20 秒；响应体上限 1 MiB。
- 禁止自动重定向（3xx 按非 2xx 处理），避免跳到 allowlist 之外的 host；不跟随代理/PAC。
- 2xx 响应完整写入 guest buffer；缓冲不足返回负码，不截断。成功返回写入字节数。
- 错误码：`-1` 无权限/无 source/host 不允许；`-2` URL/UTF-8/guest 区间非法；`-3` 非 2xx；`-4` 响应超 1 MiB；`-5` 请求或读取失败；`-6` guest 缓冲不足。
- 安全边界：只做 host allowlist，不做 DNS/IP 过滤、TLS pinning、内容类型或 JSON 校验；URL 指向私网/回环地址仍受 allowlist 约束，需要宿主侧额外策略时再收紧。
- 当前远程源 ABI 只是宿主层 spike：没有接入应用 UI、远程浏览、缓存或 scrobble。宿主 API 是同步阻塞调用，接入时必须放在插件 worker，不得在 UI/音频线程调用。

## 资源限制与隔离

每次插件调用使用独立 Store：

- fuel：默认 10,000,000，执行每条命令前重置；耗尽即 trap。
- 线性内存：每个 memory 最多 16 MiB；Store 内最多 1 个 memory。
- 表：最多 1 个 table，单表最多 4096 个元素。
- 实例：每个 Store 最多 1 个实例。
- 导入器/远程源：输入/输出 JSON 各最多 1 MiB；`host.read_file` 单文件最多 10 MiB；`host.http_get` 响应最多 1 MiB，连接超时 15 秒、总超时 20 秒；缓冲必须落在 guest memory 内。
- 插件 trap/坏模块/缺导出只让当前调用失败，不影响播放或主界面；目前没有墙钟超时（epoch deadline），只有 fuel 配额。

## Rust 插件模板

仓库内模板：`examples/plugins/template-rust/`。它是独立 Cargo 项目（带空 `[workspace]`，不会被根 workspace 吸收），`crate-type = ["cdylib"]`，无第三方依赖，手写上述命令 ABI。

模板内容：

- `Cargo.toml`：独立 workspace、`cdylib`。
- `src/lib.rs`：静态 ASCII 命令名数组，按 `index`/`offset` 返回字节；`fb_command_run` 返回 index。
- `plugin.json`：2 个命令、1 个带正文和命令引用的面板。

在宿主目标上做类型检查（不需要安装 wasm target）：

```sh
cargo check --manifest-path examples/plugins/template-rust/Cargo.toml
```

构建成插件（需要 wasm target）：

```sh
rustup target add wasm32-unknown-unknown
cargo build --release --target wasm32-unknown-unknown \
  --manifest-path examples/plugins/template-rust/Cargo.toml
```

安装：

```sh
plugins_dir="${FOOBAR_RS_DATA_DIR:-$HOME/.local/share/foobar-rs}/plugins/template-rust"
mkdir -p "$plugins_dir"
cp examples/plugins/template-rust/plugin.json "$plugins_dir/"
cp examples/plugins/template-rust/target/wasm32-unknown-unknown/release/template_rust.wasm \
  "$plugins_dir/plugin.wasm"
```

然后在应用里重新加载插件。也可以参考更小的 `examples/plugins/hello`（直接以 `.wat` 形式加载）。

## 当前未实现（不要依赖）

- 通用文件/网络 capability 执行器与用户授权 UI；目前 `read_files` 仅用于导入器/`host.read_file`，`network_access` 仅用于远程源/`host.http_get`，没有目录 allowlist、DNS/IP 过滤、代理或 TLS 策略。
- 设置 API（settings DTO 存在，但插件设置读写和 UI 未接入）。
- 远程源/导入器接入应用 UI 与播放流程；远程浏览、缓存、scrobble、元数据提供者等扩展点。
- DSP/可视化接入实时音频；见 [`dsp-wasm.md`](dsp-wasm.md)，目前只有可行性结论，不接入音频链。
- 插件签名/信任链、插件市场、启用/禁用生命周期 UI（完整性哈希校验已支持，但它不是签名）。
- 自定义 UI 绘制；面板只能是宿主渲染的声明式内容。
