#!/bin/bash
# scripts/play-e2e.sh <tmux会话> <关键词> [等最久多少秒]
#
# 端到端起播脚本：按屏幕状态驱动，给「动播放相关代码」时做人肉回归用。
#
# 按屏幕状态驱动 zmusic 起播，不靠 sleep 猜：
#   1. 等搜索结果列表里出现带时长（mm:ss）的行
#   2. 确认列表拿到焦点（提示行显示 Tab→输入栏），再按 Enter
#   3. 按 v 进「正在播放」页，标题出现「正在播放」才算起播成功
# 失败时把整屏原文存到 /tmp/zb-fail-<时间戳>.txt，方便看现场。
set -u

S="${1:?会话名}"
KW="${2:?关键词}"
TIMEOUT="${3:-25}"

fail() {
  local why="$1"
  local out="/tmp/zb-fail-$(date +%H%M%S).txt"
  { echo "why: $why"; echo "--- pane ---"; tmux capture-pane -p -t "$S" 2>/dev/null; } >"$out"
  echo "FAIL: $why (现场: $out)"
  exit 1
}

# 等到屏幕上出现匹配 $1 的行（最多 $2 秒）
wait_for() {
  local pat="$1" secs="$2" i
  for ((i = 0; i < secs * 10; i++)); do
    tmux capture-pane -p -t "$S" 2>/dev/null | grep -qE "$pat" && return 0
    sleep 0.1
  done
  return 1
}

tmux has-session -t "$S" 2>/dev/null || fail "会话 $S 不存在"

# 回搜索页（Esc 只返回，不退出程序）
tmux send-keys -t "$S" Escape; sleep 0.3
tmux send-keys -t "$S" 's'
wait_for '搜索' 5 || fail "没进搜索页"
sleep 0.2
tmux send-keys -t "$S" -l "$KW"
sleep 0.3
tmux send-keys -t "$S" Enter

# 1) 等带时长的结果行。搜索可能因为「源暂不可用」返回空，这时按 R 重新探测再重搜，
#    不能直接判死——API 慢的时候第一次探测经常是失败态。
deadline=$((SECONDS + TIMEOUT))
retried=0
while ((SECONDS < deadline)); do
  if tmux capture-pane -p -t "$S" | grep -qE '[0-9]{2}:[0-9]{2}'; then
    break
  fi
  if grep -q '不可用' <(tmux capture-pane -p -t "$S") && ((retried < 3)); then
    ((retried++))
    tmux send-keys -t "$S" 'R'
    sleep 2
    tmux send-keys -t "$S" Enter
  fi
  sleep 0.2
done
tmux capture-pane -p -t "$S" | grep -qE '[0-9]{2}:[0-9]{2}' || fail "等了 ${TIMEOUT}s 没有搜索结果行"

# 2) 焦点：提示行还是「Tab→列表」说明输入框有焦点，按 Tab 到列表
tmux send-keys -t "$S" Tab
sleep 0.3
tmux capture-pane -p -t "$S" | grep -q 'Tab→输入栏' || {
  tmux send-keys -t "$S" Tab
  sleep 0.3
}
tmux capture-pane -p -t "$S" | grep -q 'Tab→输入栏' || fail "列表拿不到焦点"

# 3) 播放并确认「正在播放」页真的出现
tmux send-keys -t "$S" Enter
for ((i = 0; i < 200; i++)); do
  if tmux capture-pane -p -t "$S" | head -1 | grep -q '正在播放'; then
    exit 0
  fi
  tmux send-keys -t "$S" 'v' 2>/dev/null
  sleep 0.1
done
fail "按了 Enter 但没进「正在播放」"
