//! 翻訳プロンプトのプレフィックス生成

fn lang_name_en(code: &str) -> &'static str {
    match code {
        "zh"     => "Chinese",
        "zh-CN"  => "Simplified Chinese",
        "zh-Hant"=> "Traditional Chinese",
        "zh-TW"  => "Traditional Chinese",
        "en"     => "English",
        "ja"     => "Japanese",
        "ko"     => "Korean",
        "fr"     => "French",
        "de"     => "German",
        "es"     => "Spanish",
        "it"     => "Italian",
        "pt"     => "Portuguese",
        "ru"     => "Russian",
        "ar"     => "Arabic",
        "th"     => "Thai",
        "vi"     => "Vietnamese",
        _        => "",
    }
}

fn resolve_lang_name(
    code: &str,
    custom_code: &str,
    custom_name: &str,
) -> String {
    if !custom_code.is_empty() && code == custom_code && !custom_name.is_empty() {
        return custom_name.to_string();
    }
    let preset = lang_name_en(code);
    if !preset.is_empty() { preset.to_string() } else { code.to_string() }
}

/// 言語ペアから翻訳指令を生成する。
///
/// HY-MT1.5 の推奨テンプレートに合わせて、常に英語の定型文を使う。
pub fn build_lang_prefix(
    _src_lang: &str,
    tgt_lang: &str,
    custom_code: &str,
    custom_name: &str,
    prompt_template: &str,
) -> String {
    let target = resolve_lang_name(tgt_lang, custom_code, custom_name);
    prompt_template
        .replace("{target}", &target)
        .replace("{language}", &target)
        .replace("{lang}", &target)
}

#[cfg(test)]
mod tests {
    use super::build_lang_prefix;

    #[test]
    fn uses_official_template_for_japanese() {
        assert_eq!(
            build_lang_prefix(
                "zh-CN",
                "ja",
                "",
                "",
                "Translate the following segment into {target}, without additional explanation.",
            ),
            "Translate the following segment into Japanese, without additional explanation.",
        );
    }

    #[test]
    fn uses_official_template_for_english() {
        assert_eq!(
            build_lang_prefix(
                "ja",
                "en",
                "",
                "",
                "Translate the following segment into {target}, without additional explanation.",
            ),
            "Translate the following segment into English, without additional explanation.",
        );
    }

    #[test]
    fn uses_custom_target_name_when_selected() {
        assert_eq!(
            build_lang_prefix(
                "ja",
                "vi",
                "vi",
                "Vietnamese",
                "Translate the following segment into {target}, without additional explanation.",
            ),
            "Translate the following segment into Vietnamese, without additional explanation.",
        );
    }

    #[test]
    fn uses_custom_template_placeholders() {
        assert_eq!(
            build_lang_prefix("ja", "en", "", "", "Only output {language}."),
            "Only output English.",
        );
    }
}
