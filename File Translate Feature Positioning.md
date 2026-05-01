File Translate Feature Positioning.md
File Translate Feature Positioning

この文書は、File Translate の位置づけを記録する steering document である。

これは active plan ではない。

これは current task list ではない。

現在のソースコードと現在のユーザー指示が、現在の実装状態を決める。

0. 目的

File Translate を、normal translation の単純な拡張として扱わない。

File Translate は、TENUKI の既存 UI stage と translation transport を借りる独立した feature lane である。

目的は、File Translate をアプリ全体の再設計理由にせず、既存 stage を壊さずに独立機能として置くこと。

1. 基本位置

File Translate は normal translation ではない。

File Translate は /list そのものでもない。

File Translate は、必要に応じて /list transport を使う独立した workflow である。

File Translate:
独立した feature lane

/list:
複数 text を処理する transport / backend entry

normal /translate:
通常翻訳の runtime entry

これらを同じ意味にしない。

2. File Translate が借りてよいもの

File Translate は、既存の部品を借りてよい。

既存 UI frame
center result / work area
log layer
translation transport
helper functions
dictionary format output

ただし、借りることは再定義ではない。

借りた部品があることは、File Translate と normal translation が同じ意味である根拠にならない。

3. File Translate がしてはいけないこと

File Translate のために、TENUKI 全体の stage を壊さない。

File Translate のために、normal translation の UI contract を弱めない。

File Translate のために、authority boundary を弱めない。

File Translate のために、project-wide abstraction を早く作りすぎない。

File Translate のために、normal translation core へ無理に押し込まない。

4. UI 位置

File Translate は、既存 UI を stage として使う。

正しい方向。

既存 layout を借りる
center result / work area を使う
feature-specific navigation は side panel に置く
preview は side panel / work area へ出す
log は既存 log structure を借りる

避ける方向。

center を File Translate 専用画面に作り替える
normal translation UI contract を変更する
File Translate のために unrelated operation system を足す
5. /list との関係

File Translate は /list transport を使ってよい。

ただし、File Translate と /list は同じ意味ではない。

/list は backend transport のひとつである。

File Translate は UI workflow と実行管理を含む feature lane である。

/list を使うために、File Translate を normal translation core へ押し込まない。

6. authority 位置

File Translate は独立機能だが、TENUKI の authority boundary は守る。

preview は observation である。

preview は authority ではない。

scan result は observation である。

run readiness / run config / run plan は、実行時に使う固定入力である。

List output directory は、その run の output placement である。

List output directory は、committed dict_slot authority ではない。

committed dict_slot と File Translate output placement を混ぜない。

7. side effect 境界

File Translate / /list は、normal translation と side effect が違う。

/list は以下をしない。

dictionary authority commit
translation cache update
input analysis update
normal statistics update
normal dict_slot commit

File Translate の実行結果を、normal translation の authority として扱わない。

File Translate の output placement を、通常辞書の committed slot に昇格させない。

8. 変更判断

File Translate の形は feature shape から決める。

次の理由だけで UI や authority を変えない。

authority を表現しやすい
commit point を置きやすい
backend に寄せやすい
commonization がきれいに見える
feature-local controller を避けたい

feature shape が先。

その後で、内部実装を authority boundary に合わせる。

9. 現在状態の確認

この文書は current state を定義しない。

現在の挙動は、ソースコードとテストを見る。

特に確認すること。

Run / Stop が現在どう実装されているか
readiness がどこで評価されているか
runner が現在どの output schema を書くか
dict_slot と List output directory が分かれているか
/list side effects が無効のままか
preview / scan / run の状態が混ざっていないか
10. One-line Summary

File Translate は、既存 UI stage と translation transport を借りる独立 feature lane である。

借りることと同一化を混同しない。

preview は observation、run output は output placement、committed dict_slot は別 authority として扱う。