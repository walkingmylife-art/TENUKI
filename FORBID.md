FORBID.md
TENUKI Forbid

この文書は、破ると責任境界が壊れる禁止事項だけを置く。

禁止事項を、キーワードだけで機械的に適用してはいけない。

禁止で判断をマスクしてはいけない。

1. Authority 違反

Observation を authority として扱ってはいけない。

Derived artifact を authority として扱ってはいけない。

Session state を committed authority として扱ってはいけない。

既存コード、既存ファイル、過去の成功、古いコメント、古い計画を、それだけで current authority として扱ってはいけない。

責任を持たない downstream code に authority の infer / select / replace / repair / recommit をさせてはいけない。

verification 前に effective value を commit してはいけない。

temporary rescue を、adopt reason と commit path なしに permanent にしてはいけない。

2. 責務違反

request / response shape handling と core logic を混ぜてはいけない。

route ごとの policy difference を、commonization の名目で消してはいけない。

failure を dummy success で隠してはいけない。

責務境界が不明確なまま、ファイルサイズ削減だけを目的に分割してはいけない。

worker に入口判断を再発明させてはいけない。

3. Side Effect 違反

観測だけの処理に commit / save / cache update / authority update を混ぜてはいけない。

出力先作成を authority commit と混同してはいけない。

preview / scan / generated output を authority に昇格させてはいけない。

route policy で禁止された side effect を共有処理に紛れ込ませてはいけない。

4. 派生入れ物による責務逃れ

派生入れ物を、元 source / TXT / request / fragment の代替 authority として扱ってはいけない。

派生入れ物が存在することを、その処理の正しさの根拠にしてはいけない。

処理が入っていることを、派生入れ物の正当性の根拠にしてはいけない。

局所処理の都合で作った派生入れ物に、全体 flow の責務を持たせてはいけない。

表示用、transport 用、cleanup 用、diagnostics 用の値を、明示理由なしに lookup / register / persist / commit の基準へ戻してはいけない。

5. Legacy / Migration 違反

legacy behavior を current specification として保護してはいけない。

old plan、old comment、old test、old memory を、現在のユーザー指示と現在のソースコードより上位にしてはいけない。

legacy compatibility と current rule を同じ階層に置いてはいけない。

legacy detection は禁止ではない。

legacy input は、責任を持つ層が明示的に adopt する場合だけ current rule へ normalize / migrate してよい。

6. UI / Worker 違反

サイズ比例の I/O、parse、render work を UI thread で実行してはいけない。

localized text helper がある場所で、user-facing string を無理由に直書きしてはいけない。

UI convenience を authority 判断にしてはいけない。

worker result は、current target / source / generation / session と一致する場合だけ反映する。

7. Comment / Naming 違反

history として comment を書いてはいけない。

old behavior を current behavior のように説明する comment を残してはいけない。

広い名前で責務を曖昧にしてはいけない。

分かりやすい名前の派生入れ物を、存在しているだけで保護対象にしてはいけない。

ただし、名前だけで禁止してはいけない。

8. 禁止ではないもの

以下は、それだけでは禁止ではない。

committed path を materialize すること
committed authority を validate / use すること
authority から derived artifact を regenerate すること
legacy path を observation として detect すること
必要な中間変数、index、cache、pattern、span を作ること
小さい local fix
route policy difference を保つこと
責務境界が明確になるまで分割を遅らせること

確認するべきことは、それが authority を変えているか、failure を隠しているか、責務を間違った層へ移しているかである。