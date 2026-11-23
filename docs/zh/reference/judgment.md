---
title: 判定参考
---

# 判定参考（`spoke-judgment`）

`spoke-judgment` 是可选的数据/一致性能力：表达对目标的已声明评估 —— 一个由精确钉住修订的证据支撑的 `Judgment` 信封。该能力在两套语言栈上定义信封形状、纯校验辅助函数与 RFC 8785 规范比较辅助函数。

## 能力（Capability）

| 方面 | 契约 |
|------|------|
| 能力字符串 | `spoke-judgment` —— `HostCapabilityManifest.capabilities` 的可选成员 |
| manifest 声明 | 主机校验或产出 `Judgment` 数据信封，并符合其纯校验与规范比较契约 |
| 依赖 | 无共能力；独立于 `spoke-knowledge` 及其它所有族 |
| 协商 | 仅 manifest：`spoke-judgment` 没有 Connect 分派行，也没有自己的操作 |
| Connect 面 | 无；承载 `Judgment` 的 `tools.<ns>.<tool_id>` 操作仍按其自身完整操作字符串门控 |

该声明表示数据与校验器一致性。校验与比较都是纯的：无 I/O、无注册表、无存储、无推理、无产品策略求值。

## 数据契约（Data contract）

### `Judgment`

封闭对象（`additionalProperties: false`）：

| 字段 | 存在性 | 类型 | 契约 |
|------|--------|------|------|
| `schema_version` | 必填 | 共享 `SchemaVersion` | 互操作整数域内 ≥ 1 的整数；`Judgment` 线上 schema 的版本 |
| `judgment_id` | 必填 | 非空字符串 | 不透明稳定判定 id |
| `judgment_kind` | 必填 | 非空开放字符串 | 评估种类词汇 |
| `target` | 必填 | `JudgmentRef` | 被评估的对象；其 `revision` 可选，存在时为正 |
| `evidence` | 必填 | `JudgmentEvidenceRef[]` | 即使为空也存在；每一项钉住必填的正 `revision` |
| `verdict` | 必填 | `Verdict` | 封闭核心加开放的产品细节 |
| `basis` | 可选 | string | 产品提供的解释或理由；它不替代 `evidence` |
| `created_at` | 可选 | 共享 `Timestamp` | 带显式 UTC 偏移的 RFC 3339 date-time |
| `extensions` | 必填 | 共享 `ExtensionMap` | 产品 namespace 扩展；为空时 `{}` |

必填成员是 `schema_version`、`judgment_id`、`judgment_kind`、`target`、`evidence`、`verdict` 与 `extensions`；仅 `basis` 与 `created_at` 可选。

### `JudgmentRef` 与 `JudgmentEvidenceRef`

`JudgmentRef` 是 `Judgment.target` 与每个 `Judgment.evidence[]` 项使用的轻量引用，为封闭对象：

| 字段 | 存在性 | 类型 | 契约 |
|------|--------|------|------|
| `kind` | 必填 | 非空开放字符串 | 标识被引用物种类，无需协议注册表 |
| `id` | 必填 | 非空字符串 | 在适用种类与产品作用域内的不透明稳定 id |
| `namespace` | 可选 | 非空字符串 | 该引用的产品作用域；不是被引用物身份的必填组成部分 |
| `revision` | 条件 | 正整数（≥ 1） | 适用修订钉扎时钉住所引用修订；钉扎在两套栈上都保持精确 |

收窄规则：`Judgment.target.revision` 可选，存在时为正。每个 `Judgment.evidence[]` 项都是 `JudgmentEvidenceRef` —— 一个 `revision` 必填且为正的 `JudgmentRef`。因此证据始终携带判定所依赖的确切修订，而目标可按产品决定不做钉扎评估。

`JudgmentRef` 只携带 kind、id、可选产品作用域与可选 revision；它不复用 `CognitionReference` 或 `CognitionTarget`，后者保持其认知作用域载体语义。

### `Verdict`

封闭对象：

| 字段 | 存在性 | 类型 | 契约 |
|------|--------|------|------|
| `outcome` | 必填 | 非空开放字符串 | 可移植核心结果；产品定义取值，读者自行解释 |
| `confidence` | 可选 | `[0, 1]` 内的有限数 | 核心置信值；非有限数拒绝 `INVALID_INPUT` |
| `detail` | 可选 | 成员值不受约束的 JSON 对象 | 产品专用评估内容；对信封校验器不透明，并纳入规范化 |

`verdict.detail` 的成员值可为对象、标量、数组或 `null`。其直接成员不得重复协议级信封位置：保留名为 `target`、`evidence`、`judgment_kind` 与 `outcome`，因此 `verdict.detail.target` 与 `verdict.detail.evidence` 无效。多个保留成员同时存在时，拒绝报告固定探测顺序 `target`、`evidence`、`judgment_kind`、`outcome` 中的第一个，且 `details.field` 携带 `verdict.detail.<name>`。`detail` 内部的产品专用成员（含嵌套对象）保持产品定义的形状。

### 开放词汇

`judgment_kind` 是开放字符串。核心取值记录在案，不由 `enum` 强制：

| 取值 | 含义 |
|------|------|
| `consistency` | 评估一致性 |
| `support` | 评估证据支撑 |
| `acceptance` | 评估接受性 |
| `review` | 记录评审结果 |

产品可添加命名空间的种类，如 `my-product:belief-assessment`。未知非空值有效且无需注册表更新。`verdict.outcome` 与 `JudgmentRef.kind` 按同一规则属于开放词汇。

### 线上通用规则

- **非空字符串。** 非空指在原始线上字符串上度量的 JSON Schema `minLength: 1`；仅含空白的字符串对本契约中每个开放字符串都是有效的非空线上值。
- **互操作整数域。** `schema_version`、`target.revision` 与每个 `evidence[].revision` 采用 `[1, 2^53-1]` 内的整数，即同时满足 u64 安全与 IEEE-754 binary64 安全的 JSON 数；schema 以 `minimum: 1` 与 `maximum: 9007199254740991` 钉住该域。`1.0` 这类整数拼写表示其所拼写的整数。非整数或域外值在该成员的字段路径返回 `INVALID_INPUT`。
- **`created_at` 偏移与原始匹配。** `created_at` 是带显式 UTC 偏移（`Z` 或 `±hh:mm`）的 RFC 3339 `date-time`；无偏移时间戳在 `created_at` 返回 `INVALID_INPUT`。语法按收到的原始线上字符串精确匹配：前后空白返回 `INVALID_INPUT`；任何非 ASCII 字符 —— 包括偏移中的 `U+2212` MINUS SIGN —— 返回 `INVALID_INPUT`；ASCII `T`/`t` 分隔符与 `Z`/`z` 后缀按 RFC 3339 保持接受。闰秒 `23:59:60` 拼写保持接受，无效日历日保持拒绝。
- **扩展映射。** `extensions` 遵循共享 `ExtensionMap` 语法：namespace 键匹配 `^[a-z][a-z0-9_-]*$`，每个值为对象。不合规条目在违规成员处以 `details.field` 返回 `INVALID_INPUT`；多个失败条目报告按 Unicode 码点字典序的首个违规者，跨栈确定。检查只读取 namespace 键与值种类；映射子项保持不透明。
- **线上成员可枚举性。** 只有对象自身的可枚举属性是线上成员。不可枚举的自有属性绝不进入线上，原型成员同样被排除在本契约命名的每个成员枚举之外，包括对 `verdict.detail` 的保留名探测。

### `validateJudgment` / `validate_judgment`

TypeScript 暴露 `validateJudgment(judgment: Judgment): SpokeResult<void>`；Rust 暴露行为完全相同的 `validate_judgment(judgment: &Value) -> SpokeResult<()>`。该辅助函数是确定性的且不做 I/O，两套语言都校验未解码的线上值。

`validateJudgment` 检查：

1. 每个必填字段的存在性与线上形状，包括封闭的 `Judgment`、`JudgmentRef` 与 `verdict` 对象，以及必填 `extensions` 映射。
2. `schema_version` 为互操作整数域内 ≥ 1 的整数；`judgment_id` 与 `judgment_kind` 为非空原始字符串。
3. `target` 与 `evidence` 引用形状：非空原始 `kind` 与 `id`，存在时非空原始 `namespace`，以及适用时互操作整数域内的正整数修订。
4. `evidence` 的存在性（含有效的空数组情形），以及每个证据项上互操作整数域内必填的正整数 `revision`。
5. `verdict.outcome` 为非空原始字符串；可选 `verdict.confidence` 为 `[0, 1]` 内的有限数；可选 `verdict.detail` 为不含保留直接成员的对象，按固定顺序探测。
6. `extensions` 条目符合共享 `ExtensionMap` 语法，并按字典序报告首个违规者。
7. 可选 `basis`、引用修订与可选 `created_at` 符合其声明的线上类型与约束。

该辅助函数不判定结论真伪、证据存在性或内容正确性、授权、业务阈值或产品专用 `detail` 形状；其保留成员检查只保护协议字段位置。解码为生成类型载体只是表示：已解码的 `Judgment` 在通过类型化校验门（Rust `validate_judgment_typed`、其 TypeScript 对应物，或本线上形状门）后才有效，线上整数域与每条形状规则都在该门处执行。

### 规范比较

规范相等即线上形状相等。`judgmentsEqualCanonical(left, right)`（Rust `judgments_equal_canonical`）比较线上 JSON —— 生产者发出或校验器接受的那份 JSON —— 的 RFC 8785 规范序列化，而非其任何类型化解码的规范序列化，并返回 `SpokeResult<bool>`。任一值无法规范化时拒绝 `INVALID_INPUT`。

- 同一 `Judgment` 的两个独立生产者在 RFC 8785 规范 JSON 下产生逐字节相同的结果。
- 线上存在性对比较有意义：显式为空的 `verdict.detail: {}` 是独立成员，与缺省 `detail` 比较不相等。
- `created_at` 按其确切线上拼写比较，因此 `2026-06-30T23:59:59+08:00` 与 `2026-06-30T15:59:59Z` 比较不相等。
- 规范化覆盖完整信封，包括 `verdict.detail`、`extensions`、显式存在的空数组与空对象，以及所有存在的可选成员；`detail` 对信封保持不透明并按产出参与比较。
- 在接收生产者对象而非已解析线上 JSON 的栈上，比较先经引擎自身的 JSON 线上发射重新发出每个值；发出的成员 —— 仅自身可枚举、`toJSON` 按成员键解析、数组空洞为 `null` —— 正是线上读取方所解析的内容。

## 拒绝词汇（Refusal vocabulary）

每个形状违规都返回 `INVALID_INPUT`，`details.field` 设为所报告的字段路径 —— 取值违规为违规成员，未知成员为所在封闭对象；`field` 是这些拒绝中 `details` 唯一由协议要求的成员，人类可读的 `message` 不携带稳定机器语义。判定校验不引入新的拒绝代码。

| 条件 | `details.field` |
|------|-----------------|
| 成员缺失、类型错误或约束失败 | 违规成员的逻辑路径：点分成员与方括号数组下标，例如 `judgment_kind`、`target.revision`、`evidence[0].revision`、`verdict.outcome`、`verdict.confidence` |
| 封闭对象形状之外的成员 | 所在封闭对象的路径：`judgment`、`target`、`evidence[n]` 或 `verdict` |
| `verdict.detail` 中的保留直接成员 | `verdict.detail.<name>`，其中 `<name>` 是固定探测顺序 `target`、`evidence`、`judgment_kind`、`outcome` 中首个存在的保留成员 |
| 不合规的 `extensions` 条目 | `extensions.<namespace>`，即按字典序的首个违规者 |
| 畸形 `created_at` | `created_at` |

共享 `SpokeResult` / `SpokeRejectCode` 面承载拒绝信封；`INVALID_INPUT` 是其共享的形状违规代码。

## 使用约束（Usage constraints）

- **仅数据与校验器一致性。** 该能力不定义引擎、推理器、存储行为或 Connect 操作。判定如何产出、存储或被使用由产品拥有。
- **复用既有 `tools.*` 传输。** 载荷携带 `Judgment` 的工具由其 `ToolDescriptor` 在 `HostCapabilityManifest.tools[]` 中声明，并经既有不透明 `{ "arguments": ... }` / `{ "result": ... }` 工具载荷调用。每个 `tools.<ns>.<tool_id>` 操作按其完整操作字符串在 `negotiated_capabilities` 中的精确成员资格门控 —— 例如 `tools.mindcore.deliberate` 由 `tools.mindcore.deliberate` 门控。`spoke-judgment` 对任何 `tools.*` 操作既不是替代物，也不是额外分派门禁。
- **可移植信封，产品内容。** `basis` 是产品提供的理由，不替代 `evidence`。超出可移植核心的产品评估内容属于 `verdict.detail`（绝不使用保留直接成员名）或 `extensions.<ns>`。
- **证据钉住修订。** `evidence` 始终存在，空数组合法，每个证据项携带必填的正整数 `revision`；目标的 `revision` 可选，存在时为正。每个整数成员都保持在互操作 `[1, 2^53-1]` 域内。
- **开放词汇原样往返。** `judgment_kind`、`verdict.outcome` 与 `JudgmentRef.kind` 原样往返；未知值有效且无需注册表更新。
- **跨栈一致。** TypeScript 与 Rust 辅助函数在包锁步 SemVer 下保持行为一致 —— 字段路径字符串、拒绝代码与规范比较语义相同。

## 相关页面

- [协议参考](/zh/reference/protocol) —— 能力标志与 schema 清单。
- [数据模型参考](/zh/reference/data-model) —— `HostCapabilityManifest`、`ToolDescriptor` 与 `ExtensionMap`。
- [操作线上（Ops wire）参考](/zh/reference/ops) —— 共享 `SpokeResult` / 拒绝信封。
- [connect 参考](/zh/reference/connect) —— 工具传输与精确字符串分派。
