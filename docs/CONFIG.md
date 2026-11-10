# zmuse 配置指南

zmuse 的所有服务/账号配置都在一个文件里：

```
~/.config/zmuse/config.toml
```

生成并打开它：

```bash
zmuse config          # 不存在就从内置模板生成，然后用 $EDITOR/$VISUAL/vi 打开
zmuse config --print  # 只打印路径
```

优先级：**环境变量 > config.toml > 内置默认值**。老的环境变量（`NCM_API`、`NCM_DIR`、
`NCM_QUALITY`、`ZMUSE_PROXY`、`ZMUSE_METING_URL`、`ZMUSE_METING_COOKIE`、
`ZMUSE_JAMENDO_CLIENT_ID`、`ZMUSE_YTDLP_BIN`）都还生效，适合临时覆盖。

UI 偏好（播放顺序、上次选的源、下载位置）存在 `~/.config/zmuse/settings.json`，
不在 config.toml 里，避免同一个值两处能改。

**从 zmusic 改名过来**：第一次跑新版时，如果 `~/.config/zmuse` 不存在而
`~/.config/zmusic` 在，会把旧目录**复制**一份过来（旧目录保留不删）；缓存
`~/.cache/zmusic` → `~/.cache/zmuse` 同样处理。环境变量 `ZMUSE_*` 优先，没写时回退旧的
`ZMUSIC_*`（`NCM_*` 那几个没改名）。Arch 包里带 `replaces=(zmusic)`/`conflicts=(zmusic)`，
`pacman -Syu` 会自动换掉旧包。

## 启用了哪些源（`[sources] enabled`）

**默认只有网易云。** 其它源都编译在二进制里，但默认不注册、不探测、不出现在源列表里，
也不弹「侧车未启动」这类提示——想要哪个自己打开：

```bash
zmuse source list          # 当前启用的
zmuse source list --all    # 全部内置源 + 是否启用
zmuse source add meting    # 一次开一组（meting = QQ/酷狗/百度/酷我）
zmuse source add youtube soundcloud
zmuse source remove migu
```

`add`/`remove` 直接写回 `config.toml`：

```toml
[sources]
enabled = ["netease", "youtube"]
```

也可以直接在 `zmuse config` 里改：第一行「源 · 已启用」按 Enter 打开勾选列表，
`j/k` 移动、空格启用/停用、`s` 保存。

写回后立即生效；已经在跑的播放器按一下 `R`（重新探测）就会按新列表重建，不用重启。
`meting` 是分组别名（展开成四个 meting 源），`all` 是全部；id 写错会报错并列出可用的 id。
停用网易云是允许的，但会提示「你没有默认源了」。

可用的 id：

| id | 源 | 说明 |
|----|----|------|
| `netease` | 网易云 | 默认启用，需要本地 ncm-api |
| `qq` | QQ 音乐（直连） | 免费曲目可播 |
| `meting:tencent` / `meting:kugou` / `meting:baidu` / `meting:kuwo` | Meting 四家 | 需要 Meting 侧车 |
| `migu` | 咪咕 | 搜索不带时长 |
| `youtube` / `soundcloud` | yt-dlp 系 | 需要 `yt-dlp` 和代理 |
| `audius` / `ccmixter` / `jamendo` / `wikimedia` / `opengameart` | 公开免费源 | Jamendo 需要 client_id |

## 源 → 需要什么

| 源 | 默认可用 | 需要什么 |
|----|----------|----------|
| 网易云 | 是 | 本地 ncm-api（`[netease] api_base`，默认 `127.0.0.1:3000`） |
| QQ 音乐（直连） | 是 | 无；免费曲目可播，会员曲提示不可播放 |
| QQ / 酷狗 / 千千 / 酷我（Meting） | 侧车起来就能搜 | `meting-sidecar` 服务；要播受限曲目还要 `[meting] cookie` |
| 咪咕 | 是 | 无（公开接口） |
| YouTube / SoundCloud | 装了 yt-dlp 就能搜 | `[ytdlp] binary` + **代理**（国内直连不通） |
| Audius | 是 | 无 |
| ccMixter | 是 | 无；音乐直链常被站点 403，运行期失败就降级 |
| Jamendo | 否 | `[jamendo] client_id`（免费申请） |
| Wikimedia Commons | 是 | 只解析 IPv6，网络没有 IPv6 时降级 |
| OpenGameArt | 是 | 无（抓页面） |

说明：bilibili 曾接入过（搜索走它的 web API、取流借 `yt-dlp`），后按需求移除——结果多是
视频切片、时长不稳定，详见 README 的「没接入」表。

「能搜不能播」的源（例如没配 cookie 的 Meting）在列表里可见，但标「仅搜索」，
聚合搜索不会把它们的结果塞进播放队列。

## 1. 网易云（必须）

播放器本身不带网易云 API，需要一个本地服务：

```bash
cd ~/Projects/zmuse      # 或解包目录
./setup.sh                # 首次：装依赖 + 起 systemd --user 服务 ncm-api
./setup.sh update         # 之后：更新 API 并重启
```

要求 Node ≥ 22。验证：

```bash
curl -s 'http://127.0.0.1:3000/search?keywords=%E6%99%B4%E5%A4%A9&limit=1' | head -c 120
```

换地址就改 `config.toml`：

```toml
[netease]
api_base = "http://127.0.0.1:3000"
```

## 2. Meting 侧车（腾讯/酷狗/千千/酷我）

装包后侧车是 disabled 的，先启用：

```bash
systemctl --user enable --now meting-sidecar
systemctl --user status meting-sidecar --no-pager   # 应为 active
curl -s '127.0.0.1:8123/api?server=tencent&type=search&id=%E6%99%B4%E5%A4%A9&limit=1'
```

从源码跑的话在 `sidecar/` 里 `npm i && node meting.js`（或 `./setup.sh`）。

地址可改：

```toml
[meting]
url = "http://127.0.0.1:8123"
```

> 注意：Meting 搜索接口不给时长，zmuse 会在源内补一次 `type=duration` 再过滤掉没有时长的行。
> 侧车必须和 zmuse 同版本（包里的 `meting-sidecar.service` 会随包更新）；用旧侧车会出现
> 「Meting 源搜不到东西」——这时先 `systemctl --user restart meting-sidecar`，确认
> `curl '127.0.0.1:8123/api?server=tencent&type=duration&id=0039MnYb0qxYhV'` 返回 `duration_ms`。

### 可选：cookie（受限曲目换流）

不配 cookie 时 tencent/kugou **能搜不能播**（列表标「仅搜索」）。要播就去浏览器里拿 cookie：

1. 打开目标站点并登录（如 `y.qq.com`）
2. F12 → Network → 随便点一个请求 → Request Headers → 复制 `Cookie:` 整行的值
3. 写进配置：

```toml
[meting]
cookie = "uin=o0123456; qm_keyst=...; ..."
```

也可以用环境变量临时覆盖：`export ZMUSE_METING_COOKIE='uin=...; qm_keyst=...'`。

cookie 等价于该账号登录态：只放在本机 600 权限的文件/环境变量里，别提交进仓库、别外借。
看到「仅搜索」还以为是 bug 的话，先检查这一项。

## 3. YouTube / SoundCloud（需要代理）

国内直连这两家都不通，需要一台海外出口：

```bash
# 例：借 VPS 开 SOCKS5
ssh -f -N -D 127.0.0.1:1080 your-vps
```

```toml
[ytdlp]
binary = "yt-dlp"                       # 或绝对路径
proxy  = "socks5h://127.0.0.1:1080"
```

关键约束：**zmuse 与 mpv 必须走同一个出口**。YouTube 的 googlevideo 直链把签名绑在出口 IP 上，
换 IP 就 403。mpv 只认 HTTP 代理，所以配 `socks5(h)://` 时 zmuse 会在本机起一个只监听
`127.0.0.1` 的 HTTP CONNECT → SOCKS5 小桥，只给 yt-dlp 系曲目用；其它源自动回直连。

验证：

```bash
yt-dlp --proxy socks5h://127.0.0.1:1080 "ytsearch1:daft punk get lucky" --flat-playlist --dump-json
yt-dlp --proxy socks5h://127.0.0.1:1080 -g -f bestaudio/best <上一步的 URL>
```

## 4. Jamendo（免费 key）

1. 打开 <https://devportal.jamendo.com/> 注册一个应用
2. 复制 Client ID
3. 写进配置：

```toml
[jamendo]
client_id = "你的 client_id"
```

没配时 Jamendo 源在列表里显示「需凭据」，不影响其它源。

## 5. 界面与波形（`[ui]`）

播放页那条对称波形和动效都是可调的，改 `~/.config/zmuse/config.toml` 的 `[ui]` 段：

| 配置 | 默认 | 说明 |
|------|------|------|
| `animations` | `true` | 动效总开关，关掉整屏静止 |
| `spinner` / `equalizer` / `smooth_progress` / `fade` | `true` | 各自单独的开关 |
| `equalizer_fps` | `30` | 波形重绘帧率，5–60；嫌终端写入多就调低 |
| `equalizer_source` | `pcm` | `pcm` 真解码；`synth` 不额外拉流 |
| `eq_reference_dbfs` | `-6` | 波形**满高**对应的电平，越小越容易顶满 |
| `eq_autogain` | `true` | **逐曲自动增益**：整首歌内一个增益值，第二次听起按整曲标定（零额外流量） |
| `eq_gain` | `1.6` | 首次播放（还没有标定）时用的固定增益倍率；标定到位后由标定说了算 |
| `eq_soft_clip` | `true` | 软限幅（tanh）：响的地方挤向顶部但不硬切，平顶明显更少 |
| `eq_agc` | `true` | 旧的多段频谱标定，波形不走它，留着给以后换样式 |

### 逐曲自动增益怎么工作

**一首歌从头到尾一个增益值，歌内绝不改**（不会忽大忽小），也**不多下一份音频**：

1. **第一次听**：还不知道这首歌有多响，用固定增益（`eq_gain`，默认 1.6）整首播完。
   同时用波形那条本来就在跑的 1x 解码流，顺手把每个 32ms 窗的峰值攒起来。
2. **播到 80%**：算出整曲 p90（每窗峰值的 90 分位），按「p90 落到 6.5/9 档」反解增益，
   钳在 **−12 ~ +6 dB**，写进 `~/.cache/zmuse/eq_gain.json`（key 是 `源|曲目id`）。
3. **第二次起**：第一秒就按这个标定显示，整首恒定。
4. 拖动/暂停不重新估；换歌才算新的一首。

第一遍的代价是「稳定但可能不够满」（热母带会被削得比较狠），换来的是**不晃、不烧流量**；
第二遍起「又准又稳」。想立刻回到固定观感就关掉 `eq_autogain`。

**实测（2026-11-10，晴天 4:30 / 周旋 4:51）**

| | 增益 | 整首极差 | 响段 p50 | 安静段 p50 | 平顶 |
|---|---|---|---|---|---|
| 晴天 第一遍（固定 1.6） | 恒定 | 0.00 dB | 8.96 | 2.28 | 40.0% |
| 晴天 第二遍（标定 +2.7） | 恒定 | 0.00 dB | 6.27 ✓ | 0.00 ✓ | 0.1% ✓ |
| 周旋 第一遍（固定 1.6） | 恒定 | 0.00 dB | 8.99 | 7.91 | 84.2% |
| 周旋 第二遍（标定 −6.6） | 恒定 | 0.00 dB | 6.00 ✓ | 0.00 ✓ | 0.0% ✓ |

两遍的音频下载都只有 2 路（mpv 播放器 + 波形解码），没有第三路。

**只想试观感、不想改文件**：下面这些都能用环境变量临时盖过去，退出就恢复。

```bash
# 把波形整体抬高一点
ZMUSE_EQ_GAIN=1.9 zmuse

# 关掉逐曲自动增益（省掉那一次额外下载），只用固定增益
ZMUSE_EQ_AUTOGAIN=0 zmuse

# 关掉软限幅，回到硬切
ZMUSE_EQ_SOFT_CLIP=0 zmuse

# 帧率调到 15，终端写入减半
ZMUSE_EQ_FPS=15 zmuse
```

参考量（晴天原唱，副歌最响 10 秒，点高按「中线上下各几点」算，满 9）：

| 配置 | 点高 p50 | 点高 p90 | 平顶占比 |
|------|---------|---------|---------|
| `eq_gain=1.6`（默认） | 5/9 | 9/9 | 20.8% |
| `eq_gain=1.6` + 软限幅 | 5/9 | 7/9 | 0% |
| `eq_gain=2.2` + 软限幅 | 6/9 | 8/9 | 0% |

### 波形调试 CSV（`ZMUSE_WAVE_DUMP`）

`ZMUSE_WAVE_DUMP=/tmp/x.csv zmuse` 会把每个分析窗（32ms）写成一行，8 列：

| 列 | 含义 |
|----|------|
| 1 | 时间戳（秒，从解码器启动算） |
| 2 | 该窗原始峰值（dBFS） |
| 3 | 归一化后的高度（0–9 连续值，9 = 满格）= `softclip(x_pre) × 9` |
| 4 | 逐曲自动增益（dB）：首次播放是 0（还没标定），第二遍起是缓存里的标定值 |
| 5 | 当前生效口径下的 p90（dBFS） |
| 6 | 段号：第几次启动解码器（正常一首歌应该整段是 1） |
| 7 | 平顶标志：1 = 该窗归一化前已经顶到/超过天花板（`x_pre ≥ 0.995`） |
| 8 | **生效增益（dB）**：第 3 列真正用的那个值（首次播放 = 固定增益折合的 dB，默认 ≈ +10.1） |

**第 3 列怎么算**（口径统一，可以拿 2、8 两列复核）：

```
x = clamp((col2 + col8 + 24) / 24, 0, 1)          # 有标定时
x = clamp((col2 - eq_reference_dbfs + 24) / 24, 0, 1) × eq_gain   # 首次播放（固定映射）
col3 = softclip(x) × 9      # 拐点 0.75 以上走 tanh
col7 = 1 ⇔ x ≥ 0.995
```

> 旧版（≤ `7d23e75`）用的是「相对 −12 dBFS 的线性幅度 × 倍率」，和这里的 24 dB 窗口
> 不是同一套，所以那批 CSV 的 level 用 `raw+gain` 推不出来——不是数据错，是公式不同。

第 1 秒内那一两行是流启动瞬态（原始 dB 接近 0、会被标成平顶），算统计时排除前 1 秒。

## 6. 排查

- `zmuse config --print` 看的是不是你以为的那个文件
- 顶栏 `源 9/15` 表示 15 个源里 9 个可用；按 `R` 重新探测
- 缺 key/侧车没起标「需凭据」，网络挂了标「离线」，两者不要混
- 单个源搜不到，先用 `curl` 直接打接口确认服务在不在
