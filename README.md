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
| `node` >= 22 | 本地网易云 API 服务（`setup.sh` 用） | 必需 |
| `cargo` / `rustc` | 从源码构建时 | 仅构建时 |

Linux x86_64 / aarch64。

## 安装

### Arch Linux（自建仓库）

包由 CI 构建，放进自建的 pacman 仓库（`dito-repo`）。在 `/etc/pacman.conf` 里加：

```ini
[dito-repo]
Server = http://192.168.1.20:8180/x86_64
SigLevel = Required
```

然后：

```bash
sudo pacman -Sy
sudo pacman -S zmusic
```

> `dito-repo` 是内网自建仓库（NAS 上，局域网地址），外网访问不到；换网络时把 `Server` 换成对应的可达地址即可。仓库里的包带签名（`SigLevel = Required`）。

### 预编译包

每次打 tag，CI 会构建 x86_64 与 aarch64 的 `.deb` / `.rpm` / Arch 包，挂在 [Releases](https://github.com/yuanjingINT/zmusic/releases) 上。

### 从源码

```bash
git clone https://github.com/yuanjingINT/zmusic.git
cd zmusic
cargo build --release
./target/release/zmusic
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

运行时会写：

```
~/.config/zmusic/favorites.json   收藏
~/.config/zmusic/settings.json    界面与播放设置
```

## 快捷键

| 键 | 作用 |
|----|------|
| `h` | 回主菜单 |
| `s` | 搜索（`Tab` 切换搜索类型） |
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
| `1` `2` `3` | 菜单里的快捷选择 |
| `Esc` | 关闭弹层 / 取消 |
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

CI（`.github/workflows/build.yml`）在 push 和 tag 时跑：

1. `x86_64`（ubuntu-latest）和 `aarch64`（ubuntu-24.04-arm）两个架构各编译一次
2. 用 [nfpm](https://github.com/goreleaser/nfpm) 打成 `.deb` / `.rpm` / Arch 包
3. 打 tag 时自动挂到 Release

本地打 Arch 包：

```bash
makepkg -si          # 需要 base-devel
```

## 许可证

[GPL-3.0](LICENSE)
