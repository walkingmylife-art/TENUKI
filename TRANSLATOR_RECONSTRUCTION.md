# TRANSLATOR_RECONSTRUCTION.md

# TENUKI Translator Reconstruction

この文書は、`src/backend/translator/` の責務境界を整理するための translator 専用文書である。

これは一般原則ではない。

現在のソースコードと現在のユーザー指示が、この文書より優先される。

この文書は、過去事故の一覧ではなく、translator を見るための処理境界を置く。

## 0. 目的

translator core を、局所修正の積み上げではなく、次の責務へ分けて見る。

```text
request text
-> source structure
-> planned document
-> resolved document
-> rendered document
-> final display text
-> TranslationResult

目的は、翻訳結果を作ることだけではない。

目的は、以下を混ぜずに翻訳結果を作ること。

source authority
model transport
model return
resolved child text
rendered response
display wrap
persist candidate
1. 最優先の境界

FragmentAuthority.source が、fragment の source authority である。

FragmentAuthority.source は、次の基準になる。

dictionary lookup source
register source
exact persist source
regex persist source basis
logs / stats / diagnostics source basis

model_input は authority ではない。

restored_output は authority ではない。

ResolvedFragmentNode.text は response ではない。

rendered String は source authority ではない。

TranslationResult.text だけが、render 後の最終出力である。

2. 正規フロー

public path は次の形を基本にする。

translate_chunk
-> plan_document
-> resolve_document
-> render_document
-> TranslationResult

現在の実装上、source parse が plan_document 内部に含まれていてもよい。

重要なのは、責務が混ざらないこと。

plan

plan_document は、翻訳対象と surface を分ける。

許可すること。

source text を構造として読む
line / segment / separator / newline を保持する
tag / protected angle / bracket / punctuation などを surface として保持する
翻訳対象 core を FragmentAuthority.source として確定する

しないこと。

dictionary lookup
model call
ZM transport
persist candidate 作成
final response String 作成
translated text の再 tokenize
resolve

resolve_document は、planned fragment を解決する。

許可すること。

FragmentAuthority.source で lookup する
lookup miss の場合だけ model transport を作る
model input にだけ ZM -> number を使う
model output を restore / cleanup する
ResolvedFragmentNode.text に child text を入れる
persist candidate を作る
logs / stats を集める
surface をそのまま通す

しないこと。

final response String を作る
render する
wrap する
render 済み String を lookup / register / persist に戻す
model_input を lookup / register / persist source にする
restored_output を lookup / register source にする
render

render_document は、resolved document から final response String を作る。

許可すること。

ResolvedDocument の親構造をたどる
surface を元位置へ戻す
ResolvedFragmentNode.text を元の fragment 位置へ戻す
line / segment / separator / newline を復元する
final response String を作る
display wrap を適用する

しないこと。

dictionary lookup
model call
persist candidate 作成
source parse
FragmentAuthority 作成
render 後 String を lookup / register / persist に戻す
3. 型の責任
FragmentAuthority
struct FragmentAuthority {
    source: String,
}

責任。

fragment の source authority を持つ
lookup / register / persist の source basis になる

責任ではないこと。

model_input を持つ
rendered output を持つ
prefix / suffix を後から削る
表示用の加工値を authority 化する
PlannedDocument
struct PlannedDocument {
    lines: Vec<PlannedLine>,
}

責任。

source structure を保持したまま、surface と fragment を分ける

責任ではないこと。

翻訳する
辞書登録する
最終文字列を作る
ResolvedDocument
struct ResolvedDocument {
    lines: Vec<ResolvedLine>,
}

責任。

planned structure を保持したまま、fragment の child text を持つ

責任ではないこと。

response String そのものになる
source authority を作り直す
TranslationResult
struct TranslationResult {
    text: String,
    new_entries: Vec<NewTranslationEntry>,
    stats: TranslationStats,
    logs: Vec<LogEvent>,
}

責任。

外部へ返す final text
new entries
stats
logs

TranslationResult.text は render 後の final output である。

4. ZM の責任

ZM 処理は、次の2つに分けて見る。

model transport

model が placeholder を壊さないように、一時的に ZM を数字へ置き換える。

これは model input 専用である。

source authority -> model_input
model_input -> model
model output -> restored_output

model_input は dictionary key ではない。

model_input は register key ではない。

model_input は exact persist source ではない。

regex persist candidate

ZM を含む source を、必要なら regex persist candidate にする。

regex persist candidate は exact persist と同じ意味ではない。

regex persist candidate は、raw exact cache insert の代替名ではない。

regex と exact は、commit 側でも意味を分ける。

5. cache / dictionary / persist

lookup source は FragmentAuthority.source を基準にする。

cache source hit と value observation は意味を分ける。

value observation は source authority ではない。

persist candidate は、resolve 中に作ってよい。

commit は backend/server 側の責任である。

translator は、commit 済み dictionary authority を直接変更する場所ではない。

6. render / wrap

render は、ResolvedDocument から final response String を作る責任を持つ。

wrap は final display text に対する処理である。

wrap は source parse ではない。

wrap は lookup / register / persist source を作らない。

protected angle / newline / visible surface は、render stage で元位置へ戻す。

7. /translate と /list

normal /translate と /list は side effect policy が違う。

translator core は再利用してよい。

ただし、route policy は混ぜない。

/list は dictionary / cache / input analysis を更新しない。

normal translation の side effect を /list に漏らさない。

8. 確認すること

translator 変更時は、まず以下を見る。

今回の変更は plan / resolve / render / persist / route policy のどこか
その関数の責任は何か
source authority はどこか
model transport を authority にしていないか
rendered String を source 側へ戻していないか
child text を response として扱っていないか
/list と /translate の side effect を混ぜていないか
9. One-line Summary

translator は、source authority、model transport、resolved child text、rendered response を混ぜない。

FragmentAuthority.source で解決し、render_document でだけ final response String を作る。