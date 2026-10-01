# ADR 0001：在独立 Repository 中开发 Aries Next

> 最后验证日期：2026-09-06。本决策至今仍有效，未被后续决策取代。

## 状态

已接受。

## 背景

Aries Next 同时更换 Backend Language、Frontend Framework、Database 和部署结构。如果将新实现长期放在旧 Go Repository 的 `v2` 子目录中，两套 Build、Dependency、CI 和 Release Boundary 会互相干扰。

## 决策

Aries Next 在旧 Aries 同级的独立 Repository 中开发，不在旧 Repository 内维护永久的 `v2` 目录。

目录关系：

```text
aries       # Legacy Go Application
aries-next  # Rust、Vue 3、Nuxt 4、PostgreSQL Application
```

## 后果

正向影响：

- 旧应用继续保持可部署，能够作为稳定的 Behavior 和 Data Reference。
- 新 Repository 具有清晰的 Rust、Node.js、CI 和 Release Boundary。
- 新项目可以独立制定 Coding Standard、Migration 和 Deployment Policy。

需要承担的成本：

- 旧版与新版的行为差异不能依赖同一 Source Tree 发现。
- 必须通过 Contract Test、Migration Document 和 URL Compatibility Test 固化跨版本行为。
- 正式切换前需要独立维护两套 Build 和 Deployment Pipeline。

## 后续约束

- 旧 Aries 只接受影响生产运行或数据迁移的必要修复。
- 新功能默认只在 Aries Next 实现。
- 迁移完成且 Rollback Window 结束后，再归档旧 Repository。
