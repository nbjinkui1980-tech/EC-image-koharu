# Lane 执行合同:L-AR10 — 发布 provenance(SHA 固定 + 最小权限 + 同 run 制品)

- 状态:**Phase 3 一次批准已覆盖(2026-08-13 授予)**;LOOP-3 本地认领已登记
- 认领基线:Phase 3 起点 main `b68f123e`;执行分支 `audit-remediation-phase3`(认领时 tip `20ec72c0`)
- 提交/回滚单元:T01/T02/T03 各一个单卡 commit(可独立 revert);lane 收口 docs(evidence) 单独一个 commit
- 前置依赖:T02 ← T01;T03 ← T02(lane 内链式)
- **§0.2 执行约束**:仅本地修改 `.github/workflows/`,不 push、不触发、不调试远端 GitHub Actions/release;真实凭据(Azure/Apple/Sentry/GHCR push)保持 PENDING-CREDENTIAL-QA 不触碰
- **§0.9 凭据门禁**:本 lane 不新增/修改任何 secrets 引用,只动权限范围、固定值与校验逻辑
- 执行环境偏差(继承):oracle 可用;lane 收口独立 review 沿用 oracle,失败则对抗性自审并落档
- 串行点声明:CI/workflow lane——门禁为 policy test(bun test)+ actionlint(若可用);不占用 Cargo target 热路径(workspace check 仅在收口验证无交叉)

## 范围文件域(域外改动禁止)

| 卡 | 允许文件 |
|---|---|
| AR10-T01 | `scripts/supply-chain-policy.test.ts`(test-as-policy 追加)、`.github/workflows/build.yml`、`publish.yml`、`release.yml`(≤4) |
| AR10-T02 | `.github/workflows/release.yml`、`scripts/supply-chain-policy.test.ts`(≤2) |
| AR10-T03 | `.github/workflows/release.yml`、`Dockerfile`、`scripts/supply-chain-policy.test.ts`(≤3) |

新依赖:无(策略测试用行级解析,不引 YAML 库)。

## 卡:AR10-T01 — Actions 固定完整 SHA

- **验收标准(TASKS 原文)**:文件:新 `scripts/supply-chain-policy.test.ts`、`.github/workflows/build.yml`、`publish.yml`、`release.yml`。RED:任意非本地 `uses:` 不是 40 字符 SHA。GREEN:固定 commit SHA,并保留版本注释。验证:policy test、`actionlint`。
- **现状(RED-0 实证)**:三文件全部 `uses:` 均为 tag/branch 引用——`actions/checkout@v7` ×4、`oven-sh/setup-bun@v2` ×3、`Jimver/cuda-toolkit@master` ×5(滚动分支!)、`Swatinem/rust-cache@v2` ×2、`ilammy/msvc-dev-cmd@v1` ×3、`actions/cache@v6` ×2、`tauri-apps/tauri-action@v0`、`docker/setup-buildx-action@v4`、`docker/login-action@v4`、`docker/build-push-action@v7`、`docker/metadata-action@v5`、`actions/upload-artifact@v7`。
- **设计**:
  - `supply-chain-policy.test.ts` 追加 `workflow action pinning` describe:行级扫描三文件全部 `uses:`,非本地(不以 `./` 开头)必须匹配 `owner/repo@<40 hex>` 且同行带 `# <tag>` 版本注释
  - 逐 action 用 `git ls-remote <repo> refs/tags/<tag>^{}`(peeled commit)解析当前 SHA 并替换;`Jimver/cuda-toolkit@master` 固定 master 当前 commit,注释 `# master (pinned 2026-08-15)`
  - actionlint 可用性:本地无则记录工具缺失,以 policy test 为主门禁
- **RED 断言**(policy test):
  1. `release workflows pin every non-local action to a full commit sha` — 三文件任一 `@tag`/`@master` → FAIL
  2. `pinned actions carry the version comment` — 缺 `# <tag>` 注释 → FAIL
- **目标文件**:上表 T01 行(≤4)
- **验收命令**:`bun test scripts/supply-chain-policy.test.ts`
- **证据**:RED 2F/4P(tag/branch 引用+无版本注释)→GREEN 6/6;commit `051ecea9`;12 个 action 经 `git ls-remote` peeled 解析(轻量 tag 回退裸 ref;Jimver/cuda-toolkit master 固定 `b8bf9c6c`+注释);actionlint 本地不可用(记录),YAML 结构经 safe_load 验证(注意 YAML 1.1 `on:`→True 陷阱)

## 卡:AR10-T02 — Release 最小权限与签名 CLI digest

- **验收标准(TASKS 原文)**:依赖:AR10-T01。文件:`release.yml`、`supply-chain-policy.test.ts`。RED:workflow 顶层写权限过宽;下载执行 CLI 无版本/digest 校验。GREEN:权限下放到 job;CLI 固定版本和 digest。验证:policy test、release actionlint;凭据项保持 `PENDING-CREDENTIAL-QA`。
- **现状(RED-0 实证)**:release.yml 顶层 `permissions: contents:write + id-token:write + packages:write` 覆盖全部 job;`trusted-signing-cli.exe` 下载固定 0.8.0 但无 sha256 校验;`id-token:write` 无 OIDC 消费步骤(Azure 走 service principal secrets)。
- **设计**:
  - 顶层 `permissions: contents: read`(收紧默认);release job:`contents: write`(tauri-action 建 release);container job:`contents: read` + `packages: write`(GHCR push);删除 `id-token: write`(无消费方)
  - trusted-signing-cli 下载后 sha256 校验:本机下载 0.8.0 算 digest 钉入,不匹配即 fail(决策点:本机算值记合同)
  - policy test 追加:顶层无写权限;仅声明的 job 级权限白名单;CLI 下载步骤含 digest 校验
- **RED 断言**(policy test):
  1. `release workflow has no write permissions at top level` → 现状 FAIL
  2. `trusted-signing-cli download verifies sha256` → 现状 FAIL
  3. `job permissions stay within the declared allowlist`(锁)
- **目标文件**:上表 T02 行(≤2)
- **验收命令**:`bun test scripts/supply-chain-policy.test.ts`
- **证据**:RED 4F/6P(顶层写权限/id-token/job 白名单/CLI 无 digest)→GREEN 10/10;commit `138237e0`;CLI digest `39ece56f` 本机自 0.8.0 资产计算(决策点回填);id-token:write 删除经取证(Azure 走 service principal secrets,无 OIDC 消费)

## 卡:AR10-T03 — 同 run artifact、Docker provenance 与 fork

- **验收标准(TASKS 原文)**:依赖:AR10-T02;独占 Dockerfile。文件:`release.yml`、`Dockerfile`、`supply-chain-policy.test.ts`。RED:Dockerfile 使用 `releases/latest`;container 重建/下载不同 binary;authority 不是 `nbjinkui1980-tech`。GREEN:只消费当前 run immutable artifact + digest;OCI source/revision/version;统一 fork。验证:policy test、actionlint、本地 docker build/digest compare;不 push。
- **现状(RED-0 实证)**:Dockerfile `curl -fL https://github.com/mayocream/koharu/releases/latest/download/koharu_linux_x64`(上游!非本 run!无 digest!);release job 不上传 artifact;container job 从源码 context 构建;`IMAGE_NAME: ${{ github.repository }}` 不固定 fork。
- **设计**:
  - release job(linux arm64 之外还需要 linux/amd64 二进制供容器——现状 matrix 的 ubuntu-latest 产物即 amd64):tauri-action 后计算 `target/release/koharu` sha256 并 `upload-artifact`(名字+sha256 清单),供同 run 消费
  - container job:`actions/download-artifact`(固定 SHA,T01 政策同约束)→ 校验 sha256 → 构建 context 仅含 Dockerfile+已验证二进制
  - Dockerfile:删 curl-upstream,改 `COPY` 已验证二进制(构建参数带期望 digest,构建内复核)
  - `IMAGE_NAME` 固定 `nbjinkui1980-tech/koharu`(统一 fork authority);policy test 断言
  - OCI labels:metadata-action 显式 `org.opencontainers.image.source/revision/version`(source=fork repo URL,revision=tag SHA,version=tag)
  - 本地验证:docker build/digest compare 需 daemon——daemon 不可用则按 §0 记录环境阻塞,以 policy test + actionlint + Dockerfile 静态检查为门禁
- **RED 断言**(policy test):
  1. `dockerfile never downloads from releases/latest` → 现状 FAIL
  2. `container job consumes only the same-run artifact with a digest check` → 现状 FAIL
  3. `image name is pinned to the fork authority` → 现状 FAIL
- **目标文件**:上表 T03 行(≤3)
- **验收命令**:`bun test scripts/supply-chain-policy.test.ts`
- **证据**:RED 4F/10P(releases/latest+curl/无同 run 制品/authority 未固定/无 OCI 标签)→GREEN 14/14;commit `4b8c5caa`;Docker daemon 不可用→本地 build 环境阻塞(记录);digest 语义 shell 级实证(well-formed 错误 digest fail-closed exit 1,正确 exit 0;早前一次 exit 误判为管道遮蔽)

## Lane 收口(2026-08-15)

- 门禁:policy 16P/0F(supply-chain 14 + tauri-security-config 2)、workspace check 净、check:generated 零漂移;YAML safe_load 结构验证;actionlint 本地不可用(记录);Docker build 环境阻塞(daemon 不可用,digest 语义 shell 级实证替代);**未 push、未触发远端**(§0.2)
- 独立 review(oracle):**3 blocker**——job 级 permissions 未声明的 scope 全部置 none,release job 缺 `actions: write`(upload-artifact 必败)、container job 缺 `actions: read`(download-artifact 必败)(均修+政策锁);sha256 清单嵌入 `target/release/koharu` 路径,download-artifact 解包到 dist 后 `sha256sum -c` 找不到文件(修:清单用裸文件名,补"manifest 用裸名"政策断言);**2 major**——CLI 步骤提取正则 6 空格硬编码(修为空白通用)/政策仅覆盖三文件(裁决记录:卡域即三文件,CI workflows 固定留作后续);minor/informational 逐条裁决(GHA 默认 shell 已 `-eo pipefail`;`$KOHARU_SHA256` 已加引号;Dockerfile 全禁 curl 保持;README 上游引用超卡域记录)
- review-fix commit:`0bb71c3a`(2 文件,+12/-2)
- 提交/回滚单元:T01 `051ecea9`、T02 `138237e0`、T03 `4b8c5caa`、review-fix `0bb71c3a`,均可独立 revert
- 依赖传播:AR14-T06 🔴→🟡(←AR10-T03 ✅);无 wave 收齐(W6 差 AR14-T06 + AR14-T03A~E ◐)
