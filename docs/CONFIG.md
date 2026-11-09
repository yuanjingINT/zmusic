# zmusic 配置指南

zmusic 的所有服务/账号配置都在一个文件里：

```
~/.config/zmusic/config.toml
```

生成并打开它：

```bash
zmusic config          # 不存在就从内置模板生成，然后用 $EDITOR/$VISUAL/vi 打开
zmusic config --print  # 只打印路径
```

优先级：**环境变量 > config.toml > 内置默认值**。老的环境变量（`NCM_API`、`NCM_DIR`、
`NCM_QUALITY`、`ZMUSIC_PROXY`、`ZMUSIC_METING_URL`、`ZMUSIC_METING_COOKIE`、
`ZMUSIC_JAMENDO_CLIENT_ID`、`ZMUSIC_YTDLP_BIN`）都还生效，适合临时覆盖。

UI 偏好（播放顺序、上次选的源、下载位置）存在 `~/.config/zmusic/settings.json`，
不在 config.toml 里，避免同一个值两处能改。

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
cd ~/Projects/zmusic      # 或解包目录
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

> 注意：Meting 搜索接口不给时长，zmusic 会在源内补一次 `type=duration` 再过滤掉没有时长的行。
> 侧车必须和 zmusic 同版本（包里的 `meting-sidecar.service` 会随包更新）；用旧侧车会出现
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

也可以用环境变量临时覆盖：`export ZMUSIC_METING_COOKIE='uin=...; qm_keyst=...'`。

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

关键约束：**zmusic 与 mpv 必须走同一个出口**。YouTube 的 googlevideo 直链把签名绑在出口 IP 上，
换 IP 就 403。mpv 只认 HTTP 代理，所以配 `socks5(h)://` 时 zmusic 会在本机起一个只监听
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

播放页那条对称波形和动效都是可调的，改 `~/.config/zmusic/config.toml` 的 `[ui]` 段：

| 配置 | 默认 | 说明 |
|------|------|------|
| `animations` | `true` | 动效总开关，关掉整屏静止 |
| `spinner` / `equalizer` / `smooth_progress` / `fade` | `true` | 各自单独的开关 |
| `equalizer_fps` | `30` | 波形重绘帧率，5–60；嫌终端写入多就调低 |
| `equalizer_source` | `pcm` | `pcm` 真解码；`synth` 不额外拉流 |
| `eq_reference_dbfs` | `-6` | 波形**满高**对应的电平，越小越容易顶满 |
| `eq_autogain` | `false` | **实验特性**（见下）：边播边估整曲响度、自动定标；首听要 30–60 秒收敛 |
| `eq_gain` | `1.6` | 主增益（自动增益关掉时由它定观感），自动增益开着时是**叠在上面的倍率**（中性写 `1.0`） |
| `eq_soft_clip` | `true` | 软限幅（tanh）：响的地方挤向顶部但不硬切，平顶明显更少 |
| `eq_agc` | `true` | 旧的多段频谱标定，波形不走它，留着给以后换样式 |

**自动增益（实验特性，默认关）**：复用波形那条已经在解码的流，把每个 32ms 窗的峰值
攒起来，取**整曲累计**的 p90 当「这首歌的响度代表」，反解出增益（目标：p90 落在 6/9 档，
留 3 档给歌内动态）。p90 用**对称双速率**平滑——上行下行一样快，不是「只升不降」的棘轮：
和当前估计差 3 dB 以上时按 **0.6 dB/s** 纠正，缩到 1 dB 以内按 **0.3 dB/s** 精修，
中间线性过渡。歌曲的 p90 按 `源|曲目id` 缓存，第二次听同一首第一秒就准。
播放页歌名下面会显示 `自动 +4.1 dB`，表示这首歌被自动调了多少，遇到异常歌一眼能看出来。

增益有两道护栏：**硬钳位 `[-12, +6] dB`**（热母带压得下去，冷母带提得起来），以及
**按播放时长放开的置信度爬坡**——前 15 秒最多 `±2 dB`、45 秒前 `±4 dB`，
之后才放到钳位区间，免得拿前十秒的估计把整首推跑。

### 为什么它默认是关的

参考量（2026-11-09 四首实测，145 秒、无缓存 = 首听）：

| 曲子 | 响段 p50 | 安静段 p50 | 平顶 | gain 极差 | 末 30s 变化 | 缓存命中后 |
|------|---------|-----------|------|----------|------------|-----------|
| 晴天（偏弱） | 6.44 ✓ | 0.00 ✓ | 0.2% ✓ | 4.8 ✓ | 0.7 ✗ | 恒定 |
| 稻香位（偏响、曲内爬升） | 6.72 ✓ | 1.86 ✓ | 0.1% ✓ | 7.1 ✗ | 1.7 ✗ | 恒定 |
| 周旋（极热，需要 −8 dB） | 6.14 ✓ | 0.68 ✓ | 0.2% ✓ | 12.0 ✗ | 0.6 ✗ | 恒定 |
| 风居住的街道（冷母带钢琴） | 6.01 ✓ | 0.45 ✓ | 0.0% ✓ | 4.0 ✓ | 0.3 ✓ | 恒定 |

问题不在平滑速度，在**因果性**：整曲 p90 只有把歌听完才知道，首听时它一路往上爬
（极热歌要从 +2 走到 −8），所以「一首歌里别晃」和「副歌一进来就到位」在不算整首的
前提下没法同时满足。缓存命中时增益**从第一秒就是常数**（实测极差 0.00 dB、末 30 秒 0.00）

- 首听：头 30–60 秒慢慢调到位，这段时间波形会整体升/降一次
- 再听同一首：第一秒就准，全程不动

想要它就写上 `eq_autogain = true`（或 `ZMUSIC_EQ_AUTOGAIN=1` 临时试）；只想稳稳的固定观感，
就把 `eq_gain` 调到顺眼为止（默认 `1.6`）。

**只想试观感、不想改文件**：下面这些都能用环境变量临时盖过去，退出就恢复。

```bash
# 把波形整体抬高一点
ZMUSIC_EQ_GAIN=1.9 zmusic

# 试一下逐曲自动增益（开着时 eq_gain 记得写 1.0，否则会叠在自动增益之上）
ZMUSIC_EQ_AUTOGAIN=1 ZMUSIC_EQ_GAIN=1.0 zmusic

# 关掉软限幅，回到硬切
ZMUSIC_EQ_SOFT_CLIP=0 zmusic

# 帧率调到 15，终端写入减半
ZMUSIC_EQ_FPS=15 zmusic
```

参考量（晴天原唱，副歌最响 10 秒，点高按「中线上下各几点」算，满 9）：

| 配置 | 点高 p50 | 点高 p90 | 平顶占比 |
|------|---------|---------|---------|
| `eq_gain=1.6`（默认） | 5/9 | 9/9 | 20.8% |
| `eq_gain=1.6` + 软限幅 | 5/9 | 7/9 | 0% |
| `eq_gain=2.2` + 软限幅 | 6/9 | 8/9 | 0% |

### 波形调试 CSV（`ZMUSIC_WAVE_DUMP`）

`ZMUSIC_WAVE_DUMP=/tmp/x.csv zmusic` 会把每个分析窗（32ms）写成一行，7 列：

| 列 | 含义 |
|----|------|
| 1 | 时间戳（秒，从解码器启动算） |
| 2 | 该窗原始峰值（dBFS） |
| 3 | 归一化后的高度（0–9 连续值，9 = 满格） |
| 4 | 当前逐曲自动增益（dB，相对中性基准 −12 dBFS） |
| 5 | 当前估计的 p90（dBFS；还没估出来时用基准反推） |
| 6 | 段号：第几次启动解码器（正常一首歌应该整段是 1） |
| 7 | 平顶标志：1 = 该窗顶到/超过天花板（算平顶率用） |

第 1 秒内那一两行是流启动瞬态（原始 dB 接近 0、会被标成平顶），算统计时排除前 1 秒。

## 6. 排查

- `zmusic config --print` 看的是不是你以为的那个文件
- 顶栏 `源 9/15` 表示 15 个源里 9 个可用；按 `R` 重新探测
- 缺 key/侧车没起标「需凭据」，网络挂了标「离线」，两者不要混
- 单个源搜不到，先用 `curl` 直接打接口确认服务在不在
