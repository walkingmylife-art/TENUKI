# TENUKI Translator Reconstruction

この文書は、`src/backend/translator.rs` / `src/backend/translator/` の現在作業用設計である。

これは一般原則ではない。

現在のソースコードと現在のユーザー指示が、この文書より優先される。

## 0. 目的

局所修正を積み上げず、translator core を以下の責務へ分ける。

```text
REQUEST
-> source parse
-> plan document
-> resolve document
-> render document
-> final display wrap
-> TranslationResult
```

目的は、翻訳結果を作ることではなく、次を守りながら翻訳結果を作ること。

* Fragment is not response.
* child.text is not response.
* model transport is not authority.
* rendered text is not authority.
* Only render_document may build the final response String.

処理が入ったこと、型が分かれたこと、テストが通ったことだけでは完了ではない。

translator 内で作る派生入れ物は、何から作られ、どの stage でだけ使ってよいかを確認する。

ここでいう派生入れ物とは、`source` から作られる `model_input`、`restored_output`、`ResolvedFragmentNode.text`、`PersistCandidate`、render atom などの中間値・一時構造を指す。

派生入れ物は、存在しているだけでは正しさの根拠にならない。

## 1. 現在の最重要方針

旧 signed-ZM key split を復活させない。

以下のような辞書前 key 加工を復活させない。

```text
HP-ZMDZ% -> HP
ATK+ZMCZ% -> ATK
+ZMDZ%压制 -> 压制
```

現在の authority はこれ。

```text
FragmentAuthority.source
```

`FragmentAuthority.source` は以下の基準になる。

* dictionary lookup key
* dictionary register source
* exact persist source
* regex persist source basis
* logs / stats / diagnostics source basis

`ZM -> number` は model transport 専用である。

```text
FragmentAuthority.source     = 原文 fragment authority
ModelTransport.model_input   = model に渡す一時文字列
ModelReturn.restored_output  = model 出力を表示・persist 候補へ戻した文字列
RenderedDocument.text        = response 文字列
```

これらを混ぜない。

`model_input`、`restored_output`、`rendered String` を lookup / register / persist source に戻さない。

## 2. 正規フロー

最終的な public path は以下。

```text
translate_chunk
-> parse_source_document
-> plan_document
-> resolve_document
-> render_document
-> wrap_final_display_text
-> TranslationResult
```

`resolve_document` の内部で、必要な fragment ごとに以下を行う。

```text
FragmentAuthority.source
-> lookup
-> miss の場合 model transport 作成
-> model call
-> restore model output
-> clean display output
-> PersistCandidate 作成
-> ResolvedFragmentNode.text に格納
```

`resolve_document` は final response String を作らない。

## 3. 目標ファイル構成

最終形。

```text
src/backend/translator/
  mod.rs        public API / coordinator
  types.rs      Source / Planned / Resolved / Outcome types
  tokenize.rs   source parse only
  plan.rs       SourceDocument -> PlannedDocument
  zm.rs         model transport ZM helpers
  resolve.rs    lookup / model / clean / persist candidate / stats / logs
  persist.rs    Exact / Regex candidate helpers
  render.rs     ResolvedDocument -> String and final wrap
  normalize.rs  display cleanup and wrap helpers
  tests/
```

既存の `client.rs`、`lang.rs`、`cache.rs`、`helpers.rs`、`normalize.rs` がある場合は、無理に作り直さない。

まず責務境界を合わせる。

ファイル名は最終形に合わせてよいが、分割だけを目的にしない。

## 4. 目標型

### 4.1 Source

```rust
struct SourceDocument {
    lines: Vec<SourceLine>,
}

struct SourceLine {
    segments: Vec<SourceSegment>,
    separators: Vec<SurfaceNode>,
    newline: Option<SurfaceNode>,
}

struct SourceSegment {
    tokens: Vec<StructureToken>,
}

enum StructureToken {
    Text(String),
    Surface(SurfaceNode),
    Bracket {
        open: SurfaceNode,
        inner: Vec<StructureToken>,
        close: SurfaceNode,
    },
}
```

### 4.2 Planned

```rust
struct PlannedDocument {
    lines: Vec<PlannedLine>,
}

struct PlannedLine {
    segments: Vec<PlannedSegment>,
    separators: Vec<SurfaceNode>,
    newline: Option<SurfaceNode>,
}

struct PlannedSegment {
    nodes: Vec<PlannedNode>,
}

enum PlannedNode {
    Surface(SurfaceNode),
    Fragment(FragmentNode),
}

struct SurfaceNode {
    text: String,
}

struct FragmentNode {
    authority: FragmentAuthority,
}

struct FragmentAuthority {
    source: String,
}
```

`FragmentNode` に `dict_key`、`prefix`、`suffix` を持たせない。

旧 signed-ZM split を実装する場所を作らない。

### 4.3 Model Transport

```rust
struct ModelTransport {
    authority: FragmentAuthority,
    model_input: String,
    zm_map: ZmTransportMap,
}

struct ModelReturn {
    raw_output: String,
    restored_output: String,
}
```

`model_input` は authority ではない。

`restored_output` も authority ではない。

### 4.4 Resolved

```rust
struct ResolvedDocument {
    lines: Vec<ResolvedLine>,
}

struct ResolvedLine {
    segments: Vec<ResolvedSegment>,
    separators: Vec<SurfaceNode>,
    newline: Option<SurfaceNode>,
}

struct ResolvedSegment {
    nodes: Vec<ResolvedNode>,
}

enum ResolvedNode {
    Surface(SurfaceNode),
    Fragment(ResolvedFragmentNode),
}

struct ResolvedFragmentNode {
    authority: FragmentAuthority,
    text: String,
    origin: ResolveOrigin,
}

enum ResolveOrigin {
    Dictionary,
    CacheSource,
    CacheValueObservation,
    Model,
}
```

`ResolvedFragmentNode.text` は child.text である。

response ではない。

### 4.5 Persist Candidate

```rust
enum PersistCandidate {
    Exact {
        authority: FragmentAuthority,
        value: String,
    },
    Regex {
        authority: FragmentAuthority,
        pattern: String,
        replacement: String,
    },
}
```

Regex persist は raw exact cache と同じ扱いにしない。

`PersistCandidate::Regex` は「この source を exact cache に入れる」意味ではない。

### 4.6 TranslationResult

```rust
struct TranslationResult {
    text: String,
    stats: TranslationStats,
    new_entries: Vec<NewTranslationEntry>,
    logs: Vec<LogEvent>,
    diagnostics: TranslationDiagnostics,
}
```

`TranslationResult.text` は必ず `render_document` 後の final output である。

## 5. Stage Contracts

### 5.1 Source Parse

Input:

```text
request text
```

Output:

```text
SourceDocument
```

Allowed:

* newline を line boundary として保持
* separator を parent line に surface として保持
* bracket open / close を surface として保持
* text token を保持
* protected tag / placeholder / escaped sequence を surface として保持

Forbidden:

* FragmentNode を作る
* dictionary lookup
* model call
* ZM -> number transport
* persist candidate 作成
* render String 作成

### 5.2 Plan

Input:

```text
SourceDocument
```

Output:

```text
PlannedDocument
```

Allowed:

* 翻訳対象 text core を `FragmentAuthority.source` として確定
* protected surface を Surface として通す
* bracket open / close を Surface として通す
* bracket inner core を同じ親構造内で再帰 plan
* separator / newline を元の line / segment 位置へ保持
* edge punctuation / visible surface を Surface として保持

Forbidden:

* dictionary lookup
* model call
* ZM -> number transport
* persist candidate 作成
* render String 作成
* translated text の tokenize
* mixed bracket whole Fragment shortcut
* Fragment 同士の後結合
* `dict_key` / `prefix` / `suffix` の再導入

### 5.3 Resolve

Input:

```text
PlannedDocument
```

Output:

```text
ResolvedDocument + TranslationAccumulation
```

Allowed:

* `FragmentAuthority.source` で lookup
* lookup miss の場合だけ model transport を作る
* model input にだけ ZM -> number を使う
* model output を restore して `ResolvedFragmentNode.text` に格納
* PersistCandidate を作る
* logs / stats / diagnostics 材料を集める
* Surface をそのまま通す

Forbidden:

* final response String を作る
* line / segment 構造を flatten する
* child.text を response 扱いする
* render 済み文字列を source parser に戻す
* render 済み文字列を lookup / register key に戻す
* wrap する
* source parser を呼ぶ
* `model_input` を lookup / register / persist source にする
* `restored_output` を lookup / register source に戻す

### 5.4 Render

Input:

```text
ResolvedDocument
```

Output:

```text
String
```

Allowed:

* line by line で render
* segment by segment で render
* Surface.text を出力
* ResolvedFragmentNode.text を元の座席へ戻す
* separator / newline を元位置へ戻す

Forbidden:

* dictionary lookup
* model call
* FragmentAuthority 作成
* ModelTransport 作成
* PersistCandidate 作成
* source tokenize
* translated tokenize
* lookup / register / persist source を作る

### 5.5 Final Display Wrap

Input:

```text
final response String
```

Output:

```text
display response String
```

Allowed:

* final output surface にだけ wrap を適用
* 既存の wrap 候補仕様を維持する

Forbidden:

* source parse
* FragmentNode 作成
* FragmentAuthority 作成
* dictionary lookup
* model call
* ZM transport
* persist candidate 作成
* render 後 String を lookup / register / persist に戻す

## 6. ZM Policy

ZM 処理は2種類に分ける。

### 6.1 Model Transport ZM

model が ZM を壊さないように、一時的に ZM を数字へ置き換える。

これは model input 専用。

```text
source: 攻撃+ZMCZ%
model_input: 攻撃+2%
```

この `2` は authority ではない。

`攻撃+2%` を dictionary key / register key / exact persist source にしない。

### 6.2 Regex Persist ZM

ZM 付き fragment を長期的に育てるため、model output restore 後に regex persist candidate を作る。

例:

```text
authority.source: 攻撃+ZMCZ%
value: 攻撃+ZMCZ%
candidate: Regex { pattern, replacement }
```

Regex candidate は raw exact cache と同じではない。

Regex persist を作ったからといって、無条件に `TranslationCache.source_index` へ raw exact を入れない。

## 7. Cache / Persist Boundary

translator core は cache / dictionary を直接 commit しない。

translator core は以下を返す。

* TranslationResult.text
* PersistCandidate
* logs
* stats
* diagnostics

server 側が route policy に従って commit する。

### 7.1 `/translate`

Allowed side effects:

* dictionary lookup
* session cache lookup
* dictionary event
* statistics update
* input analysis update
* exact persist commit
* regex persist commit

### 7.2 `/list`

Forbidden side effects:

* dictionary authority update
* TranslationCache update
* NewEntriesCache update
* input analysis update
* committed dict_slot update

`/list` は translation transport を使ってよい。

ただし normal translation の side effect を共有しない。

## 8. Whole Fragment Policy

mixed bracket segment は暗黙に whole Fragment にしない。

標準処理は以下。

```text
outer text       -> Fragment
bracket open     -> Surface
bracket inner    -> recursively planned nodes
bracket close    -> Surface
separator        -> Surface at parent line
punctuation edge -> Surface
```

whole Fragment が必要な場合だけ、専用関数に隔離する。

```rust
fn maybe_plan_segment_as_whole_fragment(
    segment: &SourceSegment,
    context: WholeFragmentContext,
) -> Option<PlannedSegment>
```

初期実装では `None` を返してよい。

whole Fragment を有効化する条件は、次を全部説明できる場合だけ。

* dictionary key authority として妥当
* model input として妥当
* register key として妥当
* resolved child.text が戻る座席が1つに定まる
* old tests のためではない
* model call 削減のためだけではない

## 9. 派生入れ物の確認

translator 内の派生入れ物は、処理の正しさの根拠ではなく確認対象である。

特に以下を確認する。

* その派生入れ物は何から作られたか
* 何を捨てたか
* どの stage でだけ使うか
* 後段判断に使ってよいか
* `FragmentAuthority.source` の代役になっていないか
* 撤去対象の旧中間値が保護対象に変わっていないか

translator で特に危険な派生入れ物。

* dict_key
* prefix
* suffix
* model_input
* cleaned
* restored_output
* rendered String
* child.text
* source_span
* pattern
* replacement
* render atom

必要な派生入れ物は作ってよい。

ただし、使える stage と責務を超えてはいけない。

## 10. Implementation Phases

### Phase A: Type Boundary

追加または整理する。

* SourceDocument
* PlannedDocument
* ResolvedDocument
* FragmentAuthority
* ModelTransport
* PersistCandidate
* TranslationAccumulation

Done when:

* 型が compile する
* `dict_key` / `prefix` / `suffix` を新しい FragmentNode に持ち込んでいない
* public `translate_chunk` の外形はまだ維持してよい

### Phase B: Source Parse / Plan

追加する。

```rust
fn parse_source_document(text: &str, options: &GameTextOptions) -> SourceDocument
fn plan_document(source: SourceDocument) -> PlannedDocument
```

Done when:

* line / segment / separator / newline shape が保持される
* `FragmentAuthority.source` が fragment 単位で確定する
* mixed bracket segment が暗黙 whole Fragment にならない

### Phase C: Resolve Document

追加する。

```rust
fn resolve_document(...) -> (ResolvedDocument, TranslationAccumulation)
```

Done when:

* lookup は `FragmentAuthority.source` を基準にする
* model transport は miss の時だけ作る
* ZM -> number は model_input にだけ使われる
* PersistCandidate が Exact / Regex に分かれる
* resolve は final String を作らない

### Phase D: Render Document

追加する。

```rust
fn render_document(document: &ResolvedDocument) -> String
```

Done when:

* `TranslationResult.text` が `render_document` 由来になる
* child.text は response として扱われない
* separator / newline が元位置へ戻る

### Phase E: Final Wrap

追加または移動する。

```rust
fn wrap_final_display_text(text: String, settings: &TranslationSettings) -> String
```

Done when:

* wrap は final String にだけかかる
* wrap から source parse / lookup / model / persist へ戻れない

### Phase F: Remove Old Paths

削除または隔離する。

* flat final `Vec<PlannedNode>` response path
* old `resolve_plan(nodes: &[PlannedNode]) -> String` path
* old mixed bracket whole Fragment tests
* old signed-ZM key split tests
* trace のためだけの `dict_key` / `prefix` / `suffix` 分岐

Done when:

* `translate_chunk` が新順序のみを通る
* compatibility wrapper が残る場合、最終 response path では使われない
* 残存経路が完了報告に明記されている

## 11. Required Tests

### 11.1 Fragment Is Not Response

Input:

```text
的招式(消耗足部架势ZMCZ点)。
```

Assert:

* PlannedDocument has one line
* line has one segment
* segment nodes keep Fragment / Surface / Fragment / Surface / Surface shape
* ResolvedDocument still has line / segment shape
* final String appears only after render_document

### 11.2 child.text Is Not Response

Mock:

```text
的招式 -> その技
消耗足部架势ZMCZ点 -> 足の構えをZMCZポイント消費する
```

Assert:

```text
result.text == "その技(足の構えをZMCZポイント消費する)。"
```

Also assert:

* each translated fragment text remains child.text until render
* resolve_document does not create final response String

### 11.3 ZM Transport Is Not Authority

Input:

```text
攻撃+ZMCZ%
```

Assert:

* `FragmentAuthority.source` remains `攻撃+ZMCZ%`
* `model_input` may become `攻撃+2%`
* lookup / register / persist source does not become `攻撃+2%`
* lookup / register / persist source does not become `攻撃`

### 11.4 Regex Persist Does Not Create Raw Exact Cache

Input:

```text
負面抗性+ZMCZ%
```

Assert:

* Regex PersistCandidate may be produced
* raw source exact cache is not inserted through generic exact insert path
* next request can still test regex behavior instead of being masked by raw exact cache

### 11.5 Wrap Does Not Reparse

Assert or review that `wrap_final_display_text()` does not call:

* source parse
* tokenize_structure
* FragmentAuthority creation
* lookup
* model
* persist candidate creation

### 11.6 `/list` Side Effects Stay Off

Assert:

* `/list` does not update dictionary authority
* `/list` does not update TranslationCache
* `/list` does not update NewEntriesCache
* `/list` does not emit InputAnalysisUpdated

### 11.7 派生入れ物が authority にならない

Assert:

* `model_input` is not used as lookup / register / persist source
* `restored_output` is not used as lookup / register source
* `rendered String` is not passed back to source parse / lookup / register
* old `dict_key` / `prefix` / `suffix` containers do not exist in current FragmentNode
* tests do not protect removed containers as current specification

## 12. Tests To Replace, Not Preserve

Do not preserve tests whose only purpose is old behavior.

Replace or delete tests expecting:

* `mixed_bracket_segment_passes_as_single_fragment`
* `multi_bracket_mixed_sentence_with_outer_text_is_whole_fragment`
* `ATK+ZMCZ% -> ATK key`
* `HP-ZMDZ% -> HP key`
* `+ZMDZ%压制 -> 压制 key`
* `dict_key/prefix/suffix trace`
* `after_apply_zm_key_plan trace`

Replacement tests must prove current boundaries.

## 13. Completion Report Requirements

Report all of the following.

* Changed files
* Removed paths
* Remaining paths
* Changed tests
* Deleted / replaced tests
* New invariant tests
* Added derived containers
* Removed derived containers
* Remaining derived containers
* Unverified risks
* Not touched

Do not report only:

* cargo fmt
* cargo test passed

Passing tests are evidence, not completion.
