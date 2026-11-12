//! Bulanık arama: kod editörlerindeki gibi harf atlayarak yazmayı affeder.
//!
//! Türkçe karakterler yok sayılır: "gozyaslari" yazınca "Gözyaşları" bulunur,
//! "İstanbul" ile "istanbul" aynıdır. Eşleştirme nucleo ile yapılıyor.

use nucleo_matcher::{
    Config, Matcher, Utf32Str,
    pattern::{CaseMatching, Normalization, Pattern},
};

/// Küçük harfe çevirir, Türkçe ve diğer aksanlı harfleri sadeleştirir.
pub fn fold(text: &str) -> String {
    text.chars()
        .flat_map(|c| {
            let folded = match c {
                'ı' | 'İ' | 'I' | 'î' | 'Î' => 'i',
                'ş' | 'Ş' => 's',
                'ğ' | 'Ğ' => 'g',
                'ç' | 'Ç' => 'c',
                'ö' | 'Ö' => 'o',
                'ü' | 'Ü' | 'û' | 'Û' => 'u',
                'â' | 'Â' => 'a',
                _ => return c.to_lowercase().collect::<Vec<_>>(),
            };
            vec![folded]
        })
        .collect()
}

/// En iyi eşleşmenin puanının bu oranından düşük sonuçlar atılır: harfleri
/// uzun bir metne dağılmış alakasız eşleşmeler listelenmesin.
const MIN_RELATIVE_SCORE: f32 = 0.7;

pub struct Fuzzy {
    matcher: Matcher,
    pattern: Pattern,
    buf: Vec<char>,
}

impl Fuzzy {
    /// Boş sorgu için `None`.
    pub fn new(query: &str) -> Option<Self> {
        let query = fold(query.trim());
        if query.is_empty() {
            return None;
        }
        Some(Self {
            matcher: Matcher::new(Config::DEFAULT),
            // Boşlukla ayrılan her kelime ayrı ayrı eşleşmeli.
            pattern: Pattern::parse(&query, CaseMatching::Ignore, Normalization::Never),
            buf: Vec::new(),
        })
    }

    /// `haystack` önceden `fold` edilmiş olmalı. Eşleşmezse `None`.
    pub fn score(&mut self, haystack: &str) -> Option<u32> {
        self.pattern
            .score(Utf32Str::new(haystack, &mut self.buf), &mut self.matcher)
    }

    /// En iyi `limit` eşleşmenin sırası (puana göre, yüksek önce).
    pub fn top<'a>(
        &mut self,
        haystacks: impl Iterator<Item = &'a str>,
        limit: usize,
    ) -> Vec<usize> {
        let mut scored: Vec<(u32, usize)> = haystacks
            .enumerate()
            .filter_map(|(i, h)| self.score(h).map(|s| (s, i)))
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let cutoff = scored
            .first()
            .map_or(0, |(best, _)| (*best as f32 * MIN_RELATIVE_SCORE) as u32);
        scored
            .into_iter()
            .take_while(|(score, _)| *score >= cutoff)
            .take(limit)
            .map(|(_, i)| i)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_turkish() {
        assert_eq!(fold("Gözyaşları ŞİMDİ Iğdır"), "gozyaslari simdi igdir");
        assert_eq!(fold("Çağrı Ünlü"), "cagri unlu");
    }

    #[test]
    fn matches_with_skipped_letters_and_without_turkish_chars() {
        let items = [
            fold("Bir Pesimistin Gözyaşları Sagopa Kajmer"),
            fold("Bring Me To Life Evanescence"),
            fold("Gece Yarısı"),
        ];
        let mut f = Fuzzy::new("sgp gzy").unwrap();
        assert_eq!(f.top(items.iter().map(String::as_str), 5), [0]);

        let mut f = Fuzzy::new("gozyaslari").unwrap();
        assert_eq!(f.top(items.iter().map(String::as_str), 5), [0]);

        let mut f = Fuzzy::new("GECE yarisi").unwrap();
        assert_eq!(f.top(items.iter().map(String::as_str), 5), [2]);

        assert!(Fuzzy::new("   ").is_none());
    }

    #[test]
    fn drops_scattered_matches() {
        // Gerçek kütüphanede "evanescence" araması, harfleri uzun klasik müzik
        // başlıklarına dağılmış alakasız şarkıları da getiriyordu.
        let items = [
            fold("Bring Me To Life Evanescence Fallen"),
            fold("Lithium Evanescence The Open Door"),
            fold("Delius: 2 Pieces for Small Orchestra: No. 1, On Hearing the First Cuckoo in Spring Frederick Delius, London Philharmonic Orchestra, Vernon Handley The Lark Ascending Collection"),
            fold("Kinderszenen, Op. 15: VII. Träumerei Robert Schumann, Marc-André Hamelin Schumann: Scenes from Childhood; Waldszenen – Janáček: On the Overgrown Path"),
        ];
        let mut f = Fuzzy::new("evanescence").unwrap();
        assert_eq!(f.top(items.iter().map(String::as_str), 8), [0, 1]);
    }
}
