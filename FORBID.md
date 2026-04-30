# TENUKI Forbid

## 0. 目的

この文書は、破壊的な実装パターンを列挙する。

この禁止事項を、キーワードだけで機械的に適用してはいけない。

見るべきことは、コードが以下をしていないかである。

* authority を誤った層で変更していないか
* failure を隠していないか
* 責務を重複させていないか
* observation を committed truth として扱っていないか
* legacy behavior を current specification として保護していないか
* temporary rescue を permanent にしていないか
* 派生入れ物を、元 source / TXT / fragment の代替 authority として扱っていないか

Stable files には、principles、prohibitions、checklists を置く。

Stable files に active phase plan を置いてはいけない。

## 1. Authority 違反

Observation を authority として使ってはいけない。

見つかったファイルを、存在しているという理由だけで truth として扱ってはいけない。

saved value、previous success、old comment、old plan、old memory、legacy path、detected candidate を current authority として扱ってはいけない。

verification 前に effective value を commit してはいけない。

backend startup に config shape を repair させてはいけない。

downstream consumer に authority の infer / select / replace / repair / provision / save / recommit をさせてはいけない。

explicit adopt reason と commit path のない temporary rescue を permanent にしてはいけない。

「前回動いた」を current evidence として使ってはいけない。

authority config に backend 名があることだけを理由に download permission としてはいけない。

## 2. Route / Pipeline 違反

route ごとに core translation logic を重複実装してはいけない。

`/translate`、`/list`、future `/v1` のために、別々の hidden translation behavior を作ってはいけない。

commonization の名目で route policy differences を消してはいけない。

dummy success response で failure を隠してはいけない。

`/list` に dictionary、cache、committed dictionary authority、input analysis を更新させてはいけない。

List output directory creation に committed dictionary authority を変更させてはいけない。

request / response shape handling と core translation logic を混ぜてはいけない。

## 3. File / Output 違反

partial write を completed output として扱ってはいけない。

continuation / resume assumptions を壊す形で output file を truncate / delete してはいけない。

hardcoded authority path を追加してはいけない。

derived artifact を authority として扱ってはいけない。

dict.bin を dictionary authority として扱ってはいけない。

dict.bin は、current dict.txt authority から再生成されない限り、slot / language changes を跨いで再利用してはいけない。

output placement を dictionary authority にしてはいけない。

## 4. Legacy / Migration 違反

legacy style や mid-layer style を、新コードへそのままコピーしてはいけない。

old slot names such as `S_0001` or `s_0001` を current naming rules として扱ってはいけない。

legacy compatibility と current specification を、comments や tests で同じ階層に置いてはいけない。

old handoff MD、old active plans、old comments、old memories を、current source code と current user instruction より上位にしてはいけない。

obsolete plan text を stable instruction files に残してはいけない。

current principles と temporary project status を混ぜてはいけない。

legacy detection を、legacy が current ではないという理由だけで削除してはいけない。

legacy は observation として検出してよい。

legacy input は、current authority layer が明示的に adopt する場合だけ、current rules へ normalize / migrate してよい。

## 5. UI / Worker 違反

size-proportional I/O、parse、render work を UI thread で実行してはいけない。

localized text helpers が既にある場所で、user-facing strings を直接書いてはいけない。

rendering convenience を理由に UI に authority を決めさせてはいけない。

preview state を、explicit readiness または plan step なしに execution authority にしてはいけない。

worker results は、current target / source / generation / session と一致する場合だけ反映する。

responsibility boundaries が不明確なまま、file size 削減だけを目的に split してはいけない。

## 6. Comment / Naming 違反

history として comment を書いてはいけない。

old behavior を current behavior のように説明する comment を残してはいけない。

code が次のどれかを意味しているとき、広い意味の “dictionary” で曖昧にしてはいけない。

* dict.txt authority
* dict.bin derived artifact
* TranslationCache
* NewEntriesCache
* committed dict_slot
* list output directory

code が committed path を materialize する場合、“read only” と書いてはいけない。

必要なら、次のように書く。

`does not infer or commit authority; may materialize committed path.`

現在その exact component が存在しない場合、“pattern dictionary” と呼んではいけない。

encoding repair、text protection、model transport、output restoration を、1つの comment に混ぜてはいけない。

legacy behavior が current specification と同格に見える test name を付けてはいけない。

分かりやすそうな名前の派生入れ物を、存在しているという理由だけで保護対象にしてはいけない。

ただし、名前だけで禁止してはいけない。

禁止対象は、派生入れ物そのものではなく、その派生入れ物を根拠なく source / TXT / fragment の代替判断材料にすることである。

## 7. 派生入れ物による責務逃れ

ここでいう派生入れ物とは、元の source / text / request / TXT / fragment などから作られた中間変数、派生値、内部 index、一時構造、別名の値を指す。

例:

* key
* value
* normalized_key
* source_norm
* model_input
* cleaned
* restored_output
* source_span
* pattern
* replacement
* index
* cache entry
* persist entry

処理が入っていることを、派生入れ物の正当性の根拠にしてはいけない。

派生入れ物があることを、その処理の正当性の根拠にしてはいけない。

処理の分離を理由に作った派生入れ物を、全体 flow からの根拠なしに後段判断へ使ってはいけない。

撤去対象の派生入れ物を、既存構造・既存仕様として保護対象に昇格させてはいけない。

全体 flow が持つ責務を、局所の派生入れ物で再実装してはいけない。

未指定の normalize / trim / sanitize / canonicalize 結果を、source / TXT / fragment の代替として lookup / register / persist / covered / conflict 判定に使ってはいけない。

辞書編集面に見えている source/value と runtime lookup 対象を、明示理由なしに別の派生値へすり替えてはいけない。

model transport、display cleanup、render/wrap 用の派生入れ物を、dictionary key / cache key / persist source に戻してはいけない。

## 8. Operational 違反

implementation 後の alignment step を skip してはいけない。

tests pass で止めてはいけない。

naming、comments、side effects、entry flow、派生入れ物の扱いが current design と矛盾している場合、tests pass でも完了扱いしてはいけない。

old phase plans を task authority として使ってはいけない。

temporary implementation note を stable instruction にしてはいけない。

rescue が必要な場合は、以下を明記する。

* どの observation が見つかったか
* なぜ adopt するか
* 誰が commit するか
* failure 時にどうなるか

大きめの変更では、処理だけでなく、追加 / 削除 / 残存した派生入れ物も報告する。

## 9. 禁止ではないもの

authority boundary が既に固定されている場合、以下は禁止ではない。

* committed path を materialize すること
* committed path を validate すること
* current authority から derived artifact を regenerate すること
* legacy paths を observation として detect すること
* current authority layer が明示的に adopt した legacy input normalization
* accepted RunPlan から output path を作ること
* route policy differences を保つこと
* final responsibility boundary へ近づける small local fix
* responsibility boundary が明確になるまで file splitting を遅らせること
* 必要な中間変数、内部 index、一時構造、cache、pattern、span を作ること

これらを、禁止パターンに名前が似ているという理由だけで止めてはいけない。

実際に確認するべきことは、それが authority を変えているか、failure を隠しているか、responsibility を wrong layer へ移しているか、派生入れ物を元 source / TXT / fragment の代替 authority にしているかである。
