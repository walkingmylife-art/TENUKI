BACKEND_AUTHORITY_CONTRACTS.md
Backend Authority Contracts

この文書は、backend の public authority boundary を記録する。

これは active plan ではない。

これは過去フェーズの作業ログではない。

現在のソースコードと現在のユーザー指示が、この文書より優先される。

0. 目的

backend がどの authority を読むか、どの authority を作らないかを明確にする。

特に、以下を混ぜない。

config authority
launcher authority
dict_slot authority
input analysis authority payload
stale replay
list mode output placement
normal translation side effect
1. Processor Boundary

src/backend/processor.rs は backend live path に存在しない。

backend は、input analysis のために processor abstraction を持たない。

次のものは backend live path の authority ではない。

ProcessorFactory
TextProcessor
TranslationContext
NormalTextProcessor
GameTextProcessor
InputAnalysisProjector

input analysis は processor から再計算しない。

2. Mode Boundary

Config.mode は runtime/config contract として残る。

game / normal の mode concept は残る。

mode は config/runtime policy と caller behavior の責任で扱う。

mode handling を shared backend processor module へ戻さない。

legacy mode value の normalize は config 層の責任である。

3. Config Authority

backend startup は、committed config.toml と launcher_config.toml を読む。

backend startup は、それらを observation から勝手に作り直さない。

backend は、launcher authority を推測し直さない。

backend は、model authority を filename だけから再構築しない。

backend は、committed config を読む側である。

必要な adopt / save command がある場合だけ、その command の責任範囲で保存する。

4. dict_slot Authority

FrontendCommand::SetLanguagePair.dict_slot は、UI/preflight 側で解決済みの authority として backend へ渡される。

backend はそれを adopt して config.toml に保存し、reload / restart する。

backend は SetLanguagePair 内で別の dict_slot を infer しない。

backend は missing slot authority を discovery から repair しない。

backend は discovery を authority として扱わない。

SetDictSlot は、dict_slot だけを変更する command である。

SetLanguagePair と SetDictSlot を同じ意味にしない。

5. Input Analysis Snapshot Contract

fresh snapshot は、successful normal /translate completion で記録された authority payload から作る。

stale snapshot は、保存済み latest snapshot を clone し、result_stale = true にした replay である。

stale replay は、現在の mode / game-text options / language settings / processor から再計算しない。

InputAnalysisSnapshot の意味。

raw_text:
normal /translate 完了 payload の original request text

extracted_text:
translation pipeline が analysis source として記録した text

visible_text:
authority payload に記録された human-readable source view

model_inputs:
完了した翻訳で観測された model call inputs

final_output:
fresh snapshot の final translated output
stale replay では保存済み値を保持する

result_stale:
fresh では false
mode / language / game-text 変更後の replay では true

dict_hits:
完了した翻訳で記録された dictionary hit count

model_calls:
完了した翻訳で記録された model call count
6. Module Contracts
backend/analysis.rs

責任。

CompletedAnalysisPayload から fresh InputAnalysisSnapshot を作る
latest completed snapshot を InputReplayState に保存する
stale display 用に保存 snapshot を replay する

責任ではないこと。

processor に依存する
mode / game_text / language から再計算する
backend/manager.rs

責任。

backend runtime を管理する
config reload / restart を扱う
mode / game_text / language change 時に stale replay を出す
dictionary reload を実行する

責任ではないこと。

input-analysis projector を持つ
processor を持つ
input analysis を再計算する

dictionary reload は real work である。

input analysis replay は saved snapshot replay である。

この2つを混ぜない。

backend/server.rs

責任。

/translate request を処理する
/list request を処理する
route ごとの PipelineBehavior を選ぶ
/translate 完了時に analysis authority payload を作る
/list side effect を抑制する

責任ではないこと。

/list で dictionary/cache/input analysis を更新する
processor で analysis を再計算する
main.rs

責任。

InputAnalysisSnapshot を UI 表示に使う
pickup / work-result display に snapshot を読む

責任ではないこと。

input analysis を再構築する
processor を使って snapshot を作る
7. Normal / List Boundary

normal /translate は、通常翻訳の entry である。

/list は、normal /translate とは別の backend entry である。

/list は以下をしない。

dictionary authority commit
translation cache update
input analysis update
normal statistics update
normal dict_slot authority update

File Translate / List output directory は、その run の execution location である。

File Translate / List output directory は、committed dict_slot authority ではない。

Continuation / resume state for List mode は、normal input analysis snapshot replay とは独立して扱う。

8. Restart / Reload Boundary

config change に対する reload / restart は backend manager の責任である。

translator-only restart と full restart は意味を分ける。

engine が必要な場合は backend manager が runtime 状態を見て決める。

UI command は、backend 内部の restart 実装詳細を直接所有しない。

9. 確認すること

backend authority 周りを変更した場合は、以下を見る。

その command は何を commit するのか
backend は read しているだけか、adopt/save しているのか
discovery を authority にしていないか
stale replay を fresh analysis として扱っていないか
/list side effect が normal に近づいていないか
dict_slot と output placement を混ぜていないか
processor / projector 的な再計算経路を戻していないか
10. One-line Summary

backend は committed authority を読み、必要な command の範囲で adopt / save する。

input analysis は completed translation payload から作り、stale は replay する。

/list は normal translation の side effect を持たない。