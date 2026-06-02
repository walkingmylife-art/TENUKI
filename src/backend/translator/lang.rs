//! 翻訳プロンプトのプレフィックス生成

fn lang_name_en(code: &str) -> &'static str {
    match code {
        "zh-CN" => "Simplified Chinese",
        "zh-TW" => "Traditional Chinese",
        "en" => "English",
        "ja" => "Japanese",
        "ko" => "Korean",
        "ar" => "Arabic",
        _ => "",
    }
}

fn resolve_lang_name(code: &str, custom_name: &str) -> String {
    let preset = lang_name_en(code);
    if !preset.is_empty() {
        preset.to_string()
    } else if !custom_name.trim().is_empty() {
        custom_name.to_string()
    } else {
        code.to_string()
    }
}

pub fn fallback_prefix(tgt_lang: &str) -> String {
    let target = resolve_lang_name(tgt_lang, "");
    format!(
        "Translate the following text into {target}. Note that you should only output the translated result without any additional explanation:\n\n{{source_text}}"
    )
}

/// 言語ペアから翻訳指令を生成する。
///
/// HY-MT1.5 の推奨テンプレートに合わせて、常に英語の定型文を使う。
pub fn build_lang_prefix(
    _src_lang: &str,
    tgt_lang: &str,
    custom_name: &str,
    prompt_template: &str,
    background_text: &str,
) -> String {
    let target = resolve_lang_name(tgt_lang, custom_name);
    prompt_template
        .replace("{target}", &target)
        .replace("{target_lang}", &target)
        .replace("{language}", &target)
        .replace("{lang}", &target)
        .replace("{background_text}", background_text)
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
                "Translate the following segment into {target}, without additional explanation.",
                "",
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
                "Translate the following segment into {target}, without additional explanation.",
                "",
            ),
            "Translate the following segment into English, without additional explanation.",
        );
    }

    #[test]
    fn uses_custom_target_name_when_selected() {
        assert_eq!(
            build_lang_prefix(
                "ja",
                "pt-BR",
                "Brazilian Portuguese",
                "Translate the following segment into {target}, without additional explanation.",
                "",
            ),
            "Translate the following segment into Brazilian Portuguese, without additional explanation.",
        );
    }

    #[test]
    fn uses_custom_template_placeholders() {
        assert_eq!(
            build_lang_prefix("ja", "en", "", "Only output {language}.", ""),
            "Only output English.",
        );
    }

    #[test]
    fn replaces_background_text_placeholder() {
        assert_eq!(
            build_lang_prefix(
                "ja",
                "en",
                "",
                "Context: {background_text}\nTranslate into {target}.",
                "This is a fantasy game.",
            ),
            "Context: This is a fantasy game.\nTranslate into English.",
        );
    }
}
