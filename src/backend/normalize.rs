//! 文字列正規化ユーティリティ
//! - `normalize_key`: 辞書のキー向け（全角英数字→半角、記号多数変換、大文字→小文字）
//! - `normalize_display`: 翻訳結果の表示向け（全角記号の一部を半角化、不要な変更は行わない）

pub fn normalize_key(key: &str) -> String {
    key.chars()
        .map(|c| match c {
            'Ａ'..='Ｚ' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            'ａ'..='ｚ' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            '０'..='９' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            '！' => '!',
            '＂' => '"',
            '＃' => '#',
            '＄' => '$',
            '％' => '%',
            '＆' => '&',
            '＇' => '\'',
            '（' => '(',
            '）' => ')',
            '＊' => '*',
            '＋' => '+',
            '，' => ',',
            '－' => '-',
            '．' => '.',
            '／' => '/',
            '：' => ':',
            '；' => ';',
            '＜' => '<',
            '＝' => '=',
            '＞' => '>',
            '？' => '?',
            '＠' => '@',
            '［' => '[',
            '＼' => '\\',
            '］' => ']',
            '＾' => '^',
            '＿' => '_',
            '｀' => '`',
            '｛' => '{',
            '｜' => '|',
            '｝' => '}',
            '～' => '~',
            _ => c,
        })
        .collect::<String>()
        .to_lowercase()
}

pub fn normalize_display(text: &str) -> String {
    text.replace('：', ":")
        .replace('＋', "+")
        .replace('－', "-")
        .replace('−', "-")
}
