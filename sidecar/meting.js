// zmusic 的 Meting 侧车：一个 GET /api 就够用。
// 只监听 127.0.0.1，契约与 Meting-API 一致：
//   /api?server=tencent&type=search&id=晴天&limit=10
//   /api?server=tencent&type=url&id=<url_id>&br=320
//   /api?server=tencent&type=lrc&id=<lyric_id>
import http from 'node:http';
import Meting from '@meting/core';

const PORT = Number(process.env.METING_PORT || 8123);
const HOST = '127.0.0.1';
const SERVERS = new Set(['netease', 'tencent', 'kugou', 'baidu', 'kuwo']);

function client(server) {
  const m = new Meting(server);
  m.format(true);
  return m;
}

async function handle(q) {
  const server = q.get('server') || 'netease';
  const type = q.get('type') || 'search';
  const id = q.get('id') || '';
  if (!SERVERS.has(server)) throw new Error(`unknown server: ${server}`);
  if (!id) throw new Error('missing id');
  const m = client(server);
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
    const out = await handle(url.searchParams);
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
