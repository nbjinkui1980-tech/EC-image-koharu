# Lane 执行合同:L-AR09 — 产物完整性(SHA-256 下载/缓存不变量 + NativeArtifact + PyPI digest)

- 状态:**Phase 3 一次批准已覆盖(2026-08-13 授予)**;LOOP-3 本地认领已登记
- 认领基线:Phase 3 起点 main `b68f123e`;执行分支 `audit-remediation-phase3`(认领时 tip `515a2c0e`)
- 提交/回滚单元:T01/T02/T03 各一个单卡 commit(可独立 revert);lane 收口 docs(evidence) 单独一个 commit
- 前置依赖:T02/T03 ← T01(本 lane 内)
- 执行环境偏差(继承):子代理模型映射故障未修(15 次失败),codegraph + 直接工具单执行器;lane 收口独立 review 重试,失败则对抗性自审并落档

## 范围文件域(域外改动禁止)

| 卡 | 允许文件 |
|---|---|
| AR09-T01 | `crates/koharu-runtime/src/downloads.rs`、`install.rs`、`crates/koharu-runtime/Cargo.toml`、根 `Cargo.toml`(≤4) |
| AR09-T02 | `crates/koharu-runtime/src/llama.rs`、`zluda.rs`、`downloads.rs`、`install.rs`(≤4) |
| AR09-T03 | `crates/koharu-runtime/src/cuda.rs`、`downloads.rs`、`install.rs`(≤3) |

新依赖:`sha2 0.10.9`(TASKS 已批准直接边)入 workspace + koharu-runtime。

## 卡:AR09-T01 — SHA-256 下载/缓存不变量

- **验收标准(TASKS 原文)**:RED:已有缓存不重验 digest;错误下载覆盖已验证安装;marker 不含 digest。GREEN:增加已批准 `sha2 0.10.9` 直接边;缓存、下载、解压前共用验证;source id 含 digest。
- **现状(RED-0 源码实证)**:`cached_download` 的 `destination.exists()` 即返(不重验);`ranged_download` 写完即用(下载后不验);`InstallState` marker 内容=裸 source_id(无 digest 段)。
- **设计**:
  - `sha2 0.10.9` 入 workspace 根 + koharu-runtime 依赖
  - downloads.rs 新增:`verify_sha256(path, expected_hex) -> Result<bool>`(流式读);`cached_download_with_sha256(url, file_name, expected)`:缓存命中先验 digest(不符则删除重下);下载后验 digest,不符删临时产物不返路径;digest 不符永不覆盖已验证缓存(先验后替)
  - `cached_download`(无 digest 旧签名)保留至 T02/T03 迁移完;本卡不动 llama/zluda/cuda
  - install.rs:marker 格式保持"内容 == source_id"语义;"source id 含 digest"由 T02/T03 在各包 source_id 内编入 digest 段实现(T01 基建就位即可)
- **RED 断言**(`bun cargo test -p koharu-runtime downloads`/`install`):
  1. `cached_download_with_sha256_rejects_bad_digest_and_keeps_cache` — 预置好缓存;同 file_name 经 digest API 以**错误** expected 调用 → Err 且原缓存保留;当前 API 不存在(RED-0 骨架:API 存在但仅委托无验证)→ FAIL
  2. `cached_download_with_sha256_redownloads_corrupt_cache` — 缓存内容与 expected 不符 → 删除重下正确内容(本地双 TcpListener/文件 server 供正确 bytes)→ 当前骨架 FAIL
  3. `verify_sha256_streams_and_matches` — 锁:正确 digest → true;错误 → false
- **目标文件**:上表 T01 行(≤4)
- **验收命令**:`bun cargo test -p koharu-runtime downloads`、`bun cargo test -p koharu-runtime install`
- **证据记录**:RED / GREEN / commit SHA

## 卡:AR09-T02 — llama/ZLUDA artifact 描述

- **验收标准(TASKS 原文)**:RED:artifact 缺 URL/digest/archive_kind/selected_files;mismatch 仍 extract/preload。GREEN:最小 `NativeArtifact` 数据结构;错误 digest 清 temp,不替换安装。
- **digest 值获取策略(决策点)**:llama.cpp/ZLUDA 的 GitHub release 无官方 digest 清单 → **本机一次性下载全部 6+1 个 artifact 计算 sha256 钉入代码**(值与下载过程记录于证据;跨平台 artifact 仅为文件下载计算,不需对应平台)。
- **设计**:`NativeArtifact { url, sha256, archive_kind, selected_files }`;llama/zluda 的 assets() 迁移为该结构数组;下载经 T01 的 `cached_download_with_sha256`;digest 不符 → 清下载 temp,install 目录不 reset/不替换(保持已验证安装);`source_id` 编入全部 artifact digest 的短缀(`llama-{tag}-{distro};sha256={首个 artifact digest 前 12}` 形态)。
- **RED 断言**(`bun cargo test -p koharu-runtime llama`/`zluda`):
  1. `native_artifact_carries_url_digest_kind_files` — 结构存在且每 artifact 四字段齐 → 当前无结构(RED-0 骨架后:存在但 digest 空/验证未接)→ FAIL
  2. `llama_bad_digest_keeps_existing_install` — 预置已验证安装(marker current);artifact digest 篡改 → ensure_ready Err 且 marker/目录不变 → FAIL
  3. `source_id_includes_digest` — source_id 含 digest 段 → FAIL
- **目标文件**:上表 T02 行(≤4)
- **验收命令**:`bun cargo test -p koharu-runtime llama`、`bun cargo test -p koharu-runtime zluda`
- **证据记录**:RED / GREEN / digest 值与计算命令 / commit SHA

## 卡:AR09-T03 — CUDA PyPI 官方 digest

- **验收标准(TASKS 原文)**:RED:wheel metadata 缺 digest 仍安装,或 digest 变化不改变 source id。GREEN:只信任 PyPI `digests.sha256`;缺失 fail closed。
- **现状(RED-0 源码实证)**:`PypiFile{filename, url}`——未读 `digests` 字段;`select_wheel` 选中即下;source_id 无 digest。
- **设计**:`PypiFile` 加 `digests: Option<PypiDigests>`(`digests.sha256`);`select_wheel` 缺 digest → bail(fail closed);wheel 下载经 `cached_download_with_sha256`;source_id 编入各 wheel digest 短缀。
- **RED 断言**(`bun cargo test -p koharu-runtime cuda`):
  1. `cuda_wheel_missing_digest_fails_closed` — mock metadata 无 digests → Err;当前忽略 → FAIL
  2. `cuda_source_id_tracks_digest` — digest 变化 → source_id 变化 → FAIL
  3. 锁:platform_tags/wheel 选择行为不变
- **目标文件**:上表 T03 行(≤3)
- **验收命令**:`bun cargo test -p koharu-runtime cuda`
- **证据记录**:RED / GREEN / commit SHA

---

## Lane 收口门禁(Wave 5 gate 对齐)

- `bun cargo test -p koharu-runtime`、`-p koharu-app`、`-p koharu-rpc` 全绿
- `bun cargo clippy --workspace --all-targets -- -D warnings`、`bun cargo fmt --all -- --check`
- `bun cargo check --workspace --all-targets`
- `bun run check:generated`(零漂移确认)
- 独立 scoped code-review 零发现(重试子代理;故障则对抗性自审并落档偏差)
- digest 拒绝/保留已验证安装/fail-closed 可重复演示(测试输出)
- macOS/Windows/Linux 真实 artifact smoke 依 TASKS 后置(落档)

## 风险与决策点(批准时一并确认)

- **T02 digest 钉值**:本机下载全部 llama/zluda artifact 算 sha256(一次性,数百 MB~GB 级);值+计算命令入证据。若网络受限则本卡暂停待网
- sha2 为 TASKS 批准新直接边;workspace 已有 blake3,但 TASKS 明确 sha2(digest 与外部生态 SHA-256 对齐)
- T01 的 `cached_download`(无 digest 旧签名)在 T02/T03 迁移后应零调用方——lane 收口时确认并移除(若成孤儿)
