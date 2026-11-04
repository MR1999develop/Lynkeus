// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Ricerca in un testo: dalle corrispondenze dei termini alle parole che le contengono.
//!
//! Tutti gli intervalli sono in byte e cadono sempre su confini di carattere UTF-8.

use std::collections::BTreeMap;
use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::query::{CompiledQuery, Query};

/// Insieme di indici di termine (0-based), al massimo [`Query::MAX_TERMS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TermSet(u32);

impl TermSet {
    /// Nessun termine.
    pub const EMPTY: TermSet = TermSet(0);

    /// Solo il termine `index`.
    ///
    /// # Panics
    /// Se `index >= Query::MAX_TERMS`: gli indici vengono da una [`Query`] valida.
    pub fn single(index: usize) -> TermSet {
        let mut set = TermSet::EMPTY;
        set.insert(index);
        set
    }

    /// Tutti i termini da 0 a `count - 1` (`count` limitato a [`Query::MAX_TERMS`]).
    pub fn all(count: usize) -> TermSet {
        match count.min(Query::MAX_TERMS) {
            0 => TermSet::EMPTY,
            Query::MAX_TERMS => TermSet(u32::MAX),
            n => TermSet((1u32 << n) - 1),
        }
    }

    /// Aggiunge il termine `index`.
    ///
    /// # Panics
    /// Se `index >= Query::MAX_TERMS`.
    pub fn insert(&mut self, index: usize) {
        assert!(index < Query::MAX_TERMS, "indice di termine fuori limite");
        self.0 |= 1 << index;
    }

    /// Vero se contiene il termine `index`.
    pub fn contains(self, index: usize) -> bool {
        index < Query::MAX_TERMS && self.0 & (1 << index) != 0
    }

    /// Vero se contiene tutti i termini di `other`.
    pub fn contains_all(self, other: TermSet) -> bool {
        self.0 & other.0 == other.0
    }

    /// Unione dei due insiemi.
    #[must_use]
    pub fn union(self, other: TermSet) -> TermSet {
        TermSet(self.0 | other.0)
    }

    /// Vero se vuoto.
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Numero di termini contenuti.
    pub fn len(self) -> usize {
        self.0.count_ones() as usize
    }

    /// Gli indici contenuti, in ordine crescente.
    pub fn iter(self) -> impl Iterator<Item = usize> {
        (0..Query::MAX_TERMS).filter(move |&i| self.contains(i))
    }
}

/// Una parola del testo che contiene almeno una corrispondenza.
///
/// Più corrispondenze nella stessa parola, anche di termini diversi, danno un solo
/// `WordMatch` con tutti gli evidenziati.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordMatch {
    /// Indice della riga nel testo, 0-based (le righe finiscono con `\n`).
    pub line: u32,
    /// Byte della riga nel testo, senza `\r\n` o `\n` finale.
    pub line_range: Range<usize>,
    /// Byte della parola nel testo: testo tra due spazi, senza punteggiatura ai bordi
    /// (mai tolta dentro una corrispondenza). Sempre dentro `line_range`.
    pub word: Range<usize>,
    /// Byte delle corrispondenze nel testo, ordinati, disgiunti e dentro `word`.
    pub highlights: Vec<Range<usize>>,
    /// Termini che corrispondono in questa parola.
    pub terms: TermSet,
}

/// Parola in costruzione: parola rifilata, evidenziati, termini.
type PendingWord = (Range<usize>, Vec<Range<usize>>, TermSet);

/// Trova le parole di `text` che contengono almeno un termine.
///
/// `text` può contenere più righe; nessuna corrispondenza attraversa una riga.
/// Risultato ordinato per posizione della parola, una voce per parola.
pub fn find_words(query: &CompiledQuery, text: &str) -> Vec<WordMatch> {
    // Chiave: inizio della parola delimitata dagli spazi; valore: parola rifilata,
    // evidenziati e termini.
    let mut words: BTreeMap<usize, PendingWord> = BTreeMap::new();
    for (term_index, regex) in query.regexes().iter().enumerate() {
        for found in regex.find_iter(text) {
            if found.is_empty() {
                continue;
            }
            let hit = found.range();
            let spaced = spaced_word(text, &hit);
            let trimmed = trim_punctuation(text, &spaced, &hit);
            let entry = words
                .entry(spaced.start)
                .or_insert_with(|| (trimmed.clone(), Vec::new(), TermSet::EMPTY));
            entry.0 = entry.0.start.min(trimmed.start)..entry.0.end.max(trimmed.end);
            entry.1.push(hit);
            entry.2.insert(term_index);
        }
    }
    let lines = LineIndex::new(text);
    words
        .into_values()
        .map(|(word, highlights, terms)| {
            let (line, line_range) = lines.line_of(word.start);
            WordMatch {
                line,
                line_range,
                word,
                highlights: merge_ranges(highlights),
                terms,
            }
        })
        .collect()
}

/// Estende la corrispondenza fino agli spazi (White_Space Unicode) a sinistra e a destra.
fn spaced_word(text: &str, hit: &Range<usize>) -> Range<usize> {
    let start = text[..hit.start]
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map_or(0, |(i, c)| i + c.len_utf8());
    let end = text[hit.end..]
        .char_indices()
        .find(|(_, c)| c.is_whitespace())
        .map_or(text.len(), |(i, _)| hit.end + i);
    start..end
}

/// Toglie la punteggiatura ai bordi di `word`, senza entrare nella corrispondenza `hit`.
///
/// Provvisorio (fase 0): "punteggiatura" = `char::is_ascii_punctuation` più virgolette e
/// trattini tipografici; la fase 1 userà le categorie Unicode.
fn trim_punctuation(text: &str, word: &Range<usize>, hit: &Range<usize>) -> Range<usize> {
    let mut start = word.start;
    for (i, c) in text[word.start..hit.start].char_indices() {
        if !is_edge_punctuation(c) {
            break;
        }
        start = word.start + i + c.len_utf8();
    }
    let mut end = word.end;
    for (i, c) in text[hit.end..word.end].char_indices().rev() {
        if !is_edge_punctuation(c) {
            break;
        }
        end = hit.end + i;
    }
    start..end
}

fn is_edge_punctuation(c: char) -> bool {
    c.is_ascii_punctuation() || matches!(c, '«' | '»' | '“' | '”' | '‘' | '’' | '…' | '–' | '—')
}

/// Unisce intervalli sovrapposti o adiacenti e li ordina.
fn merge_ranges(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    ranges.sort_by_key(|r| r.start);
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged
}

/// Inizi delle righe di un testo, per passare da un byte al numero di riga.
struct LineIndex<'a> {
    text: &'a str,
    starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    fn new(text: &'a str) -> Self {
        let starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        LineIndex { text, starts }
    }

    /// Riga (0-based, saturata a `u32::MAX`) e byte della riga senza terminatore.
    fn line_of(&self, byte: usize) -> (u32, Range<usize>) {
        let index = self
            .starts
            .partition_point(|&s| s <= byte)
            .saturating_sub(1);
        let start = self.starts.get(index).copied().unwrap_or(0);
        let mut end = self
            .starts
            .get(index + 1)
            .map_or(self.text.len(), |&next| next - 1);
        if self.text[start..end].ends_with('\r') {
            end -= 1;
        }
        (u32::try_from(index).unwrap_or(u32::MAX), start..end)
    }
}

/// Testo della parola da mostrare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordText {
    /// La parola, o il suo inizio se troncata (senza "…": la aggiunge chi la mostra).
    pub text: String,
    /// Vero se la parola superava `max_chars` caratteri.
    pub truncated: bool,
}

/// Parola di `found` in `text`, troncata a `max_chars` caratteri.
///
/// Provvisorio (fase 0): tiene l'inizio della parola; la fase 1 centrerà il taglio sulla
/// prima corrispondenza.
pub fn word_text(text: &str, found: &WordMatch, max_chars: usize) -> WordText {
    let word = text.get(found.word.clone()).unwrap_or_default();
    match word.char_indices().nth(max_chars) {
        Some((cut, _)) => WordText {
            text: word[..cut].to_owned(),
            truncated: true,
        },
        None => WordText {
            text: word.to_owned(),
            truncated: false,
        },
    }
}

/// Contesto di una parola: un pezzo della sua riga con gli evidenziati.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snippet {
    /// Il pezzo di riga (senza "…": lo aggiunge chi lo mostra, secondo i flag).
    pub text: String,
    /// Byte delle corrispondenze relativi a `text`, ordinati e disgiunti.
    pub highlights: Vec<Range<usize>>,
    /// Vero se la riga continua prima di `text`.
    pub truncated_start: bool,
    /// Vero se la riga continua dopo `text`.
    pub truncated_end: bool,
}

/// Contesto di `found`: circa `max_chars` caratteri della sua riga intorno alla parola,
/// divisi tra sinistra e destra; se la parola è più lunga, il suo inizio.
pub fn snippet(text: &str, found: &WordMatch, max_chars: usize) -> Snippet {
    let line_range = found.line_range.clone();
    let line = text.get(line_range.clone()).unwrap_or_default();
    let word_start = found
        .word
        .start
        .saturating_sub(line_range.start)
        .min(line.len());
    let word_end = found
        .word
        .end
        .saturating_sub(line_range.start)
        .clamp(word_start, line.len());
    let word_chars = line[word_start..word_end].chars().count();

    let (start, end) = if word_chars >= max_chars {
        let end = line[word_start..]
            .char_indices()
            .nth(max_chars)
            .map_or(line.len(), |(i, _)| word_start + i);
        (word_start, end)
    } else {
        let budget = max_chars - word_chars;
        let left_avail = line[..word_start].chars().count();
        let right_avail = line[word_end..].chars().count();
        let right_take = right_avail.min(budget - left_avail.min(budget / 2));
        let left_take = left_avail.min(budget - right_take);
        let start = if left_take == 0 {
            word_start
        } else {
            line[..word_start]
                .char_indices()
                .rev()
                .nth(left_take - 1)
                .map_or(0, |(i, _)| i)
        };
        let end = line[word_end..]
            .char_indices()
            .nth(right_take)
            .map_or(line.len(), |(i, _)| word_end + i);
        (start, end)
    };

    let absolute = line_range.start + start..line_range.start + end;
    let highlights = found
        .highlights
        .iter()
        .filter_map(|h| {
            let s = h.start.max(absolute.start);
            let e = h.end.min(absolute.end);
            (s < e).then(|| s - absolute.start..e - absolute.start)
        })
        .collect();
    Snippet {
        text: line[start..end].to_owned(),
        highlights,
        truncated_start: start > 0,
        truncated_end: end < line.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::{Combine, MatchOptions, Term};

    fn compile(terms: &[&str], options: MatchOptions) -> CompiledQuery {
        let terms = terms
            .iter()
            .map(|t| Term::parse(t, options.wildcards).expect("termine valido"))
            .collect();
        Query::new(terms, options, Combine::Any)
            .and_then(|q| q.compile())
            .expect("query valida")
    }

    fn words(terms: &[&str], text: &str) -> Vec<String> {
        let query = compile(terms, MatchOptions::default());
        find_words(&query, text)
            .iter()
            .map(|m| text[m.word.clone()].to_owned())
            .collect()
    }

    #[test]
    fn finds_words_containing_the_term() {
        assert_eq!(
            words(&["ar"], "Cuocere l'aragosta, poi la carta e la chitarra."),
            ["l'aragosta", "carta", "chitarra"]
        );
    }

    #[test]
    fn case_insensitive_by_default_and_sensitive_on_request() {
        assert_eq!(words(&["ar"], "ARMADIO grande"), ["ARMADIO"]);
        let sensitive = MatchOptions {
            case: crate::CaseMode::Sensitive,
            ..MatchOptions::default()
        };
        assert!(find_words(&compile(&["ar"], sensitive), "ARMADIO").is_empty());
    }

    #[test]
    fn punctuation_trimmed_at_edges_but_never_inside_the_match() {
        assert_eq!(words(&["arte"], "città d'arte."), ["d'arte"]);
        assert_eq!(words(&["ar"], "(carta)"), ["carta"]);
        assert_eq!(words(&["(car"], "(carta)"), ["(carta"]);
    }

    #[test]
    fn wildcards_and_spaces() {
        assert_eq!(
            words(&["c?rt*"], "la carta e il cortile"),
            ["carta", "cortile"]
        );
        assert_eq!(words(&["la car"], "prendi la  carta"), ["la  carta"]);
        assert!(words(&["la car"], "la\ncarta").is_empty());
    }

    #[test]
    fn several_matches_in_one_word_give_one_row() {
        let query = compile(&["ar", "ta"], MatchOptions::default());
        let found = find_words(&query, "una carta");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].terms, TermSet::all(2));
        assert_eq!(found[0].highlights, vec![5..9]);
    }

    #[test]
    fn line_numbers_and_ranges() {
        let text = "uno\r\ndue\ncarta tre";
        let query = compile(&["ar"], MatchOptions::default());
        let found = find_words(&query, text);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].line, 2);
        assert_eq!(&text[found[0].line_range.clone()], "carta tre");
        let first = find_words(&compile(&["uno"], MatchOptions::default()), text);
        assert_eq!(&text[first[0].line_range.clone()], "uno");
    }

    #[test]
    fn word_text_truncation_on_and_off_points() {
        let query = compile(&["a"], MatchOptions::default());
        let text = "abcde";
        let found = &find_words(&query, text)[0];
        assert!(!word_text(text, found, 5).truncated);
        let cut = word_text(text, found, 4);
        assert_eq!((cut.text.as_str(), cut.truncated), ("abcd", true));
    }

    #[test]
    fn snippet_centres_the_word_and_shifts_highlights() {
        let query = compile(&["ar"], MatchOptions::default());
        let text = "0123456789 carta 0123456789";
        let found = &find_words(&query, text)[0];
        let s = snippet(text, found, 9);
        assert_eq!(s.text, "9 carta 0");
        assert_eq!(&s.text[s.highlights[0].clone()], "ar");
        assert!(s.truncated_start && s.truncated_end);
        let whole = snippet(text, found, 200);
        assert_eq!(whole.text, text);
        assert!(!whole.truncated_start && !whole.truncated_end);
    }

    #[test]
    fn term_set_operations() {
        let mut set = TermSet::single(3);
        set.insert(0);
        assert_eq!(set.iter().collect::<Vec<_>>(), [0, 3]);
        assert_eq!(set.len(), 2);
        assert!(TermSet::all(32).contains(31));
        assert!(!set.contains(40));
        assert!(TermSet::all(4).contains_all(set));
    }
}
