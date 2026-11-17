<!--
  LoadingElapsed — 「已经等了多久」的一行小字（加载态专用）

  ## 为什么需要它

  一句静止的「正在加载…」在长等待里无法区分「慢」与「卡死」。后端没有进度通道
  （一次 `vdfs/read` 就是一次往返），所以这里不编造百分比，只如实给出**已等待
  的秒数**——时间在动，就说明没死。

  ## 为什么前几秒不显示

  秒数从 0 开始读会让一次瞬时就完成的加载也闪一下「已等待 1 秒」，反而制造焦虑；
  3 秒后仍未完成才是「这次真的慢」。
-->
<template>
  <span v-if="text" class="loading-elapsed" role="status">{{ text }}</span>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'

/** 超过这个秒数才开始报时 */
const SHOW_AFTER_SECONDS = 3

const seconds = ref(0)
let timer: ReturnType<typeof setInterval> | null = null

onMounted(() => {
  timer = setInterval(() => {
    seconds.value += 1
  }, 1000)
})

onBeforeUnmount(() => {
  if (timer) clearInterval(timer)
  timer = null
})

const text = computed(() =>
  seconds.value >= SHOW_AFTER_SECONDS ? `已等待 ${seconds.value} 秒` : '',
)
</script>

<style scoped>
.loading-elapsed {
  font-size: var(--font-size-xs);
  color: var(--text-muted);
  font-variant-numeric: tabular-nums;
}
</style>
