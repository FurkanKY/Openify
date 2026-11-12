//! Tasarım kurallarının testi (Adım 9, "Arayüz ve deneyim çıtası").
//!
//! Koddan sayılabilen kurallar burada sıfırda tutulur. Bir ekran bu
//! kurallardan birini bozarsa test kırılır ve hangi satır olduğunu söyler:
//! - boşluklar ve köşe yarıçapları `theme.rs`'teki ölçekten (sayı yazılmaz);
//! - yazı boyutu ve satır yüksekliği yalnızca yazı rollerinden (`type_role`);
//! - gölge yalnızca `float_shadow` (yüzen öğeler) ve odak halkası;
//! - renkler `theme.rs`'ten (beyaz ve siyah saydamlıklar `veil` / `shade`);
//! - sadece simgeden oluşan her düğmenin bir ipucu var.

#[cfg(test)]
mod tests {
    /// `theme.rs` dışındaki arayüz dosyaları.
    const FILES: &[(&str, &str)] = &[
        ("components.rs", include_str!("components.rs")),
        ("library.rs", include_str!("library.rs")),
        ("library_layout.rs", include_str!("library_layout.rs")),
        ("mod.rs", include_str!("mod.rs")),
        ("palette.rs", include_str!("palette.rs")),
        ("pill.rs", include_str!("pill.rs")),
        ("pill_glass.rs", include_str!("pill_glass.rs")),
        ("player.rs", include_str!("player.rs")),
        ("shelf.rs", include_str!("shelf.rs")),
        ("text.rs", include_str!("text.rs")),
        ("gallery.rs", include_str!("gallery.rs")),
        ("views.rs", include_str!("views.rs")),
        ("integration.rs", include_str!("integration.rs")),
        ("popups.rs", include_str!("popups.rs")),
        ("now_playing.rs", include_str!("now_playing.rs")),
        ("queue_panel.rs", include_str!("queue_panel.rs")),
        ("lyrics_view.rs", include_str!("lyrics_view.rs")),
        ("now_panel.rs", include_str!("now_panel.rs")),
        ("fullscreen.rs", include_str!("fullscreen.rs")),
        // Adım 12–13: sayfalar, gezinme, kütüphane yönetimi.
        ("pages.rs", include_str!("pages.rs")),
        ("manage.rs", include_str!("manage.rs")),
        ("history.rs", include_str!("history.rs")),
        ("explore.rs", include_str!("explore.rs")),
        ("home.rs", include_str!("home.rs")),
        ("link_text.rs", include_str!("link_text.rs")),
        // Adım 15: ayarlar sayfası, profil menüsü.
        ("settings.rs", include_str!("settings.rs")),
        // Kısayol yardımı katmanı.
        ("shortcuts.rs", include_str!("shortcuts.rs")),
        // Arama çipleri ve son çalınanlar sayfası.
        ("search_page.rs", include_str!("search_page.rs")),
        ("recent_page.rs", include_str!("recent_page.rs")),
        // Karşılama ve boş durumlar.
        ("welcome.rs", include_str!("welcome.rs")),
        // Arkadaş etkinliği paneli.
        ("friends.rs", include_str!("friends.rs")),
        // Kütüphane araçları sayfası.
        ("tools.rs", include_str!("tools.rs")),
        ("tools_view.rs", include_str!("tools_view.rs")),
        // Şarkı künyesi penceresi.
        ("credits.rs", include_str!("credits.rs")),
        // Ekolayzer bölümü.
        ("equalizer.rs", include_str!("equalizer.rs")),
    ];

    fn violations(rule: impl Fn(&str) -> bool) -> Vec<String> {
        let mut found = Vec::new();
        for (file, src) in FILES {
            for (n, line) in src.lines().enumerate() {
                let code = line.split("//").next().unwrap_or("");
                if rule(code) {
                    found.push(format!("{file}:{}: {}", n + 1, line.trim()));
                }
            }
        }
        found
    }

    /// `name(px(` ardından sayı (ya da eksi sayı) geliyor mu?
    fn literal_px(code: &str, name: &str) -> bool {
        let needle = format!(".{name}(px(");
        code.match_indices(&needle).any(|(i, _)| {
            let rest = code[i + needle.len()..].trim_start_matches('-');
            rest.starts_with(|c: char| c.is_ascii_digit())
        })
    }

    #[test]
    fn spacing_comes_from_the_scale() {
        let methods = [
            "gap", "p", "px", "py", "pt", "pb", "pl", "pr", "m", "mx", "my", "mt", "mb", "ml", "mr",
        ];
        let v = violations(|code| methods.iter().any(|m| literal_px(code, m)));
        assert!(v.is_empty(), "ölçek dışı boşluk:\n{}", v.join("\n"));
    }

    #[test]
    fn radius_comes_from_the_scale() {
        let v = violations(|code| literal_px(code, "rounded"));
        assert!(v.is_empty(), "ölçek dışı köşe yarıçapı:\n{}", v.join("\n"));
    }

    #[test]
    fn type_comes_from_roles() {
        // Stil olarak verilen boyut (`window.line_height()` gibi okumalar değil).
        let v =
            violations(|code| code.contains(".text_size(") || code.contains(".line_height(px("));
        assert!(v.is_empty(), "yazı rolü dışında boyut:\n{}", v.join("\n"));
    }

    #[test]
    fn shadows_only_for_floating_elements() {
        let v = violations(|code| {
            code.contains(".shadow_sm()")
                || code.contains(".shadow_md()")
                || code.contains(".shadow_lg()")
                || code.contains(".shadow_xl()")
        });
        assert!(
            v.is_empty(),
            "float_shadow dışında gölge:\n{}",
            v.join("\n")
        );
    }

    #[test]
    fn colors_come_from_the_theme() {
        let v = violations(|code| code.contains("with_alpha(0x") || code.contains("rgba(0x"));
        assert!(v.is_empty(), "theme.rs dışında ham renk:\n{}", v.join("\n"));
    }

    /// Klavye odağı (Adım 15 sonrası): Tab sırası görsel sırayı izler
    /// (başlık çubuğu, sayfa, panel, hap) ve her durak ayrı bir sıra sayısı
    /// alır ya da bilerek komşusuyla paylaşır; sayılar azalmaz.
    #[test]
    fn tab_order_follows_the_visual_order() {
        let order = super::super::tab::ORDER;
        for pair in order.windows(2) {
            let ((a, ia), (b, ib)) = (pair[0], pair[1]);
            assert!(
                ia <= ib,
                "Tab sırası görsel sırayı bozuyor: {a} ({ia}) sonra {b} ({ib})"
            );
        }
        // Gruplar arası boşluk: yeni duraklar araya sığsın.
        let group = |name: &str| order.iter().find(|(n, _)| *n == name).unwrap().1;
        assert!(
            group("kapat") < group("raf")
                && group("sayfa") < group("panel")
                && group("panel") < group("kalp")
        );
    }

    /// Ayarlar sayfasındaki her etkileşimli denetim klavye durağıdır (Tab,
    /// Enter / Boşluk) ve odak halkasını çizer: `Look::Live` sabitiyle
    /// verilmez, `stop_look` sonucu verilir. Onay kutusu kendi tuşlarını
    /// (`settings_key`) işler, muaf.
    #[test]
    fn settings_controls_take_focus() {
        let src = include_str!("settings.rs");
        let mut bad = Vec::new();
        for (n, line) in src.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            let call = ["switch(", "segment(", "select_field(", "secondary_button("]
                .iter()
                .any(|c| {
                    code.contains(&format!("= {c}"))
                        || code.trim_start().starts_with(c)
                        || code.contains(&format!("({c}"))
                });
            if call
                && !code.contains("fn ")
                && !code.contains("confirm-")
                && code.contains("Look::Live")
            {
                bad.push(format!("settings.rs:{}: {}", n + 1, line.trim()));
            }
        }
        assert!(
            bad.is_empty(),
            "klavye odağı olmayan ayar denetimi:\n{}",
            bad.join("\n")
        );
        // Çok satırlı çağrılar için: Look::Live yerine stop_look sonucu.
        let stops = src.matches("self.stop_look(").count();
        assert!(stops >= 10, "ayarlardaki Tab durakları eksik ({stops})");
    }

    #[test]
    fn icon_buttons_have_tooltips() {
        let mut missing = Vec::new();
        for (file, src) in FILES {
            if *file == "components.rs" || *file == "gallery.rs" {
                continue;
            }
            let lines: Vec<&str> = src.lines().collect();
            for (n, line) in lines.iter().enumerate() {
                if line.contains("icon_button(\"") {
                    let window = lines[n..(n + 4).min(lines.len())].join(" ");
                    if !window.contains(".tooltip(") {
                        missing.push(format!("{file}:{}: {}", n + 1, line.trim()));
                    }
                }
            }
        }
        assert!(
            missing.is_empty(),
            "ipucusuz simge düğmesi:\n{}",
            missing.join("\n")
        );
    }
}
