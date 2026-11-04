// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Termini, opzioni di confronto e query compilata.
//!
//! Un [`Term`] esiste solo se valido; una [`Query`] ha da 1 a [`Query::MAX_TERMS`] termini,
//! tutti validati con la stessa [`WildcardMode`] delle opzioni. [`Query::compile`] trasforma
//! ogni termine in una regex del crate `regex` (tempo lineare): il testo dell'utente non
//! diventa mai una regex grezza.

use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

/// Maiuscole e minuscole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum CaseMode {
    /// "ar" trova anche "AR" e "Ar" (predefinito).
    #[default]
    Insensitive,
    /// "ar" trova solo "ar".
    Sensitive,
}

/// Lettere accentate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum AccentMode {
    /// "città" non trova "citta" (predefinito); una lettera accentata del termine trova
    /// anche la sua forma decomposta (NFD) nel testo.
    #[default]
    Distinct,
    /// "citta" trova anche "città" e viceversa.
    Ignored,
}

/// Significato di `*` e `?` nei termini.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum WildcardMode {
    /// `*` = zero o più caratteri non spazio, `?` = un carattere non spazio (predefinito).
    #[default]
    Enabled,
    /// `*` e `?` sono caratteri letterali.
    Literal,
}

/// Regole di confronto comuni a tutti i termini di una query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct MatchOptions {
    /// Maiuscole ignorate o distinte.
    pub case: CaseMode,
    /// Accenti distinti o ignorati.
    pub accents: AccentMode,
    /// Jolly attivi o letterali.
    pub wildcards: WildcardMode,
}

/// Come si combinano i termini all'interno di un file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Combine {
    /// Una riga compare se corrisponde almeno un termine (OR, predefinito).
    #[default]
    Any,
    /// Un file compare solo se contiene tutti i termini (AND); un file dentro un archivio
    /// conta come file a sé.
    All,
}

/// Motivo per cui un termine non è valido; l'interfaccia lo mostra sotto il campo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TermError {
    /// Vuoto o fatto solo di spazi.
    #[error("termine vuoto")]
    Empty,
    /// Fatto solo di `*`, `?` e spazi, con i jolly attivi: troverebbe ogni parola.
    #[error("termine fatto solo di caratteri jolly")]
    OnlyWildcards,
    /// Più lungo di `max_chars` caratteri.
    #[error("termine più lungo di {max_chars} caratteri")]
    TooLong {
        /// Il limite superato, [`Term::MAX_CHARS`].
        max_chars: usize,
    },
    /// Contiene un a capo: nessuna corrispondenza attraversa una riga, quindi non troverebbe nulla.
    #[error("termine con un a capo")]
    LineBreak,
}

/// Un termine di ricerca valido, così come l'ha scritto l'utente (spazi compresi).
///
/// Ricorda la [`WildcardMode`] con cui è stato validato, perché con i jolly letterali
/// "*" è un termine valido e con i jolly attivi no.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Term {
    raw: String,
    wildcards: WildcardMode,
}

impl Term {
    /// Lunghezza massima di un termine, in caratteri Unicode.
    pub const MAX_CHARS: usize = 256;

    /// Valida un termine scritto dall'utente.
    ///
    /// Controlli, nell'ordine: lunghezza, a capo, vuoto (solo spazi compresi), solo jolly.
    /// Il testo resta com'è: niente `trim`, niente normalizzazione (la compilazione porta
    /// il termine in NFC).
    pub fn parse(raw: &str, wildcards: WildcardMode) -> Result<Term, TermError> {
        // Un carattere UTF-8 occupa al massimo 4 byte: oltre, è troppo lungo senza contare.
        let too_long = TermError::TooLong {
            max_chars: Self::MAX_CHARS,
        };
        if raw.len() > Self::MAX_CHARS * 4 || raw.chars().count() > Self::MAX_CHARS {
            return Err(too_long);
        }
        if raw.contains(['\n', '\r']) {
            return Err(TermError::LineBreak);
        }
        if raw.chars().all(char::is_whitespace) {
            return Err(TermError::Empty);
        }
        if wildcards == WildcardMode::Enabled
            && raw
                .chars()
                .all(|c| c == '*' || c == '?' || c.is_whitespace())
        {
            return Err(TermError::OnlyWildcards);
        }
        Ok(Term {
            raw: raw.to_owned(),
            wildcards,
        })
    }

    /// Il testo del termine, identico a quello passato a [`Term::parse`].
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// La modalità dei jolly con cui il termine è stato validato.
    pub fn wildcards(&self) -> WildcardMode {
        self.wildcards
    }
}

/// Motivo per cui una query non si può costruire o compilare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum QueryError {
    /// Nessun termine.
    #[error("nessun termine")]
    NoTerms,
    /// Più di `max` termini.
    #[error("più di {max} termini")]
    TooManyTerms {
        /// Il limite superato, [`Query::MAX_TERMS`].
        max: usize,
    },
    /// Il termine `term_index` è stato validato con jolly diversi da quelli delle opzioni.
    #[error("il termine {term_index} è stato validato con altri jolly")]
    WildcardModeMismatch {
        /// Indice del termine (0-based).
        term_index: usize,
    },
    /// Il termine `term_index` produce un automa oltre i limiti di dimensione.
    #[error("il termine {term_index} è troppo complesso")]
    TooComplex {
        /// Indice del termine (0-based).
        term_index: usize,
    },
}

/// Una ricerca: termini, opzioni e combinazione.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    terms: Vec<Term>,
    options: MatchOptions,
    combine: Combine,
}

impl Query {
    /// Numero massimo di termini: i termini corrispondenti si rappresentano in un
    /// [`TermSet`](crate::TermSet) a 32 bit.
    pub const MAX_TERMS: usize = 32;

    /// Riunisce i termini; l'ordine conta, perché l'indice di un termine è quello usato
    /// nei [`TermSet`](crate::TermSet).
    ///
    /// Errori: nessun termine, più di [`Query::MAX_TERMS`], termine validato con una
    /// [`WildcardMode`] diversa da `options.wildcards`.
    pub fn new(
        terms: Vec<Term>,
        options: MatchOptions,
        combine: Combine,
    ) -> Result<Query, QueryError> {
        if terms.is_empty() {
            return Err(QueryError::NoTerms);
        }
        if terms.len() > Self::MAX_TERMS {
            return Err(QueryError::TooManyTerms {
                max: Self::MAX_TERMS,
            });
        }
        if let Some(term_index) = terms.iter().position(|t| t.wildcards != options.wildcards) {
            return Err(QueryError::WildcardModeMismatch { term_index });
        }
        Ok(Query {
            terms,
            options,
            combine,
        })
    }

    /// I termini, nell'ordine dato.
    pub fn terms(&self) -> &[Term] {
        &self.terms
    }

    /// Le opzioni di confronto.
    pub fn options(&self) -> MatchOptions {
        self.options
    }

    /// OR o AND.
    pub fn combine(&self) -> Combine {
        self.combine
    }

    /// Compila ogni termine in un automa pronto per [`find_words`](crate::find_words).
    ///
    /// Regole (specifica, sezione 4): letterali escapati; `*` → zero o più non-spazi,
    /// `?` → un non-spazio; ogni sequenza di spazi del termine → uno o più spazi
    /// orizzontali (mai un a capo); maiuscole ignorate con `(?i)`.
    ///
    /// Implementazione provvisoria (fase 0): `AccentMode::Ignored` si comporta come
    /// `Distinct`, il termine non è portato in NFC e la forma NFD non è riconosciuta.
    ///
    /// Errore: [`QueryError::TooComplex`] se un automa supera i limiti di dimensione.
    pub fn compile(&self) -> Result<CompiledQuery, QueryError> {
        let mut regexes = Vec::with_capacity(self.terms.len());
        for (term_index, term) in self.terms.iter().enumerate() {
            let pattern = term_pattern(term.as_str(), self.options.wildcards);
            let regex = RegexBuilder::new(&pattern)
                .case_insensitive(self.options.case == CaseMode::Insensitive)
                .size_limit(REGEX_SIZE_LIMIT_BYTES)
                .build()
                .map_err(|_| QueryError::TooComplex { term_index })?;
            regexes.push(regex);
        }
        Ok(CompiledQuery {
            regexes,
            options: self.options,
            combine: self.combine,
        })
    }
}

/// Limite di memoria dell'automa di un termine: abbondante per 256 caratteri anche con
/// le classi degli accenti, ma finito.
const REGEX_SIZE_LIMIT_BYTES: usize = 16 * 1024 * 1024;

/// Spazi orizzontali: uno spazio Unicode che non è un a capo.
const HORIZONTAL_SPACES: &str = r"[^\S\r\n]+";

/// Traduce un termine nella sintassi del crate `regex`.
fn term_pattern(raw: &str, wildcards: WildcardMode) -> String {
    let mut pattern = String::with_capacity(raw.len() * 2);
    let mut in_spaces = false;
    for c in raw.chars() {
        if c.is_whitespace() {
            if !in_spaces {
                pattern.push_str(HORIZONTAL_SPACES);
            }
            in_spaces = true;
            continue;
        }
        in_spaces = false;
        match (wildcards, c) {
            (WildcardMode::Enabled, '*') => pattern.push_str(r"\S*"),
            (WildcardMode::Enabled, '?') => pattern.push_str(r"\S"),
            _ => {
                let mut buf = [0u8; 4];
                pattern.push_str(&regex::escape(c.encode_utf8(&mut buf)));
            }
        }
    }
    pattern
}

/// Query pronta per la ricerca: un automa per termine, nell'ordine della [`Query`].
///
/// È `Send + Sync`: si condivide tra i thread di ricerca con un `Arc`.
#[derive(Debug, Clone)]
pub struct CompiledQuery {
    regexes: Vec<Regex>,
    options: MatchOptions,
    combine: Combine,
}

impl CompiledQuery {
    /// Numero di termini (da 1 a [`Query::MAX_TERMS`]).
    pub fn term_count(&self) -> usize {
        self.regexes.len()
    }

    /// Le opzioni con cui è stata compilata.
    pub fn options(&self) -> MatchOptions {
        self.options
    }

    /// OR o AND, per [`FileAccumulator`](crate::FileAccumulator).
    pub fn combine(&self) -> Combine {
        self.combine
    }

    /// Gli automi, uno per termine (uso interno del crate).
    pub(crate) fn regexes(&self) -> &[Regex] {
        &self.regexes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term(raw: &str) -> Term {
        Term::parse(raw, WildcardMode::Enabled).expect("termine valido nel test")
    }

    #[test]
    fn term_parse_partitions() {
        let on = WildcardMode::Enabled;
        assert_eq!(Term::parse("", on), Err(TermError::Empty));
        assert_eq!(Term::parse("  \t", on), Err(TermError::Empty));
        assert_eq!(Term::parse("*?*", on), Err(TermError::OnlyWildcards));
        assert_eq!(Term::parse("* ?", on), Err(TermError::OnlyWildcards));
        assert_eq!(Term::parse("a\nb", on), Err(TermError::LineBreak));
        assert!(Term::parse("*", WildcardMode::Literal).is_ok());
        assert_eq!(term(" la car ").as_str(), " la car ");
    }

    #[test]
    fn term_length_on_and_off_points() {
        let on = WildcardMode::Enabled;
        let at_limit = "à".repeat(Term::MAX_CHARS);
        let over = "à".repeat(Term::MAX_CHARS + 1);
        assert!(Term::parse(&at_limit, on).is_ok());
        assert_eq!(
            Term::parse(&over, on),
            Err(TermError::TooLong {
                max_chars: Term::MAX_CHARS
            })
        );
    }

    #[test]
    fn query_new_partitions() {
        let opts = MatchOptions::default();
        assert_eq!(
            Query::new(vec![], opts, Combine::Any),
            Err(QueryError::NoTerms)
        );
        let many = vec![term("a"); Query::MAX_TERMS + 1];
        assert_eq!(
            Query::new(many, opts, Combine::Any),
            Err(QueryError::TooManyTerms {
                max: Query::MAX_TERMS
            })
        );
        assert!(Query::new(vec![term("a"); Query::MAX_TERMS], opts, Combine::All).is_ok());
        let literal = Term::parse("x", WildcardMode::Literal).expect("valido");
        assert_eq!(
            Query::new(vec![term("a"), literal], opts, Combine::Any),
            Err(QueryError::WildcardModeMismatch { term_index: 1 })
        );
    }

    #[test]
    fn compiled_query_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<CompiledQuery>();
    }
}
