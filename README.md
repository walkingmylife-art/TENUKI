# TENUKI

TENUKI は `llama-server` バックエンドを用いたリアルタイム翻訳ツールです。

## List mode

List mode は、選択したフォルダ内の表形式テキストを翻訳し、位置情報を保持した `.translated.jsonl` を出力します。
通常翻訳の辞書登録・cache・input analysis は更新しません。

### 出力

- 出力先: `dicts/{target}/text/list_output`
- 出力形式: `.translated.jsonl`
- 各行は `column_index`, `row_index`, `source`, `target`, `mode` を含む

### Column mode

| mode      | 説明 |
|-----------|------|
| Translate | セルを `/list` で翻訳して出力する |
| Original  | セルを翻訳せずそのまま出力する（`target == source`） |
| None      | その列は出力しない |

### CSV / 区切りテキスト

- 初期状態は header 未確定（`HeaderMode::Unknown`）として読み、preview suggestion から header あり（`Present`）／なし（`Absent`）を確定する
- header と判定された行はデータ行として出力しない

### JSON

- `JSON array<object>`: object の key を列名として扱い、value を行として扱う
- `JSON array<array>`: array の各行を表の行として扱い、列名は `col N` になる
- JSON はデータ形状から表を決めるため、header toggle は表示しない

### Preview

- 初期表示は最大100行
- スクロールで段階的に最後まで表示

### List ログ

- `[n/total] source` 行は source 色（白系）
- `=> target` 行は target 色（青系）
- `[done]` 行は success 色（緑）
- Error 行は赤
- 辞書 hit 色（緑）を List 翻訳行に流用しない

### 言語切替時の辞書確認

- target 変更時、現在の辞書が新 target と一致しない場合、確認ダイアログを表示する
- 「そのまま使う」「新しく作る」をユーザーが選ぶ
- 選択前に `config.toml` の `dict_slot` を変更しない

### 既知の制限

- 現在の List mode は JSONL 出力のみ
- 元の CSV / JSON ファイルへ直接書き戻さない
- AssetStudio 的なバイナリ抽出は未対応
- List mode 用の tag / placeholder 保護は別フェーズ
