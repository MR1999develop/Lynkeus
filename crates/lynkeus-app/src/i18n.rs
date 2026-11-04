// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Testi dell'interfaccia con Fluent: `locales/it/main.ftl` e `locales/en/main.ftl`,
//! inclusi nell'eseguibile.
//!
//! Ogni stringa visibile passa da [`I18n::tr`] o [`I18n::tr_args`]; gli ID sono stabili e le
//! due lingue hanno gli stessi ID (verificato da un test). L'isolamento bidi di Fluent resta
//! attivo: egui disegna i caratteri di isolamento come invisibili.

use std::borrow::Cow;
use std::collections::HashMap;

use fluent_templates::fluent_bundle::FluentValue;
use fluent_templates::{LanguageIdentifier, Loader, langid};
use serde::{Deserialize, Serialize};

fluent_templates::static_loader! {
    static LOCALES = {
        locales: "./locales",
        fallback_language: "it",
    };
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
}

/// Argomenti di un messaggio: nome della variabile e valore. I numeri si passano come
/// numeri (non stringhe già formattate), così Fluent sceglie il plurale giusto.
pub type Args<'a> = &'a [(&'static str, FluentValue<'static>)];

/// Traduttore per una lingua.
#[derive(Debug, Clone)]
pub struct I18n {
    language: Language,
    identifier: LanguageIdentifier,
}

impl I18n {
    /// Traduttore per `language`.
    pub fn new(language: Language) -> I18n {
        I18n {
            language,
            identifier: language.identifier(),
        }
    }

    /// La lingua in uso.
    pub fn language(&self) -> Language {
        self.language
    }

    /// Il messaggio `id`. Un ID sconosciuto (un bug, coperto dai test) restituisce l'ID
    /// stesso, così si nota senza fermare l'app.
    pub fn tr(&self, id: &str) -> String {
        self.tr_args(id, &[])
    }

    /// Il messaggio `id` con gli argomenti `args`.
    pub fn tr_args(&self, id: &str, args: Args<'_>) -> String {
        self.try_tr_args(id, args).unwrap_or_else(|| id.to_owned())
    }

    /// Come [`I18n::tr_args`], ma `None` se l'ID manca o la formattazione dà errori.
    pub fn try_tr_args(&self, id: &str, args: Args<'_>) -> Option<String> {
        if args.is_empty() {
            return LOCALES.try_lookup(&self.identifier, id);
        }
        let map: HashMap<Cow<'static, str>, FluentValue<'static>> = args
            .iter()
            .map(|(name, value)| (Cow::Borrowed(*name), value.clone()))
            .collect();
        LOCALES.try_lookup_with_args(&self.identifier, id, &map)
    }
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

    #[test]
    fn plural_and_arguments() {
        let it = I18n::new(Language::Italian);
        let one = it.tr_args("location-line", &[("line", FluentValue::from(12))]);
        assert!(one.contains("12"), "{one}");
    }
}
