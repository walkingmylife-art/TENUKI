# TENUKI Principles

この文書は、作業計画ではなく、TENUKI の判断原則を置くための文書である。

## 0. 目的

この文書は active plan ではない。

この文書は、TENUKI の判断をどう見るかを定義する。

- 何が authority か
- 何が observation か
- 誰が adopt するか
- 誰が commit するか
- downstream code が何を決めてよいか
- downstream code が何を決めてはいけないか

現在のソースコードと現在のユーザー指示が primary evidence である。

古い plan、古い comment、古い memory、過去の成功、既存ファイル、検出された candidate、leftover artifact は、責任を持つ層が adopt / commit するまでは observation である。

この文書に phase plan、temporary status、current task list を置かない。

## 1. 基本原則

Observation is not authority.

値は、責任を持つ層が current evidence から adopt し、effective value として commit した時だけ authority になる。

Downstream code は authority を infer / replace / repair / recommit してはいけない。

Downstream code は、すでに commit された authority を read / validate / use / materialize してよい。

## 2. 用語

### Authority

判断経路がその判断を所有し、current evidence を持ち、その判断が失敗した時に responsibility が戻るもの。

### Observation

adopt 前の情報。

例:

- saved config values
- existing files
- detected candidates
- previous successful runs
- logs
- old plans
- old comments
- old memories
- legacy paths
- session UI state
- generated previews
- leftover artifacts

### Adopt

責任を持つ層が、理由を持って observation を使う値として選ぶこと。

### Commit

責任を持つ層が adopted value を effective value として persist / finalize すること。

### Derived artifact

authority から生成された artifact。

それ自体は authority ではない。

例:

- dict.bin generated from dict.txt
- cache
- sidecar state
- preview data
- scan results

### 派生入れ物

source / text / request / TXT / fragment などから作られた中間変数、派生値、内部 index、一時構造、別名の値。

それ自体は authority ではない。

例:

- key
- value
- normalized_key
- source_norm
- model_input
- cleaned
- translated
- source_span
- pattern
- replacement
- index
- cache entry
- persist entry

派生入れ物は、存在しているだけでは正当性を持たない。

処理があることは、派生入れ物の正当性の根拠ではない。

派生入れ物があることは、その処理の正当性の根拠ではない。

### Consumer

commit 済み authority の downstream user。

Consumer は committed authority を read / validate / use / materialize してよい。

Consumer は authority を infer / select / replace / repair / recommit してはいけない。

## 3. 変更前に確認すること

コードを変更する前に、以下を確認する。

1. ここでの authority は何か
2. observation にすぎないものは何か
3. その判断を支える current evidence は何か
4. どの層が adopt するか
5. どの層が commit するか
6. 失敗時の responsibility はどこへ戻るか
7. downstream repair ではなく入口で保証できるか
8. consumer に selection / repair / provision / save / commit responsibility を渡していないか
9. source / text / request / TXT / fragment から作られる派生入れ物は何か
10. 派生入れ物が後段判断に使われていないか
11. 派生入れ物が元の authority source の代役になっていないか
12. 撤去対象の派生入れ物を、既存構造として保護していないか

## 4. 実装方向

Authority decisions は flow の入口に近い場所へ置く。

Orchestration は worker の一層上に置く。

Worker の責務は narrow に保つ。

main.rs は最終的に薄くしてよい。

ただし、未整理の authority decision を、行数削減だけを理由に散らしてはいけない。

責務境界で分割する。ファイルサイズだけで分割しない。

Handler は request / response shape を扱う。

Handler は core translation logic を重複実装しない。

Route differences は route policy、request shape、response shape として表現する。

意味が同じ場所では translation core を共有する。

Existing code style is not authority.

Legacy style や mid-layer style を、存在しているという理由だけで新コードへコピーしない。

既存コードにある派生入れ物は、それだけでは current specification の証拠にならない。

Small local fix は許可される。

良い local fix は、コードを最終的な責務境界へ近づける。

悪い local fix は、failure を隠す、誤った境界を保護する、新しい hidden authority を作る。

悪い local fix は、撤去対象の派生入れ物を保護対象に変えることでも起きる。

## 5. Startup and Config Rules

launcher_config.toml is the authority for launcher, backend, model, and llama-server conditions.

config.toml is the authority for runtime translation, UI, and TENUKI entry-server settings.

Normal mode に入る前に、config.toml は startup preflight path で current-shape loadable でなければならない。

同じ runtime config preflight は、backend startup 直前にも実行してよい。

check_ready は readiness only である。

check_ready は config shape を repair してはいけない。

Installed pass、download pass、verify、commit は分ける。

Authority backend は installed なら試してよい。

Authority backend であることは、それだけで download permission にはならない。

## 6. Dictionary and Slot Rules

dict_slot must be selected, provisioned, and committed upstream.

backend は language、leftover path、existing directory から別の dict_slot を infer / select してはいけない。

consumer は committed dict_slot path を materialize してよい。

Committed path の materialize は authority selection ではない。

dict.txt は slot の dictionary authority である。

Tenuki.dict.txt と Tenuki.regex.txt は、現在の slot dictionary files である場合、人間が編集する dictionary authority file である。

dict.bin は derived artifact である。

dict.bin は current dict.txt authority から生成されなければならない。

dict.bin は、current slot authority から再生成されない限り、slot / language changes を跨いで再利用してはいけない。

TranslationCache は session state である。

NewEntriesCache は session state である。

TranslationCache / NewEntriesCache は dict_slot や language boundary を越えてはいけない。

Dictionary TXT、dict.bin、TranslationCache、NewEntriesCache を同じ層として扱ってはいけない。

Dictionary TXT は editable source である。

dict.bin は generated lookup artifact である。

TranslationCache は same-session hit state である。

NewEntriesCache は pending save state である。

Legacy slot names such as S_0001 or s_0001 are observations for migration, normalization, or collision avoidance only.

They are not current naming rules.

New code must create current slot names only.

## 7. Route and Side Effect Rules

/translate and /list may share the translation pipeline.

They must not share side effects blindly.

/translate may use dictionary lookup, session cache, dictionary events, statistics, and input analysis according to normal translation policy.

/list must not update dictionary, cache, committed dictionary authority, or input analysis.

List output directory creation is output placement.

It must not change committed dictionary authority.

Route policy may differ.

Core translation logic should not be duplicated per route.

## 8. File Translate Rules

File Translate is an independent feature.

It borrows the existing UI stage and translation transport.

It is not normal translation.

It is not a reason to reshape the whole UI.

It is not a reason to weaken existing authority boundaries.

UI state is session state, not authority.

Preview is observation, not authority.

Scan results are observation, not authority.

Execution inputs should become fixed through readiness or RunPlan.

After that, workers should consume the fixed plan rather than rediscovering decisions.

Creating an output path from an accepted RunPlan is allowed.

That is materialization of the plan, not authority invention.

## 9. Legacy and Rescue Rules

Legacy detection is allowed.

Legacy detection is observation.

Legacy input may be normalized into current rules when the responsible layer explicitly adopts that normalization.

Rescue is allowed only when:

- the observation being rescued is identified
- the adopt reason is explicit
- the responsible layer owns the commit
- failure responsibility is clear
- the rescue does not become a hidden permanent workaround

Do not let compatibility code look like current specification.

Keep current rules and legacy normalization visibly separate in comments and tests.

## 10. Comment and Naming Rules

Comments should describe current contracts, not history.

Do not preserve old comments that describe old behavior as if it were current.

Comments may be short, but they must clearly separate:

- authority
- observation
- derived artifact
- 派生入れ物
- current rule
- legacy compatibility
- materialization
- commit

Avoid broad terms when the code means a specific component.

For dictionary code, distinguish:

- dict.txt authority
- dict.bin derived artifact
- TranslationCache
- NewEntriesCache
- committed dict_slot
- list output directory

For text processing, distinguish:

- encoding
- surface protection
- model transport
- restoration
- display normalization

For derived containers, distinguish:

- why the container exists
- what source it was derived from
- what it is allowed to decide
- whether it is only temporary
- whether it is allowed to reach lookup / register / persist / render

Do not let a useful-looking name make a container look more authoritative than it is.

## 11. Review Rule

Judge changes from the desired production shape, not from local convenience.

When reviewing or proposing a fix, name:

- authority
- observation
- adopt point
- commit point
- downstream consumer
- side effects
- 派生入れ物 created or removed

If something works but is structurally wrong, say so.

If a small fix is enough, prefer the small fix.

If the small fix preserves the wrong boundary, say that too.

If a change adds a derived container, check whether the container is necessary from the whole flow, not only from the local function.

If a change removes a derived container, check whether any old path still protects it as existing behavior.

Do not accept “the processing was added” or “the responsibilities are separated” as proof by itself.

Check what container the processing uses for decisions.

## 12. One Line Summary

Observation is only material for adoption.

Only values adopted and committed from current evidence may flow downstream as authority.

派生入れ物は、正しさの証拠ではなく、確認対象である。