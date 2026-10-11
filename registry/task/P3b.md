---
kind: task
id: P3b
title: 反射档样本恒为零：两个出厂 `false` 的开关缺一不可
status: done
created: 2026-10-11
track: P
deps: [P3]
where: 13 §2 轨 P
evidence: 13 §2「P3b 未开工的理由」（只登记、不动默认值的定案）+ `config.rs` 的 `default_skill_compile_enabled` / `default_skill_fast_path` 两条成文理由
---
## 内容
反射档样本恒为零：两个出厂 `false` 的开关缺一不可

## 处理过程
- 迁自 13 §1 问题登记（2026-10-11 进入登记机制）；批次内容与目标态归 13 §2，实施过程查 `git log`。
- **本批是「登记型」批次**：13 §2 定案「只登记，不动默认值」——两个开关
  （`skill_compile_enabled` / `skill_fast_path`）出厂均 `false` 且各带成文理由
  （`config.rs`），且任一真子集下 `reflex` 桶观测值恒零 ⇒ 按 08 §0 纪律 1c 属**原子批次**，
  翻默认值须有度量支撑（ADR-048 后果节）而非「更有效」。故本任务的交付物**就是这份登记**
  （已在 13 §2 落定），不含翻默认值——后者是将来按 1c 三条判据另行开工的行为变更。
