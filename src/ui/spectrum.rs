//! 频谱可视化的中间层：dB → 高度 → 点阵。
//!
//! 这一层只做纯计算，不碰音频、不碰 ratatui：
//!
//! 1. `HeightCfg::height`：单个 dB → 0..=7 的连续高度（**不量化**，量化留给渲染器）。
//!    换映射方式（固定刻度 / AGC / 分位数）只改这里，渲染层不用动。
//! 2. `spread`：把 n 段高度铺到屏幕列（柱子模式 / 曲线模式）。
//! 3. `DotGrid`：braille 点阵（每字符 2×4 子点，纵向分辨率是方块字符的 4 倍）。
//!    `fill` 填实心底、`stroke` 走折线，两种都吃同一份列高度。
//!
//! 为什么单独拆出来：终端里方块柱受字形限制，一根柱子最多 8 档；点阵/曲线
//! 用 braille 一个字符就有 2×4 子点，同样的数据能画出细得多的形状。渲染样式
//! 以后怎么换都只落在这一层，标定（dB 域）不用重调。

/// 满高：7 档（与方块字符 ▁▂▃▄▅▆▇█ 的 8 档对齐）
#[allow(dead_code)] // 波形用 0..=1，柱状样式接回来时用这个刻度
pub const MAX_HEIGHT: f32 = 7.0;

/// dB → 高度 的映射参数。`floor` 是 0 档，`floor + span` 是满格，`gamma` 调整曲率。
#[allow(dead_code)] // 波形不走这条映射，留给多段样式
#[derive(Debug, Clone, Copy)]
pub struct HeightCfg {
    pub floor: f32,
    pub span: f32,
    pub gamma: f32,
}

impl Default for HeightCfg {
    fn default() -> Self {
        HeightCfg {
            floor: -70.0,
            span: 70.0,
            gamma: 1.0,
        }
    }
}

impl HeightCfg {
    /// dB → 0..=7 连续高度（不四舍五入，渲染器自己决定怎么取整）
    pub fn height(&self, db: f32) -> f32 {
        let span = if self.span.abs() < 1e-3 { 1e-3 } else { self.span };
        let t = ((db - self.floor) / span).clamp(0.0, 1.0);
        let t = if (self.gamma - 1.0).abs() < 1e-3 {
            t
        } else {
            t.powf(self.gamma)
        };
        t * MAX_HEIGHT
    }

}

/// braille 点阵：`cols × rows` 个子点，`row 0` 在最上面
pub struct DotGrid {
    pub cols: usize,
    pub rows: usize,
    dots: Vec<bool>,
}

/// braille 子点 → 位。左列从上到下是 dot 1/2/3/7，右列是 4/5/6/8
const BRAILLE_BITS: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];

impl DotGrid {
    pub fn new(cols: usize, rows: usize) -> Self {
        DotGrid {
            cols,
            rows,
            dots: vec![false; cols * rows],
        }
    }

    pub fn is_set(&self, col: usize, row: usize) -> bool {
        col < self.cols && row < self.rows && self.dots[row * self.cols + col]
    }

    pub fn set(&mut self, col: usize, row: usize) {
        if col < self.cols && row < self.rows {
            self.dots[row * self.cols + col] = true;
        }
    }

    /// 合并成 braille 字符：返回 `ceil(rows/4)` 行 × `ceil(cols/2)` 列
    pub fn to_braille(&self) -> Vec<Vec<char>> {
        let crows = self.rows.div_ceil(4);
        let ccols = self.cols.div_ceil(2);
        (0..crows)
            .map(|cr| {
                (0..ccols)
                    .map(|cc| {
                        let mut bits = 0u8;
                        for (dc, col_bits) in BRAILLE_BITS.iter().enumerate() {
                            for (dr, bit) in col_bits.iter().enumerate() {
                                if self.is_set(cc * 2 + dc, cr * 4 + dr) {
                                    bits |= bit;
                                }
                            }
                        }
                        if bits == 0 {
                            ' '
                        } else {
                            char::from_u32(0x2800 + bits as u32).unwrap_or(' ')
                        }
                    })
                    .collect()
            })
            .collect()
    }
}

/// 平滑（attack 起得快、release 落得慢），高度用 0..=7 连续值
#[allow(dead_code)] // 多段样式用；波形靠滚动本身平滑
pub fn smooth(prev: &[f32], target: &[f32], attack: f32, release: f32) -> Vec<f32> {
    prev.iter()
        .zip(target.iter())
        .map(|(p, t)| {
            let k = if *t >= *p { attack } else { release };
            (p + (t - p) * k).clamp(0.0, MAX_HEIGHT)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn height_cfg_maps_db_monotonically() {
        let cfg = HeightCfg {
            floor: -70.0,
            span: 70.0,
            gamma: 1.0,
        };
        assert_eq!(cfg.height(-70.0), 0.0);
        assert!((cfg.height(0.0) - 7.0).abs() < 1e-4);
        assert!(cfg.height(-35.0) > cfg.height(-50.0));
        // 超出两端都夹住，不 panic、不越界
        assert_eq!(cfg.height(-200.0), 0.0);
        assert_eq!(cfg.height(30.0), 7.0);
    }

    #[test]
    fn height_cfg_gamma_bends_curve() {
        let lin = HeightCfg {
            floor: -70.0,
            span: 70.0,
            gamma: 1.0,
        };
        let bent = HeightCfg {
            gamma: 0.7,
            ..lin
        };
        assert!(bent.height(-35.0) > lin.height(-35.0));
    }

    #[test]
    fn dot_grid_braille_bits_and_glyphs() {
        let mut g = DotGrid::new(2, 4);
        g.set(0, 0); // dot 1
        assert_eq!(g.to_braille(), vec![vec!['⠁']]);
        let mut h = DotGrid::new(2, 4);
        h.set(0, 3); // dot 7
        assert_eq!(h.to_braille(), vec![vec!['⡀']]);
        let mut k = DotGrid::new(2, 4);
        for c in 0..2 {
            for r in 0..4 {
                k.set(c, r);
            }
        }
        assert_eq!(k.to_braille(), vec![vec!['⣿']]);
        assert_eq!(DotGrid::new(2, 4).to_braille(), vec![vec![' ']]);
    }

    #[test]
    fn dot_grid_never_panics_on_tiny_area() {
        for cols in 0..4 {
            for rows in 0..4 {
                let mut g = DotGrid::new(cols, rows);
                g.wave(&[0.2, 0.6, 1.0]);
                let _ = g.to_braille();
            }
        }
    }

    #[test]
    fn wave_ring_syncs_new_samples_only() {
        let mut w = WaveRing::new();
        w.sync(&[0.1, 0.2], 2, 8);
        assert_eq!(w.snapshot(4), vec![0.0, 0.0, 0.1, 0.2]);
        // 只补新增的那一条
        w.sync(&[0.1, 0.2, 0.9], 3, 8);
        assert_eq!(w.snapshot(2), vec![0.2, 0.9]);
    }

    #[test]
    fn wave_ring_recovers_after_decoder_restart() {
        let mut w = WaveRing::new();
        w.sync(&[0.1, 0.1, 0.1, 0.1, 0.1], 5, 8);
        assert_eq!(w.len(), 5);
        // 解码器重启，序号从 0 重来：不能把新数据当成旧的丢掉，否则画面冻住
        w.sync(&[0.5], 1, 8);
        assert_eq!(w.len(), 1);
        assert_eq!(w.snapshot(2), vec![0.0, 0.5]);
    }

    #[test]
    fn sparkline_uses_eight_steps_and_caps() {
        let s = sparkline(&[0.0, 0.5, 1.0, 2.0], 4);
        let chars: Vec<char> = s.chars().collect();
        assert_eq!(chars.len(), 4);
        assert_eq!(chars[0], '▁');
        assert_eq!(chars[2], '█');
        assert_eq!(chars[3], '█'); // 越界夹住，不 panic
        assert_eq!(sparkline(&[], 4), "");
    }

    #[test]
    fn wave_column_mirrors_around_center() {
        let mut g = DotGrid::new(1, 8);
        g.set_wave_column(0, 0.0);
        assert!(g.is_set(0, 4)); // 中线
        let mut h = DotGrid::new(1, 8);
        h.set_wave_column(0, 1.0);
        assert!(h.is_set(0, 1)); // 顶（偶数行留一格边）
        assert!(h.is_set(0, 7)); // 底
        assert!(h.is_set(0, 4)); // 中线
        // 实心填充：中间行也要点亮，不能只有上下两条描边
        for r in 1..=7 {
            assert!(h.is_set(0, r), "第 {r} 行没填满");
        }
    }

    #[test]
    fn smooth_moves_fast_up_slow_down() {
        // 起得快：一步到 4.55；落得慢：一步只到 6.16（相对满格 7）
        let up = smooth(&[0.0], &[7.0], 0.65, 0.12);
        let down = smooth(&[7.0], &[0.0], 0.65, 0.12);
        assert!((up[0] - 4.55).abs() < 0.01, "{up:?}");
        assert!((down[0] - 6.16).abs() < 0.01, "{down:?}");
        // 多步之后都能收敛
        let mut v = vec![0.0f32];
        for _ in 0..30 {
            v = smooth(&v, &[7.0], 0.65, 0.12);
        }
        assert!((v[0] - 7.0).abs() < 0.1);
    }
}

// ---------- 对称波形（示波器，方案 B） ----------

/// 波形滚动缓冲：每个点列一个幅度（0..=1），右端是最新的
pub struct WaveRing {
    cols: std::collections::VecDeque<f32>,
    /// 已消费到的样本序号（解码线程给的单调计数）
    pub seq: u64,
}

impl Default for WaveRing {
    fn default() -> Self {
        WaveRing::new()
    }
}

impl WaveRing {
    pub fn new() -> Self {
        WaveRing {
            cols: std::collections::VecDeque::new(),
            seq: 0,
        }
    }

    /// 把解码线程的新样本追加进来；`cap` 是保留的最大列数
    pub fn sync(&mut self, tail: &[f32], seq: u64, cap: usize) {
        if tail.is_empty() {
            return;
        }
        // 解码器重启（跳转/换歌）后序号从 0 重来：旧数据作废，否则会一直丢新样本、画面冻住
        if seq < self.seq {
            self.cols.clear();
            self.seq = 0;
        }
        let new = (seq - self.seq) as usize;
        let take = new.min(tail.len());
        for a in &tail[tail.len() - take..] {
            self.cols.push_back(*a);
        }
        while self.cols.len() > cap.max(1) {
            self.cols.pop_front();
        }
        self.seq = seq;
    }

    /// 右侧对齐取最近 `cols` 列，不足的左边补 0（静音）
    pub fn snapshot(&self, cols: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; cols];
        let n = self.cols.len().min(cols);
        let skip = self.cols.len() - n;
        for (i, v) in self.cols.iter().skip(skip).enumerate() {
            out[cols - n + i] = *v;
        }
        out
    }

    #[allow(dead_code)] // 测试与调试用
    pub fn len(&self) -> usize {
        self.cols.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cols.is_empty()
    }

    #[allow(dead_code)] // 换歌/换源时清屏用
    pub fn clear(&mut self) {
        self.cols.clear();
        self.seq = 0;
    }
}

impl DotGrid {
    /// 一列对称波形：从**中线向上向下填满**到幅度位置（实心面积，不是描边）。
    /// `amp` 0..=1；静音时只剩中线那一个点，看起来就是一条细线。
    pub fn set_wave_column(&mut self, col: usize, amp: f32) {
        if col >= self.cols || self.rows < 3 {
            return;
        }
        let center = self.rows / 2;
        let span_max = center.min(self.rows - 1 - center);
        // 向下取整：安静段才会真的塌成 1 个点（细线），不会被四舍五入撑成一条厚带
        let span = (amp.clamp(0.0, 1.0) * span_max as f32).floor() as usize;
        let span = span.min(span_max);
        for r in center.saturating_sub(span)..=(center + span).min(self.rows - 1) {
            self.set(col, r);
        }
    }

    /// 波形整幅：`amps` 是每个点列一个幅度，实心填充
    pub fn wave(&mut self, amps: &[f32]) {
        for (c, a) in amps.iter().enumerate().take(self.cols) {
            self.set_wave_column(c, *a);
        }
    }

    /// 一个 braille 字符格里「离中线最远的点」的归一化距离（0 = 中线，1 = 上下边）
    pub fn cell_distance(&self, cell_col: usize, cell_row: usize) -> f32 {
        let half = (self.rows / 2).max(1) as f32;
        let mut best = 0.0f32;
        for dc in 0..2 {
            for dr in 0..4 {
                let (col, row) = (cell_col * 2 + dc, cell_row * 4 + dr);
                if self.is_set(col, row) {
                    let d = (row as f32 - self.rows as f32 / 2.0).abs() / half;
                    best = best.max(d);
                }
            }
        }
        best
    }
}

/// 兜底单行 sparkline：把幅度列压成 `width` 个 ▁▂▃▄▅▆▇█
pub fn sparkline(amps: &[f32], width: usize) -> String {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    if width == 0 || amps.is_empty() {
        return String::new();
    }
    let n = amps.len();
    (0..width)
        .map(|i| {
            let lo = i * n / width;
            let hi = ((i + 1) * n / width).max(lo + 1).min(n);
            let peak = amps[lo..hi].iter().copied().fold(0.0f32, f32::max);
            let idx = (peak.clamp(0.0, 1.0) * 7.0).round() as usize;
            BARS[idx.min(7)]
        })
        .collect()
}

/// 终端能不能画 braille：dumb 终端退 sparkline
pub fn braille_ok() -> bool {
    match std::env::var("TERM") {
        Ok(t) => !t.is_empty() && t != "dumb",
        Err(_) => true,
    }
}
