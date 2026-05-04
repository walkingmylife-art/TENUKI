use crate::messages::ModelCandidateKind;
use crate::ui::container::{StatusIcon, StatusKey};

#[derive(Debug, Clone, Copy)]
pub enum NormalText {
    StatusLamp,
    NoLogsYet,
    NoEntriesYet,
    Ok,
    Exit,
    Log,
    DictNone,
    Dict,
    NoModel,
    SelectModel,
    NoModels,
    Target,
    Display,
    Network,
    Host,
    Port,
    ResetToLocal,
    NetworkAccessible,
    LocalOnly,
    CustomLanguage,
    Code,
    Name,
    ModelKnownTag,
    ModelLocalTag,
    HostPlaceholder,
    PortPlaceholder,
    CustomLanguageCodePlaceholder,
    CustomLanguageNamePlaceholder,
    MetricVram,
    MetricShared,
    MetricTokens,
    MetricDictHits,
    Url,
    DictCheckCurrent,
    DictCheckQuestion,
    DictCheckUseAsIs,
    DictCheckCreateNew,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopModeText {
    Game,
    Normal,
    List,
}

pub fn text(lang: &str, key: NormalText) -> &'static str {
    match lang {
        "ja" => ja(key),
        "zh-CN" => zh_cn(key),
        _ => en(key),
    }
}

pub fn top_mode_label(lang: &str, key: TopModeText) -> &'static str {
    match lang {
        "ja" => match key {
            TopModeText::Game => "ゲーム",
            TopModeText::Normal => "通常",
            TopModeText::List => "リスト",
        },
        "zh-CN" => match key {
            TopModeText::Game => "游戏",
            TopModeText::Normal => "普通",
            TopModeText::List => "列表",
        },
        _ => match key {
            TopModeText::Game => "Game",
            TopModeText::Normal => "Normal",
            TopModeText::List => "List",
        },
    }
}

pub fn target_language_label(ui_lang: &str, code: &str) -> &'static str {
    match ui_lang {
        "ja" => match code {
            "ja" => "日本語",
            "en" => "英語",
            "zh-CN" => "中国語（簡体字）",
            "zh-TW" => "中国語（繁体字）",
            "ko" => "韓国語",
            _ => "不明",
        },
        "zh-CN" => match code {
            "ja" => "日语",
            "en" => "英语",
            "zh-CN" => "中文（简体）",
            "zh-TW" => "中文（繁体）",
            "ko" => "韩语",
            _ => "未知",
        },
        _ => match code {
            "ja" => "Japanese",
            "en" => "English",
            "zh-CN" => "Chinese (Simplified)",
            "zh-TW" => "Chinese (Traditional)",
            "ko" => "Korean",
            _ => "Unknown",
        },
    }
}

pub fn local_host_value() -> &'static str {
    "127.0.0.1"
}

pub fn public_host_value() -> &'static str {
    "0.0.0.0"
}

pub fn model_kind_tag(lang: &str, kind: &ModelCandidateKind) -> &'static str {
    match kind {
        ModelCandidateKind::Known => text(lang, NormalText::ModelKnownTag),
        ModelCandidateKind::Local => text(lang, NormalText::ModelLocalTag),
    }
}

pub fn vram_metric(lang: &str, mb: f32) -> String {
    format!("{}: {:.0}MB", text(lang, NormalText::MetricVram), mb)
}

pub fn shared_metric(lang: &str, mb: f32) -> String {
    format!("{}: {:.0}MB", text(lang, NormalText::MetricShared), mb)
}

pub fn tokens_metric(lang: &str, tokens_per_second: f32) -> String {
    format!(
        "{}: {:.1} t/s",
        text(lang, NormalText::MetricTokens),
        tokens_per_second
    )
}

pub fn dict_hits_metric(lang: &str, hits: usize) -> String {
    format!("{}: {}", text(lang, NormalText::MetricDictHits), hits)
}

pub fn server_url(lang: &str, host: &str, port: u16) -> String {
    format!("{}: http://{}:{}", text(lang, NormalText::Url), host, port)
}

pub fn status_icon_label(icon: StatusIcon) -> &'static str {
    match icon {
        StatusIcon::None => "",
        StatusIcon::Spinner => "...",
        StatusIcon::Check => "OK",
        StatusIcon::Warning => "WARN",
    }
}

pub fn status_label(lang: &str, key: StatusKey) -> &'static str {
    match lang {
        "zh-CN" => status_zh_cn(key),
        "zh-TW" => status_zh_tw(key),
        "en" => status_en(key),
        _ => status_ja(key),
    }
}

fn en(key: NormalText) -> &'static str {
    match key {
        NormalText::StatusLamp => "●",
        NormalText::NoLogsYet => "No logs yet",
        NormalText::NoEntriesYet => "No entries yet",
        NormalText::Ok => "OK",
        NormalText::Exit => "Exit",
        NormalText::Log => "Log",
        NormalText::DictNone => "Dict[none]",
        NormalText::Dict => "Dict",
        NormalText::NoModel => "No model",
        NormalText::SelectModel => "Select model",
        NormalText::NoModels => "No models",
        NormalText::Target => "Target",
        NormalText::Display => "Display",
        NormalText::Network => "Network",
        NormalText::Host => "Host",
        NormalText::Port => "Port",
        NormalText::ResetToLocal => "Reset to local",
        NormalText::NetworkAccessible => "Network accessible (0.0.0.0)",
        NormalText::LocalOnly => "Local only (127.0.0.1)",
        NormalText::CustomLanguage => "Custom language",
        NormalText::Code => "Code",
        NormalText::Name => "Name",
        NormalText::ModelKnownTag => "[known]",
        NormalText::ModelLocalTag => "[local]",
        NormalText::HostPlaceholder => "127.0.0.1",
        NormalText::PortPlaceholder => "14371",
        NormalText::CustomLanguageCodePlaceholder => "pt-BR",
        NormalText::CustomLanguageNamePlaceholder => "Brazilian Portuguese",
        NormalText::MetricVram => "VRAM",
        NormalText::MetricShared => "Shared",
        NormalText::MetricTokens => "Tokens",
        NormalText::MetricDictHits => "Dict hits",
        NormalText::Url => "URL",
        NormalText::DictCheckCurrent => "Current:",
        NormalText::DictCheckQuestion => "Use current dictionary as-is?",
        NormalText::DictCheckUseAsIs => "Use as-is",
        NormalText::DictCheckCreateNew => "Create new",
    }
}

fn ja(key: NormalText) -> &'static str {
    match key {
        NormalText::StatusLamp => "●",
        NormalText::NoLogsYet => "ログはまだありません",
        NormalText::NoEntriesYet => "履歴はまだありません",
        NormalText::Ok => "OK",
        NormalText::Exit => "終了",
        NormalText::Log => "ログ",
        NormalText::DictNone => "辞書[なし]",
        NormalText::Dict => "辞書",
        NormalText::NoModel => "モデルなし",
        NormalText::SelectModel => "モデル選択",
        NormalText::NoModels => "モデルがありません",
        NormalText::Target => "翻訳先",
        NormalText::Display => "表示",
        NormalText::Network => "ネットワーク",
        NormalText::Host => "ホスト",
        NormalText::Port => "ポート",
        NormalText::ResetToLocal => "ローカルへ戻す",
        NormalText::NetworkAccessible => "ネットワーク公開 (0.0.0.0)",
        NormalText::LocalOnly => "ローカルのみ (127.0.0.1)",
        NormalText::CustomLanguage => "カスタム言語",
        NormalText::Code => "コード",
        NormalText::Name => "言語名",
        NormalText::ModelKnownTag => "[既知]",
        NormalText::ModelLocalTag => "[ローカル]",
        NormalText::HostPlaceholder => "127.0.0.1",
        NormalText::PortPlaceholder => "14371",
        NormalText::CustomLanguageCodePlaceholder => "pt-BR",
        NormalText::CustomLanguageNamePlaceholder => "Brazilian Portuguese",
        NormalText::MetricVram => "VRAM",
        NormalText::MetricShared => "共有",
        NormalText::MetricTokens => "トークン",
        NormalText::MetricDictHits => "辞書ヒット",
        NormalText::Url => "URL",
        NormalText::DictCheckCurrent => "現在:",
        NormalText::DictCheckQuestion => "現在の辞書をそのまま使いますか？",
        NormalText::DictCheckUseAsIs => "そのまま使う",
        NormalText::DictCheckCreateNew => "新しく作る",
    }
}

fn zh_cn(key: NormalText) -> &'static str {
    match key {
        NormalText::StatusLamp => "●",
        NormalText::NoLogsYet => "暂无日志",
        NormalText::NoEntriesYet => "暂无记录",
        NormalText::Ok => "OK",
        NormalText::Exit => "退出",
        NormalText::Log => "日志",
        NormalText::DictNone => "词典[无]",
        NormalText::Dict => "词典",
        NormalText::NoModel => "无模型",
        NormalText::SelectModel => "选择模型",
        NormalText::NoModels => "无模型",
        NormalText::Target => "翻译目标",
        NormalText::Display => "显示",
        NormalText::Network => "网络",
        NormalText::Host => "主机",
        NormalText::Port => "端口",
        NormalText::ResetToLocal => "重置为本地",
        NormalText::NetworkAccessible => "网络可访问 (0.0.0.0)",
        NormalText::LocalOnly => "仅本地 (127.0.0.1)",
        NormalText::CustomLanguage => "自定义语言",
        NormalText::Code => "代码",
        NormalText::Name => "名称",
        NormalText::ModelKnownTag => "[已知]",
        NormalText::ModelLocalTag => "[本地]",
        NormalText::HostPlaceholder => "127.0.0.1",
        NormalText::PortPlaceholder => "14371",
        NormalText::CustomLanguageCodePlaceholder => "pt-BR",
        NormalText::CustomLanguageNamePlaceholder => "Brazilian Portuguese",
        NormalText::MetricVram => "VRAM",
        NormalText::MetricShared => "共享",
        NormalText::MetricTokens => "Token",
        NormalText::MetricDictHits => "词典命中",
        NormalText::Url => "URL",
        NormalText::DictCheckCurrent => "当前:",
        NormalText::DictCheckQuestion => "是否继续使用当前词典？",
        NormalText::DictCheckUseAsIs => "继续使用",
        NormalText::DictCheckCreateNew => "新建",
    }
}

fn status_en(key: StatusKey) -> &'static str {
    match key {
        StatusKey::None => "",
        StatusKey::Ready => "Ready",
        StatusKey::Failed => "Failed",
        StatusKey::Stopped => "Stopped",
        StatusKey::Starting => "Starting...",
        StatusKey::Stopping => "Stopping...",
        StatusKey::Restarting => "Restarting...",
        StatusKey::ConfigError => "Config error",
    }
}

fn status_ja(key: StatusKey) -> &'static str {
    match key {
        StatusKey::None => "",
        StatusKey::Ready => "準備完了",
        StatusKey::Failed => "起動に失敗しました",
        StatusKey::Stopped => "停止しました",
        StatusKey::Starting => "起動しています...",
        StatusKey::Stopping => "停止しています...",
        StatusKey::Restarting => "再起動しています...",
        StatusKey::ConfigError => "設定エラー",
    }
}

fn status_zh_cn(key: StatusKey) -> &'static str {
    match key {
        StatusKey::None => "",
        StatusKey::Ready => "就绪",
        StatusKey::Failed => "失败",
        StatusKey::Stopped => "已停止",
        StatusKey::Starting => "正在启动...",
        StatusKey::Stopping => "正在停止...",
        StatusKey::Restarting => "正在重启...",
        StatusKey::ConfigError => "配置错误",
    }
}

fn status_zh_tw(key: StatusKey) -> &'static str {
    match key {
        StatusKey::None => "",
        StatusKey::Ready => "就緒",
        StatusKey::Failed => "失敗",
        StatusKey::Stopped => "已停止",
        StatusKey::Starting => "正在啟動...",
        StatusKey::Stopping => "正在停止...",
        StatusKey::Restarting => "正在重新啟動...",
        StatusKey::ConfigError => "設定錯誤",
    }
}

#[cfg(test)]
mod tests {
    use super::{target_language_label, top_mode_label, TopModeText};

    #[test]
    fn top_mode_label_uses_ui_language() {
        assert_eq!(top_mode_label("en", TopModeText::Normal), "Normal");
        assert_eq!(top_mode_label("ja", TopModeText::Normal), "通常");
        assert_eq!(top_mode_label("zh-CN", TopModeText::List), "列表");
    }

    #[test]
    fn target_language_label_uses_ui_language() {
        assert_eq!(target_language_label("en", "ja"), "Japanese");
        assert_eq!(target_language_label("ja", "en"), "英語");
        assert_eq!(target_language_label("zh-CN", "ja"), "日语");
    }
}
