# TENUKI Alignment Checklist

新実装後に毎回通す、短い揃え工程。

これは新機能の設計書ではなく、「動いたあとに全体へ揃えて閉じる」ための確認リスト。

現在のソースコードと現在のユーザー指示が、この文書より優先される。

更新元確認: `ALIGNMENT_CHECKLIST.md` 

## 0. 目的

実装完了と揃え完了を分ける。

実装完了は、目的の機能または修正が動き、対象テストが通り、依頼された挙動が入っている状態。

揃え完了は、その実装が TENUKI 全体の入口、責務、authority、side effect、UI 資産、命名、派生入れ物の扱いに揃っている状態。

処理が入ったこと、型が分かれたこと、テストが通ったことだけでは揃え完了ではない。

## 1. 完了の分離

### 実装完了

* 目的の機能または修正が動く
* 変更対象のテストが通る
* 依頼された挙動が入っている
* コンパイルが通る
* 必要な最低限の境界テストがある

### 揃え完了

* 入口が本来の mode / flow に載っている
* 既存の authority / side effect 境界を越えていない
* UI 文言や表示資産が退化していない
* サイズ比例の I/O / parse / render が UI thread に残っていない
* 命名と責務が今回の意味に揃っている
* 壊してはいけない最小シナリオを確認した
* 旧経路を新経路の横に残しただけで閉じていない
* 完了報告に削除/残存経路を書いた
* 完了報告に追加/削除/残存した派生入れ物を書いた

## 2. 派生入れ物の確認

ここでいう派生入れ物とは、元の `source` / `text` / request / TXT / fragment などから作られた中間変数、派生値、内部 index、一時構造、別名の値、cache entry、pattern、span、model input、render atom などを指す。

会話内で「皿」と呼ぶものは、ここでは「派生入れ物」と書く。

例:

* key
* value
* normalized_key
* normalized_value
* source_norm
* source_raw
* model_input
* cleaned
* translated
* restored_output
* visible_text
* source_span
* pattern
* replacement
* index
* exact_index
* value_index
* source_index
* cache entry
* persist entry
* render atom
* diagnostics payload

派生入れ物は、存在しているだけでは根拠にならない。

処理があることは、その派生入れ物の正当性の根拠ではない。

派生入れ物があることは、その処理の正当性の根拠ではない。

### 必ず確認すること

各派生入れ物について確認する。

* 何から作られたか
* 何を捨てたか
* 何を足したか
* どの処理が使うか
* 局所では何のために必要か
* 全体 flow から見て必要か
* 有効範囲はどこまでか
* 後段判断に使ってよいか
* 元入力の代役になっていないか
* 撤去対象なのに保護対象に昇格していないか

### 特に危険な状態

* 処理を分離するために作った派生入れ物が、後段判断の根拠になっている
* 派生入れ物があるから処理が正しい、処理があるから派生入れ物が正しい、という循環になっている
* 名前が分かりやすいだけで、その入れ物が保護対象になっている
* 旧実装由来の派生入れ物を、既存仕様として守っている
* 全体で持っている責務を、局所の派生入れ物が再実装している
* source / TXT / fragment を見るべき処理が、派生値を見ている
* lookup / register / persist / covered / conflict 判定が、未指定の normalize / trim / sanitize / canonicalize 結果を見ている

## 3. 揃え工程

### A. 入口

* 本来の mode / flow の入口になっているか
* temporary button / temporary branch / if false が残っていないか
* 補助機能扱いに戻っていないか
* 旧入口と新入口が並走していないか
* compatibility wrapper が final path に残っていないか
* 入口で固定すべき判断を、下流 worker が再判断していないか

### B. 境界

* authority は正しい場所を読んでいるか
* observation / adopt / commit を混ぜていないか
* 今回触らない cache / dict / input analysis / config save を巻き込んでいないか
* 既存の拘束済みテストを維持しているか
* 旧 comment / 旧 plan / 旧 test を current spec として扱っていないか
* derived artifact を authority として扱っていないか
* session state を committed authority として扱っていないか

### C. 派生入れ物

* 今回追加した派生入れ物は何か
* 今回削除した派生入れ物は何か
* 今回残した派生入れ物は何か
* 各派生入れ物は何から作るか
* 各派生入れ物はどの処理に使われるか
* 各派生入れ物は後段判断に使われるか
* その後段判断に使ってよい根拠はあるか
* 元 source / TXT / request / fragment と一致しなくなる点はあるか
* 局所では成立しているが、全体では不要な処理はないか
* 撤去対象の派生入れ物が、既存構造として保護されていないか

### D. UI 資産

* 既存言語表示を落としていないか
* ユーザー向け文字列が局所直書きに逆流していないか
* 文字化け literal や英語固定化が混じっていないか
* status / log / empty state が今回の state と矛盾していないか
* 既存 helper がある表示を別実装していないか
* UI convenience が authority 判断になっていないか

### E. 重い処理

* サイズ比例の I/O / parse / render を UI thread で実行していないか
* preview は bounded preview に留まっているか
* loading 中も終了、切替、別操作が返るか
* worker 結果は現在の target と一致する場合だけ反映しているか
* scan / preview / run の generation mismatch を無視していないか
* worker 側で入口判断を再発明していないか

### F. 命名と責務

* state 名は実際の意味と一致しているか
* helper / enum variant / command 名が古い責務を引きずっていないか
* run-only / observe-only / authority / commit などの語が混ざっていないか
* コメントは短く、壊れやすい契約だけを書いているか
* broad term を使って responsibility を曖昧にしていないか
* 分かりやすそうな名前の派生入れ物が、実際より強い意味を持っていないか

### G. 確認

* 今回壊してはいけない最小シナリオを通したか
* 既存の重要テストを維持しているか
* 新しく落ちやすい契約は小テストへ落としたか
* 実装完了ではなく、揃え完了まで確認したか
* 完了報告に削除/残存/未確認リスクを書いたか
* 完了報告に追加/削除/残存した派生入れ物を書いたか
* passing tests が何を保証し、何を保証しないかを書いたか

## 4. 混ぜてはいけない境界

### 仕様境界

* authority
* observation
* adopt
* commit
* side effect
* derived artifact
* session state

### UI 資産

* wording
* status
* log
* empty state
* preview
* work result
* committed setting

### 実行方式

* worker
* UI thread
* preview upper bound
* cancel
* exit response
* generation / session target

### 命名契約

* state
* helper
* enum variant
* command
* comment
* test name
* report label

### translator 境界

* source parse
* plan
* resolve
* render
* wrap
* diagnostics
* final response

### ZM 境界

* FragmentAuthority
* ModelTransport
* ModelReturn
* PersistCandidate
* display text
* dictionary key
* regex pattern

### persist 境界

* Exact
* Regex
* TranslationCache
* NewEntriesCache
* dict.txt authority
* dict.bin derived artifact
* same-session hit
* shutdown flush

## 5. TENUKI の拘束済み仕様例

* `/list` は dictionary / cache / input analysis を更新しない
* List mode の file_translate.active は panel visibility ではなく List mode state
* List output directory 作成は authority を変更しない
* dict_slot は upstream で選択・commit された authority
* dict.txt / Tenuki.dict.txt / Tenuki.regex.txt は辞書編集面の authority
* dict.bin は辞書 TXT authority から生成される derived artifact
* dict.bin を authority として扱わない
* TranslationCache / NewEntriesCache は session state であり、dict_slot や language boundary を越えない
* normal `/translate` と `/list` は route policy で side effect を分ける
* File Translate の output placement を committed dict_slot に昇格させない

## 6. 辞書 / cache / persist 揃え

辞書・cache・persist 周りの実装後は、特に以下を確認する。

### A. TXT と dict.bin

* 辞書 TXT に見えている source/value と runtime lookup 対象が対応しているか
* dict.bin は TXT から生成された derived artifact に留まっているか
* dict.bin を跨いで slot / language を再利用していないか
* dict.bin に載せる key は、意図した source そのものか
* dict.bin 生成前に未指定の normalize / trim / sanitize / canonicalize をしていないか
* TXT 読み込み時の行判定と key/value 生成を混ぜていないか

### B. 辞書と cache

* Dictionary exact lookup と TranslationCache same-session hit を混同していないか
* NewEntriesCache は TXT 保存待ちであり、dictionary authority ではないことが保たれているか
* exact same-session hit を Dictionary 側で再実装していないか
* regex live registration が same-session variant hit のためであることが明示されているか
* exact と regex の commit 経路が混ざっていないか

### C. 派生入れ物

* source_norm / normalized_key / normalized_value のような派生入れ物が、lookup/register/persist/covered/conflict の基準になっていないか
* value_index を残す場合、その意味が source lookup と混ざっていないか
* covered / conflict 判定が、実際の lookup target と一致しているか
* regex-covered exact 除外が、辞書編集面から理解できる report を持っているか
* 表示用 preview / diagnostics 用 normalized text を、lookup/register/persist に戻していないか

## 7. Translator 揃え

translator 実装後は、実装完了と揃え完了を分けて確認する。

### A. Entry flow

`translate_chunk` が現在の正規順序になっているか確認する。

```text
parse_source_document
-> plan_document
-> resolve_document
-> render_document
-> wrap_final_display_text
```

確認項目。

* 旧 flat final path が response 作成に残っていないか
* temporary wrapper が final response path で使われていないか
* old resolve_plan(nodes) -> String 系が残っていないか
* TranslationResult.text が render_document 由来になっているか
* wrap が render_document の前に戻っていないか

### B. Authority

* FragmentAuthority.source が lookup/register/persist の基準になっているか
* model_input が lookup/register/persist source になっていないか
* restored_output が lookup/register source に戻っていないか
* rendered String が source parse / lookup / register に戻っていないか
* old comment / old trace / old test が authority の根拠になっていないか

### C. ZM

* ZM -> number が model transport 専用になっているか
* 辞書前に signed-ZM を core / prefix / suffix へ削る経路が復活していないか
* Regex persist が raw exact cache によって即座にマスクされないか
* dict_key/prefix/suffix という旧中間 authority を新型へ持ち込んでいないか
* apply_zm_key_plan / zm_key_split 系の旧仕様が current spec として残っていないか
* model_input から authority source を再構築していないか

### D. Fragment / render

* Fragment / child.text を response として扱っていないか
* ResolvedFragmentNode.text が元の座席へ戻るまで final String になっていないか
* result.text を作る場所が render_document に限定されているか
* separator / newline / protected surface が元位置へ戻っているか
* rich tag / existing newline / visible surface の境界を壊していないか
* render が lookup / model / persist を呼んでいないか

### E. Mixed bracket / whole Fragment

* mixed bracket segment が暗黙 whole Fragment に戻っていないか
* whole Fragment を使う場合、専用 policy 関数と理由コメントがあるか
* old whole Fragment tests を維持目的で残していないか
* model call 削減だけを理由に whole Fragment 化していないか
* Fragment 同士を後から結合していないか

### F. Route side effects

* `/translate` の side effect と `/list` の side effect が route policy で分かれているか
* `/list` が dictionary / cache / input analysis / committed dict_slot を更新していないか
* File Translate の output placement が dictionary authority に昇格していないか
* List output directory creation が committed dict_slot を変更していないか
* server 側 commit が Exact / Regex を明示分岐しているか

### G. Cache / persist

* Exact persist と Regex persist が commit 前に潰れていないか
* Regex persist が generic exact cache insert に流れていないか
* TranslationCache source index と value observation index の意味が混ざっていないか
* NewEntriesCache が Exact/Regex 区別を失っていないか
* dict.txt authority と session cache を混同していないか
* 派生入れ物の存在を理由に、cache / persist の正当性を後付けしていないか

### H. Tests

* 境界テストが追加されているか
* 旧 signed-ZM key split テストを削除または置換したか
* 旧 whole Fragment テストを削除または置換したか
* 旧 trace 名を守るだけのテストを残していないか
* passing tests だけでなく、削除/残存経路を完了報告に書いたか
* テストが新しい派生入れ物を保護対象にしていないか

## 8. 実装指示 MD の末尾テンプレート

作業AIへ渡す実装指示 MD の末尾には、必要に応じて以下を付ける。

### 揃え工程

* 入口が temporary shape に戻っていないか確認
* 既存の authority / side effect 境界を越えていないか確認
* UI 文言、status、log、empty state が退化していないか確認
* UI thread にサイズ比例の I/O / parse / render が残っていないか確認
* state / helper / enum / command 名が今回の責務と一致しているか確認
* 追加/削除/残存した派生入れ物を確認
* 各派生入れ物が何から作られ、どの処理に使われ、後段判断に使われるか確認
* 撤去対象の派生入れ物が保護対象に昇格していないか確認
* 壊してはいけない最小シナリオと重要テストを確認
* 削除/残存経路と未確認リスクを完了報告に書く

## 9. 完了報告チェック

完了報告には最低限以下を含める。

### 変更概要

* 変更したファイル
* 変更した関数
* 変更した型
* 変更した entry flow
* 変更した side effect

### 削除/残存経路

* 削除した旧経路
* 残存している経路
* 残した理由
* 残してはいけないが残っている未解決経路

### 派生入れ物

* 追加した派生入れ物
* 削除した派生入れ物
* 残した派生入れ物
* 各派生入れ物の元入力
* 各派生入れ物を使う処理
* 各派生入れ物が後段判断に使われるか
* 後段判断に使う根拠

### before / after 対応表

最低限、以下を書く。

* 旧要素
* 新要素
* 責務の移動先
* 呼び出し元
* 呼び出し先
* 読み込み元
* 保存先
* lookup 順
* 残存確認

### テスト

* 通過テスト
* 変更したテスト
* 削除/置換したテスト
* 新しく追加した境界テスト
* テストが保証すること
* テストが保証しないこと

### 未確認

* 未確認リスク
* 触っていない範囲
* 別フェーズに分けた事項
* 判断保留事項

通過テストだけで完了扱いしない。
