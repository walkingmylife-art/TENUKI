TENUKI Alignment Checklist

新実装後に毎回通す、短い揃え工程。

これは新機能の設計書ではなく、「動いたあとに全体へ揃えて閉じる」ための確認リスト。

完了の分離
実装完了
目的の機能または修正が動く
変更対象のテストが通る
依頼された挙動が入っている
揃え完了
入口が本来の mode / flow に載っている
既存の authority / side effect 境界を越えていない
UI 文言や表示資産が退化していない
サイズ比例の I/O / parse / render が UI thread に残っていない
命名と責務が今回の意味に揃っている
壊してはいけない最小シナリオを確認した
旧経路を新経路の横に残しただけで閉じていない
完了報告に削除/残存経路を書いた
揃え工程
A. 入口
本来の mode / flow の入口になっているか
temporary button / temporary branch / if false が残っていないか
補助機能扱いに戻っていないか
旧入口と新入口が並走していないか
compatibility wrapper が final path に残っていないか
B. 境界
authority は正しい場所を読んでいるか
observation / adopt / commit を混ぜていないか
今回触らない cache / dict / input analysis / config save を巻き込んでいないか
既存の拘束済みテストを維持しているか
旧 comment / 旧 plan / 旧 test を current spec として扱っていないか
C. UI 資産
既存言語表示を落としていないか
ユーザー向け文字列が局所直書きに逆流していないか
文字化け literal や英語固定化が混じっていないか
status / log / empty state が今回の state と矛盾していないか
既存 helper がある表示を別実装していないか
D. 重い処理
サイズ比例の I/O / parse / render を UI thread で実行していないか
preview は bounded preview に留まっているか
loading 中も終了、切替、別操作が返るか
worker 結果は現在の target と一致する場合だけ反映しているか
scan / preview / run の generation mismatch を無視していないか
E. 命名と責務
state 名は実際の意味と一致しているか
helper / enum variant / command 名が古い責務を引きずっていないか
run-only / observe-only / authority / commit などの語が混ざっていないか
コメントは短く、壊れやすい契約だけを書いているか
broad term を使って responsibility を曖昧にしていないか
F. 確認
今回壊してはいけない最小シナリオを通したか
既存の重要テストを維持しているか
新しく落ちやすい契約は小テストへ落としたか
実装完了ではなく、揃え完了まで確認したか
完了報告に削除/残存/未確認リスクを書いたか
皿を混ぜない
仕様境界: authority / observation / adopt / commit / side effect
UI 資産: wording / status / log / empty state
実行方式: worker / UI thread / preview upper bound / cancel and exit response
命名契約: state / helper / enum variant / command / comment
translator 境界: source parse / plan / resolve / render / wrap
ZM 境界: FragmentAuthority / ModelTransport / ModelReturn / PersistCandidate
persist 境界: Exact / Regex / TranslationCache / NewEntriesCache / dict.txt authority
実装指示 MD の末尾テンプレート
## 揃え工程

- 入口が temporary shape に戻っていないか確認
- 既存の authority / side effect 境界を越えていないか確認
- UI 文言、status、log、empty state が退化していないか確認
- UI thread にサイズ比例の I/O / parse / render が残っていないか確認
- state / helper / enum / command 名が今回の責務と一致しているか確認
- 壊してはいけない最小シナリオと重要テストを確認
- 削除/残存経路と未確認リスクを完了報告に書く
TENUKI の拘束済み仕様例
/list は dictionary / cache / input analysis を更新しない
List mode の file_translate.active は panel visibility ではなく List mode state
List output directory 作成は authority を変更しない
dict_slot は upstream で選択・commit された authority
dict.bin は dict.txt authority から生成される derived artifact
TranslationCache / NewEntriesCache は session state であり、dict_slot や language boundary を越えない
Translator 揃え

translator 実装後は、実装完了と揃え完了を分けて確認する。

A. Entry flow

translate_chunk が現在の正規順序になっているか確認する。

parse_source_document
-> plan_document
-> resolve_document
-> render_document
-> wrap_final_display_text

確認項目。

旧 flat final path が response 作成に残っていないか
temporary wrapper が final response path で使われていないか
old resolve_plan(nodes) -> String 系が残っていないか
TranslationResult.text が render_document 由来になっているか
wrap が render_document の前に戻っていないか
B. Authority

確認項目。

FragmentAuthority.source が lookup/register/persist の基準になっているか
model_input が lookup/register/persist source になっていないか
restored_output が lookup/register source に戻っていないか
rendered String が source parse / lookup / register に戻っていないか
old comment / old trace / old test が authority の根拠になっていないか
C. ZM

確認項目。

ZM -> number が model transport 専用になっているか
辞書前に signed-ZM を core / prefix / suffix へ削る経路が復活していないか
Regex persist が raw exact cache によって即座にマスクされないか
dict_key/prefix/suffix という旧中間 authority を新型へ持ち込んでいないか
apply_zm_key_plan / zm_key_split 系の旧仕様が current spec として残っていないか
model_input から authority source を再構築していないか
D. Fragment / render

確認項目。

Fragment / child.text を response として扱っていないか
ResolvedFragmentNode.text が元の座席へ戻るまで final String になっていないか
result.text を作る場所が render_document に限定されているか
separator / newline / protected surface が元位置へ戻っているか
rich tag / existing newline / visible surface の境界を壊していないか
render が lookup / model / persist を呼んでいないか
E. Mixed bracket / whole Fragment

確認項目。

mixed bracket segment が暗黙 whole Fragment に戻っていないか
whole Fragment を使う場合、専用 policy 関数と理由コメントがあるか
old whole Fragment tests を維持目的で残していないか
model call 削減だけを理由に whole Fragment 化していないか
Fragment 同士を後から結合していないか
F. Route side effects

確認項目。

/translate の side effect と /list の side effect が route policy で分かれているか
/list が dictionary / cache / input analysis / committed dict_slot を更新していないか
File Translate の output placement が dictionary authority に昇格していないか
List output directory creation が committed dict_slot を変更していないか
server 側 commit が Exact / Regex を明示分岐しているか
G. Cache / persist

確認項目。

Exact persist と Regex persist が commit 前に潰れていないか
Regex persist が generic exact cache insert に流れていないか
TranslationCache source index と value observation index の意味が混ざっていないか
NewEntriesCache が Exact/Regex 区別を失っていないか
dict.txt authority と session cache を混同していないか
H. Tests

確認項目。

境界テストが追加されているか
旧 signed-ZM key split テストを削除または置換したか
旧 whole Fragment テストを削除または置換したか
旧 trace 名を守るだけのテストを残していないか
passing tests だけでなく、削除/残存経路を完了報告に書いたか
完了報告チェック

完了報告には最低限以下を含める。

Changed files:
Removed paths:
Remaining paths:
Changed tests:
Deleted/replaced tests:
New invariant tests:
Unverified risks:
Not touched:

通過テストだけで完了扱いしない。