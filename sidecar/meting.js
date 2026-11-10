// zmuse 的 Meting 侧车：一个 GET /api 就够用。
// 只监听 127.0.0.1，契约与 Meting-API 一致：
//   /api?server=tencent&type=search&id=晴天&limit=10
//   /api?server=tencent&type=url&id=<url_id>&br=320
//   /api?server=tencent&type=lrc&id=<lyric_id>
import http from 'node:http';
import Meting from '@meting/core';

const PORT = Number(process.env.METING_PORT || 8123);
const HOST = '127.0.0.1';
const SERVERS = new Set(['netease', 'tencent', 'kugou', 'baidu', 'kuwo']);

// cookie 只从请求头（Rust 侧的 ZMUSE_METING_COOKIE）或本机环境变量来，
// 绝不写进仓库、绝不打印到日志。
function cookieOf(req) {
  const h = req.headers['x-meting-cookie'];
  if (typeof h === 'string' && h.length < 8192) return h;
  const env = process.env.METING_COOKIE || process.env.ZMUSE_METING_COOKIE;
  return env && env.length < 8192 ? env : '';
}

function client(server, cookie) {
  const m = new Meting(server);
  m.format(true);
  if (cookie) m.cookie(cookie);
  return m;
}

// 各平台「原始」song 接口里时长的位置不同；Meting 的 format() 会把它丢掉，
// 所以这里绕开 format 取一次原始 JSON。取不到就回 0（UI 继续显示 --:--）。
async function durationMs(server, id, cookie) {
  const m = client(server, cookie).format(false);
  let raw;
  try {
    raw = JSON.parse(await m.song(id));
  } catch {
    return 0;
  }
  const num = (v) => (typeof v === 'number' && Number.isFinite(v) ? v : 0);
  // format(false) 之后各平台结构不一样：腾讯是 data[0]，酷狗/酷我是对象本身
  const d = Array.isArray(raw?.data) ? raw.data[0] : (raw?.data ?? raw);
  if (server === 'tencent') return num(d?.interval) * 1000;
  // 酷狗 playInfo 对未授权 hash 的 timeLength 恒为 0（实测），拿不到就回 0
  if (server === 'kugou') return num(d?.timeLength ?? d?.timelength) * 1000;
  if (server === 'netease') return num(d?.dt);
  if (server === 'kuwo') return num(d?.duration) * 1000;
  return 0;
}

async function handle(q, cookie) {
  const server = q.get('server') || 'netease';
  const type = q.get('type') || 'search';
  const id = q.get('id') || '';
  if (!SERVERS.has(server)) throw new Error(`unknown server: ${server}`);
  if (!id) throw new Error('missing id');
  const m = client(server, cookie);
  switch (type) {
    case 'search': {
      const limit = Math.min(Number(q.get('limit') || 10), 30);
      return JSON.parse(await m.search(id, { type: 1, page: 1, limit }));
    }
    case 'url': {
      const br = Number(q.get('br') || 320);
      return JSON.parse(await m.url(id, br));
    }
    case 'lrc':
      return JSON.parse(await m.lyric(id));
    case 'pic':
      return JSON.parse(await m.pic(id, 300));
    case 'duration': {
      const ms = await durationMs(server, id, cookie);
      return { duration_ms: ms };
    }
    default:
      throw new Error(`unknown type: ${type}`);
  }
}

const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, `http://${HOST}:${PORT}`);
  if (url.pathname !== '/api') {
    res.writeHead(404, { 'content-type': 'application/json' });
    return res.end(JSON.stringify({ error: 'not found' }));
  }
  try {
    const out = await handle(url.searchParams, cookieOf(req));
    res.writeHead(200, { 'content-type': 'application/json' });
    res.end(JSON.stringify(out));
  } catch (e) {
    res.writeHead(502, { 'content-type': 'application/json' });
    res.end(JSON.stringify({ error: String(e?.message || e) }));
  }
});

server.listen(PORT, HOST, () => {
  console.log(`meting sidecar on http://${HOST}:${PORT}/api`);
});
