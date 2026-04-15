// src/ui/fonts.rs
use eframe::egui;

pub fn setup_fonts(cc: &eframe::CreationContext) {
    let mut fonts = egui::FontDefinitions::default();

    // 1. 各 Variable Font (TTF) を登録
    // 全部で約15-20MB程度。以前の巨大なOTF1本とほぼ変わらないサイズで、対応言語は数倍です。
    fonts.font_data.insert("noto_jp".to_owned(), 
        egui::FontData::from_static(include_bytes!("../../assets/fonts/noto_jp.ttf")));
    fonts.font_data.insert("noto_sc".to_owned(), 
        egui::FontData::from_static(include_bytes!("../../assets/fonts/noto_sc.ttf")));
    fonts.font_data.insert("noto_tc".to_owned(), 
        egui::FontData::from_static(include_bytes!("../../assets/fonts/noto_tc.ttf")));
    fonts.font_data.insert("noto_latin".to_owned(), 
        egui::FontData::from_static(include_bytes!("../../assets/fonts/noto_latin.ttf")));
    fonts.font_data.insert("noto_thai".to_owned(), 
        egui::FontData::from_static(include_bytes!("../../assets/fonts/noto_thai.ttf")));
    fonts.font_data.insert("noto_arabic".to_owned(), 
        egui::FontData::from_static(include_bytes!("../../assets/fonts/noto_arabic.ttf")));

    // 2. 優先順位（フォールバック）の設定
    let p = fonts.families.entry(egui::FontFamily::Proportional).or_default();
    p.clear();
    p.push("noto_jp".to_owned());     // メインは日本語
    p.push("noto_sc".to_owned());     // 日本語にない簡体字をカバー
    p.push("noto_tc".to_owned());     // 繁体字をカバー
    p.push("noto_latin".to_owned());  // 英語・ベトナム語をカバー
    p.push("noto_thai".to_owned());   // タイ語をカバー
    p.push("noto_arabic".to_owned()); // アラビア語をカバー

    // Monospace ファミリもフォールバックを設定（ログやコード表示で tofu を防ぐ）
    let m = fonts.families.entry(egui::FontFamily::Monospace).or_default();
    m.clear();
    m.push("noto_latin".to_owned());
    m.push("noto_jp".to_owned());
    m.push("noto_sc".to_owned());
    m.push("noto_tc".to_owned());
    m.push("noto_thai".to_owned());
    m.push("noto_arabic".to_owned());

    cc.egui_ctx.set_fonts(fonts);

    // スタイル設定
    let mut style = (*cc.egui_ctx.style()).clone();
    // 基本の本文は14pt。小さいテキストや等幅も14に設定して言語切替の入力等の文字が小さくならないようにする
    style.text_styles.insert(egui::TextStyle::Body, egui::FontId::new(14.0, egui::FontFamily::Proportional));
    style.text_styles.insert(egui::TextStyle::Small, egui::FontId::new(14.0, egui::FontFamily::Proportional));
    style.text_styles.insert(egui::TextStyle::Monospace, egui::FontId::new(14.0, egui::FontFamily::Monospace));
    style.text_styles.insert(egui::TextStyle::Button, egui::FontId::new(14.0, egui::FontFamily::Proportional));
    style.text_styles.insert(egui::TextStyle::Heading, egui::FontId::new(18.0, egui::FontFamily::Proportional));
    cc.egui_ctx.set_style(style);
}