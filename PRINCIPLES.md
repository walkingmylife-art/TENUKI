PRINCIPLES.md
TENUKI Principles

この文書は、TENUKI の判断原則を置くための文書である。

作業計画、現在タスク、過去事故の一覧はここに置かない。

0. 最優先

現在のユーザー指示と現在のソースコードが primary evidence である。

過去の計画、コメント、記憶、既存テスト、過去の成功、検出されたファイル、残存 artifact は、それだけでは authority ではない。

必要な場合だけ、責任を持つ層が observation として採用する。

1. Authority

Authority とは、判断経路がその判断を所有し、失敗時の責任が戻る値である。

値は、責任を持つ層が current evidence から adopt し、effective value として commit した時だけ authority になる。

Observation は authority ではない。

Derived artifact は authority ではない。

Session state は authority ではない。

既存コードの形は authority ではない。

2. Observation / Adopt / Commit

Observation は、判断前に見えている材料である。

Adopt は、責任を持つ層が observation を使うと決めること。

Commit は、adopt した値を effective value として保存、確定、または下流へ渡すこと。

観測段階では、判断に必要な候補を広く見る。

commit 段階では、採用された値だけを狭く確定する。

観測段階の候補列挙と、commit 段階の authority 判定を混同しない。

3. Authority と実装判断

authority は実装判断そのものではない。

authority validator を通したことは、その実装判断が正しい根拠にならない。

確認するべきことは、以下である。

その関数の責務は何か
その判断は誰が持つべきか
その validator はどの責任境界のためのものか
その場所で使ってよい判定か
その判定で、本来観測すべきものを消していないか
4. 責務

責務は、処理がある場所ではなく、判断が失敗した時に責任が戻る場所で見る。

downstream code は、commit 済み authority を read / validate / use / materialize してよい。

downstream code は、authority を infer / select / replace / repair / recommit してはいけない。

worker は narrow に保つ。

orchestration は worker の一層上に置く。

責務境界で分割する。

ファイルサイズだけで分割しない。

5. 派生入れ物

派生入れ物とは、元の source / text / request / TXT / fragment などから作られた中間変数、派生値、内部 index、一時構造、別名の値である。

派生入れ物は、存在しているだけでは正当性を持たない。

処理があることは、派生入れ物の正当性の根拠ではない。

派生入れ物があることは、その処理の正当性の根拠ではない。

必要な派生入れ物は作ってよい。

ただし、元入力や authority の代役にしてはいけない。

6. 禁止と観測

禁止は、破ると責任境界が壊れるものに絞る。

禁止を増やして、実装判断を見えなくしてはいけない。

判断が必要な箇所では、禁止で覆うのではなく、判断がどの責務に乗っているかを観測する。

7. 新規開発

新規開発では、過去の TENUKI 事例をそのまま前提にしない。

まず以下を置く。

目的
入力
出力
読むもの
書くもの
責任境界
観測点
最小の完了条件

過去事例は、必要な場合だけ判断軸へ圧縮して使う。

8. One Line Summary

現在の証拠から、責任を持つ層が adopt / commit したものだけが authority である。

authority は実装判断そのものではない。

禁止を増やすより、責務と観測点を見る。