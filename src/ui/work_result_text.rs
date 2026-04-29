use crate::ui::container::InputAnalysisRecord;

#[derive(Debug, Clone, Copy)]
enum WorkResultText {
    PickupTitle,
    PickupEmpty,
    Raw,
    Extracted,
    Visible,
    ModelInput,
    Note,
    EmptyValue,
    Separator,
}

pub fn pickup_preview(ui_lang: &str, records: &[InputAnalysisRecord]) -> (String, String) {
    if records.is_empty() {
        return (
            text(ui_lang, WorkResultText::PickupTitle).to_string(),
            text(ui_lang, WorkResultText::PickupEmpty).to_string(),
        );
    }

    let sections = records
        .iter()
        .map(|record| format_pickup_record_preview(ui_lang, record))
        .collect::<Vec<_>>();
    (
        pickup_entries_title(ui_lang, records.len()),
        sections.join(text(ui_lang, WorkResultText::Separator)),
    )
}

fn pickup_entries_title(ui_lang: &str, count: usize) -> String {
    match ui_lang {
        "ja" => format!("pickup {} 件", count),
        "zh-CN" => format!("pickup {} 条", count),
        _ => format!("pickup {} entries", count),
    }
}

fn format_pickup_record_preview(ui_lang: &str, record: &InputAnalysisRecord) -> String {
    format!(
        "[{}] {}\n{}: {}\n{}: {}\n{}: {}\n{}: {}\n{}: {}",
        record.timestamp,
        pickup_occurrence_label(record),
        text(ui_lang, WorkResultText::Raw),
        record.snapshot.raw_text,
        text(ui_lang, WorkResultText::Extracted),
        record.snapshot.extracted_text,
        text(ui_lang, WorkResultText::Visible),
        nonempty_or_dash(ui_lang, &record.snapshot.visible_text),
        text(ui_lang, WorkResultText::ModelInput),
        model_inputs(ui_lang, record),
        text(ui_lang, WorkResultText::Note),
        nonempty_or_dash(ui_lang, &record.note)
    )
}

fn model_inputs(ui_lang: &str, record: &InputAnalysisRecord) -> String {
    if record.snapshot.model_inputs.is_empty() {
        text(ui_lang, WorkResultText::EmptyValue).to_string()
    } else {
        record.snapshot.model_inputs.join(" | ")
    }
}

fn nonempty_or_dash(ui_lang: &str, value: &str) -> String {
    if value.trim().is_empty() {
        text(ui_lang, WorkResultText::EmptyValue).to_string()
    } else {
        value.to_string()
    }
}

fn pickup_occurrence_label(record: &InputAnalysisRecord) -> String {
    if record.occurrences > 1 {
        format!("x{}", record.occurrences)
    } else {
        "x1".to_string()
    }
}

fn text(ui_lang: &str, key: WorkResultText) -> &'static str {
    match ui_lang {
        "ja" => ja(key),
        "zh-CN" => zh_cn(key),
        _ => en(key),
    }
}

fn en(key: WorkResultText) -> &'static str {
    match key {
        WorkResultText::PickupTitle => "pickup",
        WorkResultText::PickupEmpty => "No pickup entries yet",
        WorkResultText::Raw => "raw",
        WorkResultText::Extracted => "extracted",
        WorkResultText::Visible => "visible",
        WorkResultText::ModelInput => "model input",
        WorkResultText::Note => "note",
        WorkResultText::EmptyValue => "-",
        WorkResultText::Separator => "\n\n--------------------------------\n\n",
    }
}

fn ja(key: WorkResultText) -> &'static str {
    match key {
        WorkResultText::PickupTitle => "pickup",
        WorkResultText::PickupEmpty => "pickup はまだありません",
        WorkResultText::Raw => "原文",
        WorkResultText::Extracted => "抽出",
        WorkResultText::Visible => "表示テキスト",
        WorkResultText::ModelInput => "モデル入力",
        WorkResultText::Note => "メモ",
        WorkResultText::EmptyValue => "-",
        WorkResultText::Separator => "\n\n--------------------------------\n\n",
    }
}

fn zh_cn(key: WorkResultText) -> &'static str {
    match key {
        WorkResultText::PickupTitle => "pickup",
        WorkResultText::PickupEmpty => "暂无 pickup 条目",
        WorkResultText::Raw => "原文",
        WorkResultText::Extracted => "提取",
        WorkResultText::Visible => "可见文本",
        WorkResultText::ModelInput => "模型输入",
        WorkResultText::Note => "备注",
        WorkResultText::EmptyValue => "-",
        WorkResultText::Separator => "\n\n--------------------------------\n\n",
    }
}

#[cfg(test)]
mod tests {
    use super::pickup_preview;
    use crate::messages::InputAnalysisSnapshot;
    use crate::ui::container::InputAnalysisRecord;

    fn record() -> InputAnalysisRecord {
        InputAnalysisRecord {
            id: 1,
            timestamp: "2026-04-23 10:00:00".to_string(),
            snapshot: InputAnalysisSnapshot {
                raw_text: "RAW".to_string(),
                extracted_text: "EXTRACTED".to_string(),
                visible_text: "VISIBLE".to_string(),
                model_inputs: vec!["MODEL".to_string()],
                final_output: None,
                result_stale: false,
                dict_hits: 0,
                model_calls: 0,
            },
            occurrences: 2,
            pickup: true,
            note: "NOTE".to_string(),
        }
    }

    #[test]
    fn pickup_preview_formats_empty_state() {
        let (title, text) = pickup_preview("en", &[]);
        assert_eq!(title, "pickup");
        assert_eq!(text, "No pickup entries yet");
    }

    #[test]
    fn pickup_preview_uses_ui_language_for_labels() {
        let records = vec![record()];
        let (title, text) = pickup_preview("ja", &records);
        assert_eq!(title, "pickup 1 件");
        assert!(text.contains("原文: RAW"));
        assert!(text.contains("モデル入力: MODEL"));
    }
}
