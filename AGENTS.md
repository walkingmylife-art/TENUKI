PRINCIPLES.md と FORBID.md の内容を遵守して作業すること。

応答は日本語。

現在のソースコードと現在のユーザー指示が、過去の計画・コメント・記憶・既存テストより優先される。

translator.rs / backend translator 配下の作業では TRANSLATOR_RECONSTRUCTION.md の処理順を守ること。

ただし、TRANSLATOR_RECONSTRUCTION.md より現在のソースコードと現在のユーザー指示が優先される。

## translator 作業時の現在方針

translator refactor では、旧 signed-ZM key split を復活させない。

現在の lookup / register / persist の基準は FragmentAuthority.source である。

ZM -> number は model transport 専用の一時値であり、辞書 key authority ではない。

Regex persist は resolve / persist candidate の段で判断する。

辞書前に ZM 付き fragment を core / prefix / suffix に削って key 化しない。

render 済み String を source parse / lookup / register / persist に戻さない。

## 派生入れ物の確認

実装時は、処理だけでなく派生入れ物も確認すること。

ここでいう派生入れ物とは、元の source / text / request / TXT / fragment などから作られた中間変数、派生値、内部 index、一時構造、別名の値を指す。

例: key, value, normalized_key, source_norm, model_input, cleaned, translated, source_span, pattern, replacement, index, cache entry, persist entry。

派生入れ物は、存在しているだけでは根拠にならない。

処理が入ったこと、分離されたこと、テストが通ったことだけを、その派生入れ物の正当性の根拠にしないこと。

特に lookup / register / persist / cache / dict / render 周りでは、以下を確認すること。

- その派生入れ物は何から作ったか
- どの処理が使うか
- 後段判断に使っているか
- 元 source / TXT / fragment の代役になっていないか
- 撤去対象の派生入れ物が保護対象に変わっていないか
- 全体で持つ責務を局所処理で再実装していないか

## 完了扱いにしないもの

以下だけでは完了扱いしない。

- 型を追加しただけ
- テストが通っただけ
- 処理を分離しただけ
- 派生入れ物を作っただけ
- 旧経路が残ったまま新経路を横に足しただけ
- trace 名や旧テストを満たすために compatibility 分岐を残しただけ
- render 済み String を再び parse / lookup / register に戻せる経路が残っている状態
- 派生入れ物の存在を理由に処理の正当性を説明している状態

## 完了報告に必ず書くもの

完了報告では、通過テストだけでなく以下を書く。

- 変更したファイル
- 変更した関数 / 型 / 主要変数
- 削除した旧経路
- 残存している経路
- 追加した派生入れ物
- 削除した派生入れ物
- 残した派生入れ物
- 各派生入れ物が何から作られ、どの処理に使われるか
- 変更または削除したテスト
- 新しく追加した境界テスト
- 未確認リスク
- 触っていない範囲

## before / after 報告

大きめの変更では、可能な範囲で before / after を表で出すこと。

最低限、以下を比較できる形にする。

- 旧要素
- 新要素
- 責務の移動先
- 呼び出し元
- 呼び出し先
- lookup 順
- 読み込み元
- 保存先
- 残存確認

## 注意

既存コードにある normalize / trim / sanitize / cache / index / value などの処理を、存在しているという理由だけで保護対象にしないこと。

それらは過去の局所最適で混入した撤去対象の可能性がある。

ただし、名前だけで禁止しないこと。

必要な派生入れ物は存在してよい。

判断するのは、その派生入れ物が今回の責務位置に合っているか、元 source / TXT / fragment の責任を逃がしていないかである。