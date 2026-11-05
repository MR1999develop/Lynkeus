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

use crate::pattern::{GUARD_GROUP, LEAD_GROUP, TRAIL_GROUP, TermPattern, term_pattern};
use crate::unicode::without_marks;

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
    /// Vuoto, o fatto solo di spazi e di segni combinanti (accenti senza lettera).
    #[error("termine vuoto")]
    Empty,
    /// Fatto solo di `*`, `?`, spazi e segni combinanti, con i jolly attivi: troverebbe
    /// ogni parola.
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

/// Un termine di ricerca valido, come l'ha scritto l'utente ma senza gli spazi iniziali e
/// finali (quelli interni restano e contano).
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
    /// Prima toglie gli spazi (White_Space Unicode) iniziali e finali: con `" car"` la
    /// corrispondenza inizierebbe con uno spazio e la parola si estenderebbe oltre lo
    /// spazio. Poi controlla, nell'ordine: lunghezza (senza quegli spazi), a capo, vuoto,
    /// solo jolly. Vuoto e solo jolly si giudicano senza i segni combinanti, come li
    /// vedrebbe la ricerca con gli accenti ignorati: "\u{301}" è vuoto e "*\u{301}" è solo
    /// jolly con qualunque opzione degli accenti, perché un termine valido lo resta anche
    /// cambiando opzione. Il termine non si normalizza: lo fa la compilazione.
    pub fn parse(raw: &str, wildcards: WildcardMode) -> Result<Term, TermError> {
        let raw = raw.trim();
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
        let mut bases = without_marks(raw).filter(|c| !c.is_whitespace()).peekable();
        if bases.peek().is_none() {
            return Err(TermError::Empty);
        }
        if wildcards == WildcardMode::Enabled && bases.all(|c| c == '*' || c == '?') {
            return Err(TermError::OnlyWildcards);
        }
        Ok(Term {
            raw: raw.to_owned(),
            wildcards,
        })
    }

    /// Il testo del termine: quello passato a [`Term::parse`] senza spazi iniziali e finali.
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
    /// Regole (specifica, sezione 4):
    ///
    /// - il termine è portato in NFC e i letterali sono escapati;
    /// - jolly attivi: `*` → zero o più caratteri non spazio (greedy), `?` → un carattere
    ///   non spazio con i suoi eventuali segni combinanti; jolly spenti: `*` e `?` letterali;
    /// - ogni sequenza di spazi del termine → uno o più spazi orizzontali (mai un a capo);
    /// - maiuscole ignorate con `(?i)`;
    /// - accenti distinti: una lettera accentata del termine trova la forma precomposta e
    ///   quella decomposta (NFD) del testo; il testo non si normalizza, quindi le posizioni
    ///   restano quelle originali;
    /// - accenti ignorati: ogni lettera trova le sue varianti accentate (tabella costruita
    ///   una volta dalla decomposizione canonica Unicode) e gli eventuali segni combinanti
    ///   `Mn` che la seguono. Le lettere senza decomposizione canonica (`ø`, `ł`, `æ`)
    ///   restano distinte.
    ///
    /// Errore: [`QueryError::TooComplex`] se un automa supera i limiti di dimensione.
    pub fn compile(&self) -> Result<CompiledQuery, QueryError> {
        let mut matchers = Vec::with_capacity(self.terms.len());
        for (term_index, term) in self.terms.iter().enumerate() {
            let TermPattern {
                pattern,
                lead_star,
                trail_star,
            } = term_pattern(term.as_str(), self.options);
            let regex = RegexBuilder::new(&pattern)
                .case_insensitive(self.options.case == CaseMode::Insensitive)
                .size_limit(REGEX_SIZE_LIMIT_BYTES)
                .build()
                .map_err(|_| QueryError::TooComplex { term_index })?;
            let group = |present: bool, name: &str| {
                present
                    .then(|| regex.capture_names().position(|n| n == Some(name)))
                    .flatten()
            };
            let lead_group = group(lead_star, LEAD_GROUP);
            let trail_group = group(trail_star, TRAIL_GROUP);
            let guard_group = group(true, GUARD_GROUP);
            matchers.push(TermMatcher {
                regex,
                lead_group,
                trail_group,
                guard_group,
            });
        }
        Ok(CompiledQuery {
            matchers,
            options: self.options,
            combine: self.combine,
        })
    }
}

/// Limite di memoria dell'automa di un termine: abbondante per 256 caratteri anche con
/// le classi degli accenti, ma finito.
const REGEX_SIZE_LIMIT_BYTES: usize = 16 * 1024 * 1024;

/// L'automa di un termine, con gli indici dei gruppi da trattare dopo la corrispondenza
/// (vedi `pattern.rs`).
#[derive(Debug, Clone)]
pub(crate) struct TermMatcher {
    /// L'automa.
    pub(crate) regex: Regex,
    /// Gruppo del `*` iniziale, se il termine inizia con `*`.
    pub(crate) lead_group: Option<usize>,
    /// Gruppo del `*` finale, se il termine finisce con `*`.
    pub(crate) trail_group: Option<usize>,
    /// Gruppo della guardia finale (c'è sempre: `None` solo se l'automa non lo avesse).
    pub(crate) guard_group: Option<usize>,
}

/// Query pronta per la ricerca: un automa per termine, nell'ordine della [`Query`].
///
/// È `Send + Sync`: si condivide tra i thread di ricerca con un `Arc`.
#[derive(Debug, Clone)]
pub struct CompiledQuery {
    matchers: Vec<TermMatcher>,
    options: MatchOptions,
    combine: Combine,
}

impl CompiledQuery {
    /// Numero di termini (da 1 a [`Query::MAX_TERMS`]).
    pub fn term_count(&self) -> usize {
        self.matchers.len()
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
    pub(crate) fn matchers(&self) -> &[TermMatcher] {
        &self.matchers
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
        assert_eq!(term(" la car ").as_str(), "la car");
        assert_eq!(term("\u{3000}car\t").as_str(), "car");
        assert_eq!(Term::parse(" * ", on), Err(TermError::OnlyWildcards));
        assert_eq!(
            term("arte\r\n").as_str(),
            "arte",
            "a capo finale tolto come spazio"
        );
    }

    /// Un termine che, tolti i segni combinanti, resta vuoto o fatto di soli jolly non è
    /// valido: con gli accenti ignorati "*\u{301}" diventerebbe "*" e troverebbe ogni parola.
    #[test]
    fn terms_normalizing_to_wildcards_are_rejected() {
        let on = WildcardMode::Enabled;
        assert_eq!(Term::parse("*\u{301}", on), Err(TermError::OnlyWildcards));
        assert_eq!(Term::parse("?\u{301}", on), Err(TermError::OnlyWildcards));
        assert_eq!(Term::parse("* \u{301}?", on), Err(TermError::OnlyWildcards));
        assert_eq!(Term::parse("\u{301}", on), Err(TermError::Empty));
        assert_eq!(
            Term::parse(" \u{301}\u{302} ", WildcardMode::Literal),
            Err(TermError::Empty)
        );
        // Con i jolly letterali `*` è un carattere come gli altri.
        assert!(Term::parse("*\u{301}", WildcardMode::Literal).is_ok());
        // Una lettera con i suoi segni resta valida, anche accanto ai jolly.
        assert!(Term::parse("e\u{301}*", on).is_ok());
        assert!(Term::parse("\u{301}a", on).is_ok());
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

    /// I termini più lunghi e più costosi restano sotto il limite di dimensione degli automi.
    #[test]
    fn longest_terms_compile_in_every_mode() {
        let accented = "àèìòù".repeat(Term::MAX_CHARS / 5);
        let wild = "*a?".repeat(Term::MAX_CHARS / 3);
        for accents in [AccentMode::Distinct, AccentMode::Ignored] {
            for case in [CaseMode::Insensitive, CaseMode::Sensitive] {
                let options = MatchOptions {
                    case,
                    accents,
                    wildcards: WildcardMode::Enabled,
                };
                let terms = vec![term(&accented), term(&wild)];
                let compiled = Query::new(terms, options, Combine::Any).and_then(|q| q.compile());
                assert!(compiled.is_ok(), "{accents:?} {case:?}");
            }
        }
    }

    #[test]
    fn compiled_query_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<CompiledQuery>();
    }
}
