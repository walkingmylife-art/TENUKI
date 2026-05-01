ALIGNMENT_CHECKLIST.md
TENUKI Alignment Checklist

新実装後に通す、短い揃え工程。

これは新機能の設計書ではない。

現在のソースコードと現在のユーザー指示が、この文書より優先される。

0. 目的

実装完了と揃え完了を分ける。

実装完了は、目的の機能または修正が入り、必要な確認が通った状態。

揃え完了は、その実装が TENUKI 全体の入口、責務、authority、side effect、UI、命名に揃っている状態。

テストが通っただけでは揃え完了ではない。

authority validator を通しただけでも揃え完了ではない。

1. 最初に見ること
今回の目的は何か
変更範囲はどこか
触らない範囲はどこか
入力は何か
出力は何か
読むものは何か
書くものは何か
責任境界はどこか
最小の完了条件は何か
2. 入口
本来の mode / flow の入口になっているか
temporary button / temporary branch が残っていないか
旧入口と新入口が並走していないか
下流 worker が入口判断を再発明していないか
3. 境界
authority は正しい場所を読んでいるか
observation / adopt / commit を混ぜていないか
derived artifact を authority として扱っていないか
session state を committed authority として扱っていないか
今回触らない cache / dict / input analysis / config save を巻き込んでいないか
authority validator を使った場合、その validator の責任境界と今回の使用場所が合っているか
観測段階で拾うべき候補を、commit 用の判定で消していないか
4. Side Effect
読むだけの処理が save / commit / cache update をしていないか
出力先作成を authority commit にしていないか
route policy で禁止された side effect が共有処理に混ざっていないか
worker result は current target / source / generation / session と一致する時だけ反映しているか
5. UI
既存言語表示を落としていないか
ユーザー向け文字列が局所直書きに逆流していないか
status / log / empty state が今回の state と矛盾していないか
UI convenience が authority 判断になっていないか
サイズ比例の I/O / parse / render が UI thread に残っていないか
6. 派生入れ物

派生入れ物を追加・変更した場合だけ見る。

何から作ったか
何を捨てたか
何を足したか
どの処理が使うか
有効範囲はどこまでか
後段判断に使うか
後段判断に使ってよい理由はあるか
元入力や authority の代役になっていないか
7. 命名とコメント
state / helper / command 名は実際の責務と合っているか
古い責務を名前が引きずっていないか
コメントは現在の契約を書いているか
history を current behavior のように書いていないか
広い語で責務を曖昧にしていないか
8. 確認
壊してはいけない最小シナリオを確認したか
必要なテストを実行したか
テストが保証することと保証しないことを把握したか
旧経路が残っている場合、残す理由があるか
未確認リスクを言えるか
9. 完了報告

小変更では、最低限これだけでよい。

変更したファイル
変更した主な箇所
削除した旧経路があればそれ
残した経路があればそれ
実行した確認
未確認リスク

大きい変更、責務移動、authority 境界変更がある場合だけ、以下を追加する。

before / after
呼び出し元 / 呼び出し先
読み込み元 / 保存先
side effect の変化
追加 / 削除 / 残存した派生入れ物
authority validator を使った場所と、その責任境界
10. One Line Summary

動いたかではなく、責務の場所が合っているかを見る。

禁止を増やすのではなく、判断を観測できる形にする。