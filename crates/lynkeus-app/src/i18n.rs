// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Testi dell'interfaccia con Fluent: `locales/it/main.ftl` e `locales/en/main.ftl`,
//! inclusi nell'eseguibile.
//!
//! Ogni stringa visibile passa da [`I18n::tr`] o [`I18n::tr_args`]; gli ID sono stabili e le
//! due lingue hanno gli stessi ID (verificato da un test). L'isolamento bidi di Fluent resta
//! attivo: egui disegna i caratteri di isolamento come invisibili.
//!
//! Fluent non formatta i numeri per lingua: un formatter aggiunto a ogni bundle raggruppa
//! le cifre degli interi ("200.000" in italiano, "200,000" in inglese). I plurali restano
//! di Fluent, che li sceglie dal valore numerico. Un ID mancante o un errore di
//! formattazione si registra nel log una volta per ID e lingua.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use fluent_templates::fluent_bundle::FluentValue;
use fluent_templates::{LanguageIdentifier, Loader, langid};
use serde::{Deserialize, Serialize};

fluent_templates::static_loader! {
    static LOCALES = {
        locales: "./locales",
        fallback_language: "it",
        customise: |bundle| {
            let language = bundle
                .locales
                .first()
                .map_or(Language::Italian, |id| Language::from_tag(id.language.as_str()));
            bundle.set_formatter(Some(match language {
                Language::Italian => format_italian_number,
                Language::English => format_english_number,
            }));
        },
    };
}

/// Errori di traduzione registrati al massimo per [`I18n`]: oltre, il log non cresce.
const MAX_REPORTED: usize = 256;

/// Formatter dei numeri del bundle italiano (la firma è quella di Fluent).
fn format_italian_number<M>(value: &FluentValue<'_>, _memoizer: &M) -> Option<String> {
    group_digits(value, Language::Italian.digit_group_separator())
}

/// Formatter dei numeri del bundle inglese (la firma è quella di Fluent).
fn format_english_number<M>(value: &FluentValue<'_>, _memoizer: &M) -> Option<String> {
    group_digits(value, Language::English.digit_group_separator())
}

/// Un intero con le cifre raggruppate a tre a tre da `separator`; `None` (formato di
/// Fluent) per testi, numeri con decimali o con il raggruppamento spento.
fn group_digits(value: &FluentValue<'_>, separator: char) -> Option<String> {
    let FluentValue::Number(number) = value else {
        return None;
    };
    let options = &number.options;
    let integer = number.value.is_finite() && number.value.fract().abs() < f64::EPSILON;
    if !options.use_grouping || options.minimum_fraction_digits.is_some() || !integer {
        return None;
    }
    let digits = format!("{:.0}", number.value.abs());
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    if number.value.is_sign_negative() && digits != "0" {
        out.push('-');
    }
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(separator);
        }
        out.push(digit);
    }
    Some(out)
}

/// Lingua dell'interfaccia.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Language {
    /// Italiano.
    Italian,
    /// Inglese.
    English,
}

impl Language {
    /// Tutte le lingue disponibili.
    pub const ALL: [Language; 2] = [Language::Italian, Language::English];

    /// La lingua del sistema se è l'italiano, altrimenti l'inglese.
    pub fn from_system() -> Language {
        sys_locale::get_locale().map_or(Language::English, |tag| Language::from_tag(&tag))
    }

    /// Lingua per un tag BCP 47 o POSIX ("it-IT", "it_CH", "en-US"): italiano se inizia
    /// con "it", altrimenti inglese.
    pub fn from_tag(tag: &str) -> Language {
        let primary = tag.split(['-', '_', '.']).next().unwrap_or_default();
        if primary.eq_ignore_ascii_case("it") {
            Language::Italian
        } else {
            Language::English
        }
    }

    fn identifier(self) -> LanguageIdentifier {
        match self {
            Language::Italian => langid!("it"),
            Language::English => langid!("en"),
        }
    }

    /// Il separatore delle migliaia: "200.000" in italiano, "200,000" in inglese.
    pub fn digit_group_separator(self) -> char {
        match self {
            Language::Italian => '.',
            Language::English => ',',
        }
    }
}

/// Argomenti di un messaggio: nome della variabile e valore. I numeri si passano come
/// numeri (non stringhe già formattate), così Fluent sceglie il plurale giusto e li
/// formatta per la lingua.
pub type Args<'a> = &'a [(&'static str, FluentValue<'static>)];

/// Traduttore per una lingua. I cloni condividono l'elenco degli errori già registrati.
#[derive(Debug, Clone)]
pub struct I18n {
    language: Language,
    identifier: LanguageIdentifier,
    /// ID già registrati come non risolti in questa lingua.
    reported: Arc<Mutex<HashSet<String>>>,
}

impl I18n {
    /// Traduttore per `language`.
    pub fn new(language: Language) -> I18n {
        I18n {
            language,
            identifier: language.identifier(),
            reported: Arc::default(),
        }
    }

    /// La lingua in uso.
    pub fn language(&self) -> Language {
        self.language
    }

    /// Il messaggio `id`. Un ID sconosciuto o un errore di formattazione (un bug, coperto
    /// dai test) si registra nel log una volta e restituisce il testo italiano o, se manca
    /// anche quello, l'ID stesso: si nota senza fermare l'app.
    pub fn tr(&self, id: &str) -> String {
        self.tr_args(id, &[])
    }

    /// Il messaggio `id` con gli argomenti `args`, come [`I18n::tr`].
    pub fn tr_args(&self, id: &str, args: Args<'_>) -> String {
        let args = to_map(args);
        let found = LOCALES.lookup_single_language(&self.identifier, id, args.as_ref());
        let error = match found {
            Ok(text) => return text,
            Err(error) => error,
        };
        self.report(id, &error);
        LOCALES
            .lookup_single_language(LOCALES.fallback(), id, args.as_ref())
            .unwrap_or_else(|_| id.to_owned())
    }

    /// Come [`I18n::tr_args`], ma `None` (senza log) se l'ID manca o la formattazione dà
    /// errori.
    pub fn try_tr_args(&self, id: &str, args: Args<'_>) -> Option<String> {
        let args = to_map(args);
        LOCALES.try_lookup_complete(&self.identifier, id, args.as_ref())
    }

    /// Registra nel log che `id` non si è risolto, la prima volta.
    fn report(&self, id: &str, error: &impl std::fmt::Display) {
        let Ok(mut reported) = self.reported.lock() else {
            return;
        };
        if reported.len() >= MAX_REPORTED || reported.contains(id) {
            return;
        }
        reported.insert(id.to_owned());
        tracing::warn!(id, language = ?self.language, %error, "testo dell'interfaccia non risolto");
    }
}

/// Gli argomenti come li vuole Fluent; `None` senza argomenti.
fn to_map(args: Args<'_>) -> Option<HashMap<Cow<'static, str>, FluentValue<'static>>> {
    (!args.is_empty()).then(|| {
        args.iter()
            .map(|(name, value)| (Cow::Borrowed(*name), value.clone()))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    const IT: &str = include_str!("../locales/it/main.ftl");
    const EN: &str = include_str!("../locales/en/main.ftl");

    /// ID dei messaggi e dei termini (`-nome`) di un file `.ftl`.
    fn ids(source: &str) -> BTreeSet<String> {
        source
            .lines()
            .filter(|line| line.starts_with(|c: char| c.is_ascii_alphabetic() || c == '-'))
            .filter_map(|line| line.split_once('='))
            .map(|(id, _)| id.trim().to_owned())
            .collect()
    }

    #[test]
    fn both_languages_have_the_same_ids() {
        let it = ids(IT);
        let en = ids(EN);
        assert!(!it.is_empty());
        let only_it: Vec<_> = it.difference(&en).collect();
        let only_en: Vec<_> = en.difference(&it).collect();
        assert!(
            only_it.is_empty() && only_en.is_empty(),
            "solo in it: {only_it:?}; solo in en: {only_en:?}"
        );
    }

    /// Nomi delle variabili (`$nome`) usate nei messaggi.
    fn variables(source: &str) -> BTreeSet<String> {
        source
            .split('$')
            .skip(1)
            .map(|rest| {
                rest.chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                    .collect::<String>()
            })
            .filter(|name| !name.is_empty())
            .collect()
    }

    #[test]
    fn every_message_resolves_in_both_languages() {
        // Ogni variabile vale 1: basta perché nessun messaggio dia errori di formattazione.
        let args: HashMap<Cow<'static, str>, FluentValue<'static>> = variables(IT)
            .into_iter()
            .map(|name| (Cow::Owned(name), FluentValue::from(1)))
            .collect();
        for language in Language::ALL {
            for id in ids(IT).iter().filter(|id| !id.starts_with('-')) {
                let found = LOCALES.lookup_single_language(&language.identifier(), id, Some(&args));
                assert!(found.is_ok(), "{id} in {language:?}: {found:?}");
            }
        }
    }

    #[test]
    fn language_from_system_tags() {
        assert_eq!(Language::from_tag("it-IT"), Language::Italian);
        assert_eq!(Language::from_tag("it_CH.UTF-8"), Language::Italian);
        assert_eq!(Language::from_tag("en-US"), Language::English);
        assert_eq!(Language::from_tag("de"), Language::English);
        assert_eq!(Language::from_tag(""), Language::English);
    }

    /// Toglie i caratteri d'isolamento bidi che Fluent mette intorno alle variabili.
    fn plain(text: &str) -> String {
        text.replace(['\u{2068}', '\u{2069}'], "")
    }

    /// T-C05 (C-05): i numeri si raggruppano come nella lingua dell'interfaccia.
    #[test]
    fn numbers_grouped_per_language() {
        let count = |language, n: u64| {
            plain(&I18n::new(language).tr_args("tab-results", &[("count", n.into())]))
        };
        assert_eq!(count(Language::Italian, 200_000), "Risultati (200.000)");
        assert_eq!(count(Language::English, 200_000), "Results (200,000)");
        assert_eq!(count(Language::Italian, 1_234), "Risultati (1.234)");
        assert_eq!(count(Language::English, 1_234_567), "Results (1,234,567)");
        assert_eq!(count(Language::Italian, 999), "Risultati (999)");
        assert_eq!(count(Language::English, 0), "Results (0)");
    }

    proptest::proptest! {
        /// Andata e ritorno: tolti i separatori restano le cifre; i gruppi dopo il primo
        /// hanno tre cifre, il primo da una a tre.
        #[test]
        fn grouped_digits_round_trip(n in 0u64..(1 << 53)) {
            #[expect(clippy::cast_precision_loss, reason = "n < 2^53: rappresentazione esatta")]
            let value = FluentValue::from(n as f64);
            let grouped = group_digits(&value, '.').unwrap_or_default();
            proptest::prop_assert_eq!(grouped.replace('.', ""), n.to_string());
            let groups: Vec<&str> = grouped.split('.').collect();
            proptest::prop_assert!((1..=3).contains(&groups[0].len()));
            proptest::prop_assert!(groups[1..].iter().all(|group| group.len() == 3));
        }
    }

    /// Un `Write` che conserva ciò che riceve, per leggere il log nei test.
    #[derive(Clone, Default)]
    struct Captured(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for Captured {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if let Ok(mut buffer) = self.0.lock() {
                buffer.extend_from_slice(bytes);
            }
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// C-18: un ID mancante mostra l'ID e finisce nel log una volta sola, anche se la
    /// vista lo chiede a ogni frame.
    #[test]
    fn missing_id_is_logged_once() {
        let captured = Captured::default();
        let writer = captured.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .finish();
        let i18n = I18n::new(Language::English);
        tracing::subscriber::with_default(subscriber, || {
            for _ in 0..3 {
                assert_eq!(i18n.tr("id-che-non-esiste-c18"), "id-che-non-esiste-c18");
            }
        });
        let log = captured
            .0
            .lock()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default();
        let lines = log
            .lines()
            .filter(|line| line.contains("id-che-non-esiste-c18"))
            .count();
        assert_eq!(lines, 1, "{log}");
    }

    #[test]
    fn plural_and_arguments() {
        let it = I18n::new(Language::Italian);
        let one = it.tr_args("location-line", &[("line", FluentValue::from(12))]);
        assert!(one.contains("12"), "{one}");
    }
}
