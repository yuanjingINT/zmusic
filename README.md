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

`install.sh` 生成的启动器会在每次启动播放器时自动执行 `setup.sh update`，所以装了启动器之后一般不用手动管。

## 配置

全部通过环境变量，都有默认值：

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `NCM_API` | `http://127.0.0.1:3000` | 网易云 API 地址 |
| `NCM_DIR` | `~/Music/ncm` | 下载目录 |
| `NCM_QUALITY` | `exhigh` | 音质：`standard` / `higher` / `exhigh` / `lossless` / `hires` |
| `ZMUSIC_PROXY` | 空 | 可选代理，如 `socks5h://127.0.0.1:11080`；YouTube / SoundCloud 必需，见下 |
| `ZMUSIC_METING_URL` | `http://127.0.0.1:8123` | Meting 侧车地址（腾讯/酷狗/百度/酷我） |

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
| bilibili | `bilibili` | 国内 | 自带 web API 搜索（先取 `buvid3` 过风控），取流借 `yt-dlp`；结果是视频，标题里混切片/MV |
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
| Bandcamp | `yt-dlp` 没有 Bandcamp 的搜索前缀，只能抓网页；而 `bandcamp.com` 本机直连被重置，经海外出口拿到的是 3038 字节的 `Client Challenge` 反爬页（没有 `tralbum` 数据），`yt-dlp -g` 报 `Unable to extract tralbum data` |
| Free Music Archive | 老 API 404，新接口要 key |
| Spotify / Tidal / Qobuz / Apple Music | 官方 API 只给元数据与 30s 试听，`stream_url` 只能返回 `None`，等有账号 token 再谈 |

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

## 快捷键

| 键 | 作用 |
|----|------|
| `h` | 回主菜单 |
| `s` | 搜索 |
| `Tab` | 搜索页：输入栏 ↔ 结果列表 切换焦点 |
| `S` | 搜索页：切换音乐源（按「国内 / 国际 / 公开免费」分组循环） |
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
