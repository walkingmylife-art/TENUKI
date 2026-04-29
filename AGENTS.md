PRINCIPLES.md と FORBID.md の内容を遵守して作業すること。

応答は日本語。

translator.rs / backend translator 配下の作業では TRANSLATOR_RECONSTRUCTION.md の処理順を守ること。

ただし、TRANSLATOR_RECONSTRUCTION.md より現在のソースコードと現在のユーザー指示が優先される。

translator 作業時の現在方針

translator refactor では、旧 signed-ZM key split を復活させない。

現在の lookup / register / persist の基準は FragmentAuthority.source である。

ZM -> number は model transport 専用の一時値であり、辞書 key authority ではない。

Regex persist は resolve / persist candidate の段で判断する。

辞書前に ZM 付き fragment を core / prefix / suffix に削って key 化しない。

完了扱いにしないもの

以下だけでは完了扱いしない。

型を追加しただけ
テストが通っただけ
旧経路が残ったまま新経路を横に足しただけ
trace 名や旧テストを満たすために compatibility 分岐を残しただけ
render 済み String を再び parse / lookup / register に戻せる経路が残っている状態
完了報告に必ず書くもの

完了報告では、通過テストだけでなく以下を書く。

削除した旧経路
残存している経路
変更または削除したテスト
新しく追加した境界テスト
未確認リスク
触っていない範囲