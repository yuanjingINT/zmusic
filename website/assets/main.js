// Small enhancements: copy-to-clipboard on code blocks, smooth scroll already via CSS.
document.querySelectorAll('.code').forEach((el) => {
  el.style.cursor = 'pointer';
  el.title = '点击复制';
  el.addEventListener('click', () => {
    navigator.clipboard.writeText(el.innerText).then(() => {
      const prev = el.dataset.prev || '';
      el.dataset.prev = el.innerText;
      const toast = document.createElement('div');
      toast.textContent = '已复制';
      toast.style.cssText = 'position:fixed;bottom:24px;left:50%;transform:translateX(-50%);background:#0a0a0a;color:#fff;padding:8px 16px;font-size:13px;z-index:999;';
      document.body.appendChild(toast);
      setTimeout(() => toast.remove(), 1200);
    }).catch(() => {});
  });
});
