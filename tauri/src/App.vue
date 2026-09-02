<template>
  <div class="app-container">
    <RouterView />
  </div>
</template>

<script setup lang="ts">
import { RouterView } from 'vue-router'
</script>

<style>
* {
  margin: 0;
  padding: 0;
  box-sizing: border-box;
}

:root {
  --color-primary: #667eea;
  --color-primary-dark: #764ba2;
  --color-bg: #f5f5f5;
  --color-surface: #ffffff;
  --color-border: #e0e0e0;
  --color-text: #333333;
  --color-text-secondary: #666666;
  --color-text-muted: #999999;
  
  --sidebar-width: 48px;
  --panel-width: 280px;
  --header-height: 48px;

  /* ── 会话区语义令牌（浅色）──
     所有会话区可见配色集中在此，深色主题在下方覆盖同名变量即可自动适配。
     组件内禁止再出现业务硬编码色值，统一引用这些令牌。 */
  --color-chat-bg: #ffffff;          /* 聊天消息区背景 */
  --color-msg-card: #f6f8fb;         /* 思考 / 工具等次级内容卡片底色（极淡） */
  --color-msg-card-border: #e8edf4;  /* 卡片描边 */
  --msg-gap: 0.45rem;
  --color-nested-line: #e8edf4;      /* 嵌套层级引导竖线 */
  --color-chip-bg: #eef2f7;          /* 中性图标芯片底 */
  --color-chip-fg: #64748b;          /* 中性图标芯片字 */
  --color-chip-user-bg: #dbeafe;     /* 用户图标芯片底 */
  --color-chip-user-fg: #2563eb;     /* 用户图标芯片字 */
  --color-chip-sub-bg: #e0f2fe;      /* 子智能体图标芯片底 */
  --color-chip-sub-fg: #0284c7;      /* 子智能体图标芯片字 */
  --color-chip-tool-bg: #eef2ff;     /* 工具图标芯片底 */
  --color-chip-tool-fg: #4f46e5;     /* 工具图标芯片字 */
  --color-chip-reasoning-bg: #f3e8ff;/* 思考图标芯片底 */
  --color-chip-reasoning-fg: #7c3aed;/* 思考图标芯片字 */
  --color-user-bubble: #2563eb;      /* 用户气泡底（扁平纯色） */
  --color-user-bubble-fg: #ffffff;   /* 用户气泡字 */
  --color-code-bg: #0f172a;          /* 代码块底 */
  --color-code-fg: #e2e8f0;          /* 代码块字 */
  --color-prompt-bg: #eef2ff;        /* 待用户响应（提问/确认）卡底 */
  --color-prompt-border: #c7d2fe;    /* 待用户响应卡描边 */
  --color-error-bg: #fef2f2;         /* 错误卡底 */
  --color-error-border: #fecaca;     /* 错误卡描边 */
  --color-error-fg: #b91c1c;         /* 错误卡字 */
  --color-supply-bg: #ffffff;        /* 补充参数表单底 */
  --color-supply-border: #dbe2f0;    /* 补充参数表单描边 */
  --color-option-bg: #ffffff;        /* 选项芯片底 */
  --color-option-border: #dbe2f0;    /* 选项芯片描边 */
  --color-option-active-bg: #eef2ff; /* 选项选中底 */
  --color-option-active-border: #6366f1; /* 选项选中描边 */
  --color-tag-sub-bg: #e0e7ff;       /* 节点标签·子底色 */
  --color-tag-sub-fg: #4338ca;       /* 节点标签·子字色 */
  --color-tag-warn-bg: #fef3c7;      /* 节点标签·警告底 */
  --color-tag-warn-fg: #92400e;      /* 节点标签·警告字 */
  --color-tag-err-bg: #fee2e2;       /* 节点标签·错误底 */
  --color-tag-err-fg: #b91c1c;       /* 节点标签·错误字 */
  --color-banner-bg: #fef3c7;        /* 初始化错误横幅底 */
  --color-banner-border: #f59e0b;    /* 初始化错误横幅描边 */
  --color-banner-fg: #92400e;        /* 初始化错误横幅字 */
}

html, body {
  height: 100%;
  overflow: hidden;
}

body {
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, sans-serif;
  background: var(--color-bg);
  color: var(--color-text);
  /* 用 rem 参照 <html> 根字号（16px），使字号档位（小/中/大）能整体缩放界面文本 */
  font-size: 0.875rem;
  line-height: 1.5;
}

.app-container {
  display: flex;
  height: 100vh;
  width: 100vw;
  overflow: hidden;
}

/* 滚动条样式 */
::-webkit-scrollbar {
  width: 8px;
  height: 8px;
}

::-webkit-scrollbar-track {
  background: transparent;
}

::-webkit-scrollbar-thumb {
  background: #ccc;
  border-radius: 4px;
}

::-webkit-scrollbar-thumb:hover {
  background: #bbb;
}

:root[data-theme="dark"] {
  --color-primary: #818cf8;
  --color-primary-dark: #6366f1;
  --color-bg: #1e1e2e;
  --color-surface: #27273a;
  --color-border: #3f3f5a;
  --color-text: #e2e8f0;
  --color-text-secondary: #94a3b8;
  --color-text-muted: #64748b;

  /* ── 会话区语义令牌（深色覆盖）── 与浅色同名变量一一对应 */
  --color-chat-bg: #181825;
  --color-msg-card: #27273a;
  --color-msg-card-border: #3f3f5a;
  --msg-gap: 0.45rem;
  --color-nested-line: #3f3f5a;
  --color-chip-bg: #313147;
  --color-chip-fg: #94a3b8;
  --color-chip-user-bg: #1e3a5f;
  --color-chip-user-fg: #93c5fd;
  --color-chip-sub-bg: #0c2e3e;
  --color-chip-sub-fg: #38bdf8;
  --color-chip-tool-bg: #232347;
  --color-chip-tool-fg: #a5b4fc;
  --color-chip-reasoning-bg: #2e2342;
  --color-chip-reasoning-fg: #c4b5fd;
  --color-user-bubble: #3b82f6;
  --color-user-bubble-fg: #ffffff;
  --color-code-bg: #11111b;
  --color-code-fg: #e2e8f0;
  --color-prompt-bg: #232347;
  --color-prompt-border: #4f46e5;
  --color-error-bg: #2a1f24;
  --color-error-border: #7f1d1d;
  --color-error-fg: #fca5a5;
  --color-supply-bg: #21222e;
  --color-supply-border: #3f3f5a;
  --color-option-bg: #21222e;
  --color-option-border: #3f3f5a;
  --color-option-active-bg: #232347;
  --color-option-active-border: #818cf8;
  --color-tag-sub-bg: #232347;
  --color-tag-sub-fg: #a5b4fc;
  --color-tag-warn-bg: #3a2f12;
  --color-tag-warn-fg: #fcd34d;
  --color-tag-err-bg: #2a1f24;
  --color-tag-err-fg: #fca5a5;
  --color-banner-bg: #3a2f12;
  --color-banner-border: #b45309;
  --color-banner-fg: #fcd34d;
  /* 输入区 / 弹层 / 悬浮态等派生表面 */
  --color-input-bg: #11111b;
  --color-surface-strong: #21222e;
  --color-hover-bg: #2a2a3e;
  --color-active-bg: #232347;
}
</style>
