# zmusic

网易云音乐的终端播放器。Rust + [ratatui](https://github.com/ratatui/ratatui) 写的 TUI，键盘操作，在终端里搜歌、听歌、下载。

- 播放走 `mpv`，下载走网易云 API
- 默认参数、收藏、设置都存在 `~/.config/zmusic/`
- 只有一个二进制，没有 Electron，没有浏览器

```
  ⏮  ⏯  ⏭     正在播放：晴天 - 周杰伦
  ────────────────────────────────────────────
  ▍搜索音乐      ← h 回主菜单 / s 搜索 / g 下载
  ▍歌曲推荐
  ▍专辑推荐
```

## 依赖

运行时：

| 依赖 | 用途 | 是否必需 |
|------|------|----------|
| `mpv` | 音频播放 | 必需 |
| `node` >= 22 | 本地网易云 API 服务（`setup.sh` 起 `ncm-api`） | 必需 |

构建时（从源码编译）：

| 依赖 | 用途 |
|------|------|
| `cargo` / `rustc`（stable，edition 2021） | 编译 |
| C 工具链（`gcc` / `cc`） | 链接 |
| `openssl` 头文件 + 库（Debian 系：`libssl-dev`） | `reqwest` 用 `native-tls`，经 `openssl-sys` 链接系统 OpenSSL |
| `pkg-config` | 定位 OpenSSL |

> 为什么是 `native-tls` 而不是 `rustls`：ring 0.17.14 与 gcc 16 / LLVM 21 链接失败，改用系统 OpenSSL。

Linux x86_64 / aarch64。

## 安装

### Arch Linux（自建仓库，公网）

包在本机用 `makepkg` 构建、在 NAS 上 GPG 签名后发布到自建仓库 `repo.dito.asia`，包和数据库都用仓库密钥签名。

> 当前仓库里是 **beta** 版（`0.1.1beta2`）：功能能跑，但还在连着改。beta 不会自动升到 `0.1.1` 正式版，正式版出来会自然盖上去。

一键安装（自动加源、导入并本地信任公钥）：

```bash
curl -fsSL https://repo.dito.asia/install-dito-repo.sh | sudo bash
sudo pacman -S zmusic
```

手动加源：

```ini
[dito-repo]
Server = https://repo.dito.asia/x86_64
SigLevel = Required
```

```bash
sudo pacman-key --add <(curl -fsSL https://repo.dito.asia/x86_64/dito-repo.pub)
sudo pacman-key --lsign-key 6469F0BEB079C3EDCC47947627FEDD0D74F7D714
sudo pacman -Sy
sudo pacman -S zmusic
```

> 仓库密钥指纹 `6469 F0BE B079 C3ED CC47 9476 27FE DD0D 74F7 D714`（`Dito Arch Repo <repo@dito.local>`）。
> 密钥在 2026-11-07 重签过：之前加过源的机器需要重跑一次上面的安装脚本，否则新包验签会失败。
> 内网可直连 NAS 回落：`Server = http://192.168.1.20:8180/x86_64`。

### 预编译包

每次打 `v*` tag，CI 会构建 x86_64 与 aarch64 的 `.deb` / `.rpm` / Arch 包，挂在 [GitLab Releases](https://git.tuxai.org/personal/zmusic/-/releases)（和 [GitHub Releases](https://github.com/yuanjingINT/zmusic/releases) 镜像）上。Arch 用户优先用上面的 `dito-repo`。

### 从源码

主仓库在 GitLab，GitHub 是镜像：

```bash
git clone https://git.tuxai.org/personal/zmusic.git
cd zmusic
cargo build --release --locked
./target/release/zmusic
```

先装构建依赖（缺 OpenSSL 头文件会报 `openssl-sys` / `Could not find openssl`）：

```bash
# Arch
sudo pacman -S --needed base-devel openssl pkg-config
# Debian / Ubuntu
sudo apt install build-essential pkg-config libssl-dev
```

### 一键脚本

```bash
./install.sh    # 编译 release，装启动器到 ~/.local/bin，装桌面启动器
```

## 首次运行

播放器本身不带 API，需要本地起一个网易云 API 服务：

```bash
./setup.sh          # 首次安装并启动服务（systemd --user，服务名 ncm-api）
./setup.sh update   # 更新 API 到最新版并重启
```

> 如果 zmusic 是用 pacman 包安装的，`setup.sh` 会检测到侧车由包提供
> （`pacman -Qo /usr/lib/zmusic/sidecar/meting.js` 成立）并**跳过源码侧车安装**：
> 不再往 `~/.local/share/zmusic/meting` 复制旧副本，也不写用户级 unit（用户级 unit 会遮蔽
> 包里的 `meting-sidecar.service`，导致跑旧代码、缺 `type=duration`）。发现遗留的用户级 unit
> 时只提示怎么备份和重启，不替你删。

`install.sh` 生成的启动器（`~/.local/bin/zmusic` → 仓库里的 `run.sh`）**不会联网更新**：
它只做三件事——二进制比源码旧时增量 `cargo build --release`、`ncm-api` 没跑时
`systemctl --user start ncm-api`、然后 `exec target/release/zmusic`。所以菜单是秒开的。

想更新 API 服务时手动跑（不再挂在启动路径上）：

```bash
./setup.sh update
```

## 配置

所有服务/账号配置收在一个文件：`~/.config/zmusic/config.toml`。生成/编辑：

```bash
zmusic config          # 不存在就生成模板并打开编辑器
zmusic config --print  # 只打印路径
```

完整步骤（ncm-api、Meting 侧车与 cookie、YouTube 代理、Jamendo key、每个源需要什么）
见 [docs/CONFIG.md](docs/CONFIG.md)。

环境变量仍然生效，优先级 **环境变量 > config.toml > 默认值**：

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `NCM_API` | `http://127.0.0.1:3000` | 网易云 API 地址 |
| `NCM_DIR` | `~/Music/ncm` | 下载目录 |
| `NCM_QUALITY` | `exhigh` | 音质：`standard` / `higher` / `exhigh` / `lossless` / `hires` |
| `ZMUSIC_PROXY` | 空 | 可选代理，如 `socks5h://127.0.0.1:11080`；YouTube / SoundCloud 必需，见下 |
| `ZMUSIC_METING_URL` | `http://127.0.0.1:8123` | Meting 侧车地址（腾讯/酷狗/百度/酷我） |
| `ZMUSIC_METING_COOKIE` | 空 | 可选。Meting 换流用的平台 cookie（建议写 config.toml `[meting] cookie`），不落盘、不进日志 |
| `ZMUSIC_YTDLP_BIN` | `yt-dlp` | yt-dlp 可执行文件名或绝对路径 |
| `ZMUSIC_SUPERDL_DIR` | 空 | 设了就让**超级下载**直接写这个目录，跳过选盘/卸载/格式化（无 U 盘时测功能用） |

运行时会写：

```
~/.config/zmusic/favorites.json   收藏
~/.config/zmusic/settings.json    界面与播放设置
```

### 音乐源一览

`S` 键在搜索页按「国内 → 国际 → 公开免费」分组循环，选择记在 `settings.json` 里，默认仍是网易云。
每个源的能力（支持哪些搜索类型、有没有歌词、可不可播放）由 `Source::capabilities()` 给出，
UI 据此禁用不支持的项，不靠猜。

| 源 | `Song.source` | 组 | 状态 |
|----|---------------|----|------|
| 网易云 | `netease` | 国内 | 默认源，搜索/播放/歌词全通 |
| QQ 音乐（直连） | `qq` | 国内 | 免费曲目可出流；会员曲返回「不可播放」 |
| QQ / 酷狗 / 千千 / 酷我（Meting） | `meting:tencent` `meting:kugou` `meting:baidu` `meting:kuwo` | 国内 | 走侧车；tencent/kugou 能搜、取流需 cookie；baidu/kuwo 上游返回空壳 |
| 咪咕音乐 | `migu` | 国内 | app 侧公开接口；搜索不带时长，会员曲返回「不可播放」 |
| YouTube | `youtube` | 国际 | `yt-dlp`，国内需代理 |
| SoundCloud | `soundcloud` | 国际 | `yt-dlp`，国内需代理；搜索较慢（6–7s） |
| Audius | `audius` | 公开免费 | 搜索 + 取流可用 |
| ccMixter | `ccmixter` | 公开免费 | 搜索可用；音乐直链被站点 403，运行期尝试后降级「不可播放」 |
| Wikimedia Commons | `wikimedia` | 公开免费 | 只解析 IPv6，网络慢时降级 |
| Jamendo | `jamendo` | 公开免费 | 需要 `ZMUSIC_JAMENDO_CLIENT_ID`，没配就置灰 |
| OpenGameArt | `opengameart` | 公开免费 | 抓页面，只出单曲 |

没接入的（和原因，都是实测过的）：

| 源 | 原因 |
|----|------|
| bilibili | 曾接入（自带 web API 搜索 + yt-dlp 取流），后按需求移除：结果多是视频切片、标题噪音大，且没有稳定的音频时长；代码与注册表已删 |
| Bandcamp | `yt-dlp` 没有 Bandcamp 的搜索前缀，只能抓网页；而 `bandcamp.com` 本机直连被重置，经海外出口拿到的是 3038 字节的 `Client Challenge` 反爬页（没有 `tralbum` 数据），`yt-dlp -g` 报 `Unable to extract tralbum data` |
| StreetVoice | 搜索 API 通（`/api/v4/search/?q=` 返回 JSON），但播放接口 `POST /api/v4/song/<id>/play/` 恒返回 `{}`（要登录态），网页版本身回 451 地区限制——能搜不能播，先不接 |
| JOOX | `api.joox.com` / `api-jooxtt.sanook.com` 两个域名都回 `code=-2`、`itemlist` 为空（地区/风控），拿不到可用结果 |
| MOOV | `api.moov.hk` 直连超时，没有公开搜索接口 |
| 汽水音乐（抖音） | `api.qishui.com/luna/pc/search/track` 回 200 但 0 字节，要 `a_bogus`/`X-Bogus` 签名；网页端是 SPA 壳，不走逆签名就不碰 |
| Free Music Archive | 老 API 404，新接口要 key |
| Spotify / Tidal / Qobuz / Apple Music | 官方 API 只给元数据与 30s 试听，`stream_url` 只能返回 `None`，等有账号 token 再谈 |

### 源探测与聚合

启动时 zmusic 会在后台并发探测所有源（并发 4、每源一次最小请求），**不进渲染/按键路径**，
所以界面不会等探测。顶栏会显示 `探测中 3/15`，探测完变成 `源 9/15`。

- `R` 键手动重探（同样在后台，界面不卡，大写 R，避免和小写搜索词冲突）
- 搜索页列表区：还没搜索时整块显示**逐行的源状态**（`● 网易云  ok` / `○ YouTube  代理未配置` /
  `⚠ 酷狗(Meting)  仅搜索（无 cookie）`），末尾 `14/16 可用 · R 重探`；
  有结果时状态收进列表标题右侧（`搜索结果   源 14/16`）
- `V` 键在「搜索结果 ↔ 源状态明细」之间切换；`a` 切换 `S` 循环范围（只看可用 / 含离线）
- 探测结果缓存 10 分钟；缺 key（如 Jamendo）和侧车没起标「需凭据」，网络挂了标「离线」，两者不混

**没有时长的不进列表**：搜索/聚合结果里 `dt <= 0` 的曲目直接丢弃（没有进度条、点了才知道
能不能放）。Meting 搜索不带时长，会在源内先并发补前 12 条再过滤；懒补时长补完仍为 0 的同样不显示。

**能搜 ≠ 能播**：`Capabilities::can_stream` 标明一个源能不能出流。聚合只把 `can_stream=true`
的源并进来；没配 cookie 的 Meting、上游只回空壳的源在列表里标「仅搜索」（红色），不塞进播放队列。
单源搜索时这类结果照样能看，点播放会提示「这首来自『仅搜索』的源」。

取不到播放地址时（会员曲、直链过期、源失效）不会停在「标题变了没声音」：后台记下这首、
按播放顺序跳到下一首能播的，顶栏给出 `跳过无法播放: X（源）→ 接着放 Y`；
连续跳 5 首还没播上就停下并提示。随机模式下不会反复撞同一首。

搜索页最前面是 **聚合** 伪源：把关键词并发发给所有可用源（并发 4、每源 5s），
超时的源会写在状态行 `失败源: …`，不影响其它源。聚合只做歌曲搜索，播放仍按每条结果的
`Song.source` 派发。

聚合搜索的执行细节：并发上限 24（这台机器 24 核，纯 I/O 不抢 CPU），会 fork 子进程的源
（`yt-dlp` / `bilibili`）单独限 2 个并发；每个源的 HTTP 调用 6s 超时、建连 2s；
`(源, 关键词)` 结果缓存 60 秒（`R` 重探会清空），改一个字再改回来不会重新打接口。
启动时会预热一次网易云搜索，避免第一次搜索卡在它的上游冷启动上。

排序与代表源按固定优先级（`source_priority`）：

```
netease > qq > meting:tencent > meting:kugou > meting:baidu > meting:kuwo
        > migu > youtube > soundcloud
        > audius > ccmixter > jamendo > wikimedia > opengameart
```

- 注册顺序 = `S` 循环顺序 = 这个优先级，网易云永远第一
- 聚合结果按优先级稳定排序（同源内保持接口返回顺序），网易云的歌排最前
- 同一首（歌名 + 主歌手）只留一行，**代表源取优先级最高的那条**（后到的网易云会顶掉先到的咪咕）；
  源列把所有命中的源按优先级并列显示，如 `网易云+QQ(直连)`

### yt-dlp 源（YouTube / SoundCloud）与代理

YouTube 和 SoundCloud 共用一个 adapter（`src/source/ytdlp.rs`，按 `Site` 参数化），都走 `yt-dlp` 子进程。
**国内直连都不通，需要代理**：

```bash
# 例：借一台海外机器开 SOCKS5
ssh -f -N -D 127.0.0.1:11080 your-vps
ZMUSIC_PROXY=socks5h://127.0.0.1:11080 zmusic
```

关键约束：**zmusic 与 mpv 必须走同一个出口**。googlevideo 的直链把签名绑在出口 IP 上，
换 IP 就 403。具体实现：

- `yt-dlp` 侧直接收 `--proxy`
- mpv 的 `--http-proxy` 只认 HTTP 代理，不认 SOCKS；所以配成 `socks5(h)://` 时，
  zmusic 会在本机起一个只监听 `127.0.0.1` 的小桥（HTTP CONNECT → SOCKS5），
  只给 yt-dlp 系的曲目（`youtube` / `soundcloud`）用；播放其它源时自动清空，回到直连
- 配成 `http://…` 时原样交给 mpv，不起桥

`yt-dlp` 不在 PATH、代理挂了、或超时：该源报错并置灰，其它源不受影响。`yt-dlp` 系源的搜索
结果按 `(源, 关键词)` 缓存 5 分钟，取流地址**不缓存**（会过期）。

冒烟测试（真跑 `yt-dlp`，默认 ignore）：

```bash
ZMUSIC_LIVE_PROXY=socks5h://127.0.0.1:11080 cargo test --release ytdlp_live -- --ignored --nocapture
```

### Meting 侧车（腾讯 / 酷狗 / 百度 / 酷我）

国内这几家用 `sidecar/meting.js`（Node + `@meting/core`）统一承接，只监听 `127.0.0.1:8123`：

```bash
cd sidecar && npm i && node meting.js        # 或在仓库根跑 ./setup.sh，会装成 systemd --user
```

Rust 侧注册成四个源：`QQ 音乐 (Meting)` / `酷狗 (Meting)` / `千千/百度 (Meting)` / `酷我 (Meting)`；
`ZMUSIC_METING_URL` 可改地址（默认 `http://127.0.0.1:8123`）。侧车没起或装不上，这组源整体置灰，
不影响网易云和其它源。QQ 另有直连源（标 `QQ 音乐 (直连)`）作回退。

装了包之后侧车是 `disabled` 的，要开机自启：

```bash
systemctl --user enable --now meting-sidecar.service
```

#### 可选：cookie（会员/受限曲目换流用）

tencent/kugou 的搜索不需要登录，但取流对很多曲目要平台 cookie。zmusic 只接受从**环境变量**
读到的 cookie（`ZMUSIC_METING_COOKIE`，或侧车自己的 `METING_COOKIE`），不会写盘、不会进日志：

```bash
# 临时用（当前 shell）
export ZMUSIC_METING_COOKIE='uin=o0123456; qm_keyst=...'
# 常驻：写进 600 的环境文件，由 systemd 读（unit 里已配 EnvironmentFile）
mkdir -p ~/.config/zmusic && install -m 600 /dev/null ~/.config/zmusic/meting.env
printf 'METING_COOKIE=%s\n' 'uin=...; qm_keyst=...' > ~/.config/zmusic/meting.env
systemctl --user restart meting-sidecar
```

cookie 从浏览器开发者工具里复制（目标站点已登录状态下，任意请求的 `Cookie` 请求头）。
**用自己账号的 cookie，风险自担**：它等价于该账号的登录态，别外借、别提交进仓库。

#### 均衡器（真频谱）

正在播放页的频谱来自**真实音频**：程序会另起一个隐藏的 mpv 进程，用
`--ao=pcm` 把同一首歌解码成 8kHz 单声道 PCM 写到临时 FIFO，再按实时速度消费、
用 Goertzel 算 12 个对数频段的能量（终端够宽 16 段、窄于 80 列 8 段），
按下表映射成 3–5 行高的柱状图：

- 对数映射（dB），低频不会一响就顶满
- attack 0.65 / release 0.12，起得快落得慢
- 峰值保持 0.5 秒后用 `▔` 标出并匀速下落
- 颜色分三档（低/中/高），一眼能看出强弱

**代价**：同一首歌会**再拉一次流**（多一份带宽与解码），所以它只在「正在播放页
且确实在播放」时运行，切页/暂停/停止立刻杀掉解码进程（不留孤儿 mpv）。

配置：

| 配置 | 默认 | 说明 |
|------|------|------|
| `[ui] equalizer` | `true` | 总开关，关掉零重绘 |
| `[ui] equalizer_fps` | `30` | 5–60，播放页的重绘帧率 |
| `[ui] equalizer_source` | `pcm` | `pcm` 真频谱；`synth` 不拉流的慢呼吸示意波形（省流量用） |

解码失败连续两次会降级到 `synth` 并在状态栏提示一次。

#### 超级下载

超级下载（菜单「设备」→ 选可移动盘）会格式化目标盘再往里灌热榜歌曲。取流失败的处理：
`/song/url/v1` 返回的 `code!=200` 或拿不到可播流时（会员曲、无版权、第三方兜底死链），
**跳过该曲并继续下一首**，不会写盘、也不会留下 `.part` 文件；失败的曲目在日志里标
「跳过(无播放链接)」。设 `ZMUSIC_SUPERDL_DIR=<目录>` 可跳过格式化直接写入该目录，
用于没有 U 盘时验证这条路径。

#### 时长

Meting 的搜索接口不给时长，列表里先显示 `--:--`；选中或播放时补问一次（`type=duration`），
补到了就刷新。腾讯能补（实测 `晴天` → 269s），酷狗的 `playInfo` 对未授权 hash 恒返回
`timeLength=0`，这类就一直是 `--:--`。

## 快捷键

| 键 | 作用 |
|----|------|
| `0`–`6` | 主菜单直达对应项（菜单编号从 0 开始） |
| `h` | 回主菜单 |
| `s` | 搜索 |
| `Tab` | 搜索页：输入栏 ↔ 结果列表 切换焦点 |
| `S` | 搜索页：在**可用**源之间循环（第一个是「聚合」；`V` 打开全部） |
| `R` | 重新探测所有源（后台，不卡界面） |
| `V` | 搜索页：源列表 只列可用 / 列全部 切换 |
| `F1`–`F4` / `Alt+0`–`3` | 搜索页：切搜索类型（歌曲 / 歌手 / 歌单 / 专辑），两种焦点下都可用 |
| `g` | 下载列表 |
| `v` | 正在播放界面 |
| `Enter` | 播放选中项 |
| `Space` | 播放 / 暂停 |
| `n` / `p` | 下一首 / 上一首 |
| `j` / `k`、`↑` / `↓` | 上下移动 |
| `d` | 下载选中项 |
| `D` | 批量下载 |
| `l` | 收藏 / 取消收藏 |
| `x` | 从收藏移除 |
| `A` | 打开所选歌曲的专辑 |
| `t` | 收藏页切换视图 |
| `-` / `=` | 音量 −5 / +5 |
| `0` `1` `2` `3` | 菜单里的快捷选择；搜索页列表聚焦时切类型（输入栏聚焦时是普通输入） |
| `Esc` | 关闭弹层；否则回上一页（逐层后退，已在主菜单时不动） |
| `q` | 退出 |

## 项目结构

```
src/
  api.rs        网易云 API 客户端
  config.rs     环境变量与路径
  downloader.rs 下载（含批量）
  favorites.rs  收藏读写
  models.rs     数据结构
  player.rs     mpv 控制（Unix socket）
  superdl.rs    批量下载队列
  settings.rs   设置持久化
  ui/           ratatui 界面（screens / theme）
  util.rs       杂项
PKGBUILD        Arch 打包
packaging/      nfpm 打包配置（deb / rpm / Arch）
.github/workflows/build.yml   CI：多架构构建 + 打包 + 发 Release
```

## 构建与打包

Arch 仓库里的包是本机构建、NAS 签名的，不走 CI：

```bash
makepkg -s           # 需要 base-devel + openssl（PKGBUILD 已声明）
# 产物 zmusic-<ver>-<rel>-x86_64.pkg.tar.zst，签名入库由发布脚本处理
```

发版两条硬规矩：

- **`pkgrel` 必须递增**。版本号完全相同时 pacman 不会升级，只提示“重装”，已经装过旧包的人永远拿不到新包。
- `PKGBUILD` 的 `_commit` 要指向 GitLab 上**已推送**的 commit，并同步重算 `sha256sums`；本地没推的 commit 归档会 404。

CI（`.github/workflows/build.yml`）只产出给非 Arch 用户的便携包，push / PR / tag 都跑：

1. `x86_64`（ubuntu-latest）和 `aarch64`（ubuntu-24.04-arm）两个架构各编译一次
2. 装 `libssl-dev` + `pkg-config`（`native-tls` 需要），`cargo build --release --locked`
3. 用 [nfpm](https://github.com/goreleaser/nfpm) 打成 `.deb` / `.rpm` / Arch 包；`nfpm.yaml` 里 `${GOARCH}` / `${VERSION}` 由 workflow 显式注入，不靠默认值
4. 打 tag 时自动挂到 GitHub Release（GitLab Release 用 tag + API 建，正文放 sha256 / 指纹 / 仓库地址）

## 版本记录

| 版本 | 变更 |
|------|------|
| 0.1.1beta2 | 修自动连播只切歌不出声：keep-open 下 mpv 播完停在暂停态，自动连播现在显式解暂停；手动切歌与切音质仍保持原暂停状态 |
| 0.1.1beta1 | 底部提示按页面显示；`i` 键当前歌曲弹窗（收藏/下载/音质/播放顺序）；播完自动下一首（顺序/随机/单曲循环）；下载音质跟随运行期设置 |
| 0.1.0-2 | 许可证改 GPL-3.0，依赖修正（`openssl` 必需、`mpv`/`nodejs` 可选），TLS 换 native-tls |
| 0.1.0-1 | 首个公开版本 |

## 许可证

[GPL-3.0](LICENSE)
