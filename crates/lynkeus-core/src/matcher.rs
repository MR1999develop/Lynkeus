// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Ricerca in un testo: dalle corrispondenze dei termini alle parole che le contengono.
//!
//! Tutti gli intervalli sono in byte e cadono sempre su confini di carattere UTF-8.
//!
//! # Dalla corrispondenza alla parola
//!
//! 1. **Corrispondenza.** Ogni termine trova le sue corrispondenze (non sovrapposte, da
//!    sinistra). Una corrispondenza non è mai seguita da ciò che continuerebbe il suo ultimo
//!    carattere: un segno combinante `Mn` con gli accenti distinti (nel testo NFD `citta` +
//!    `◌̀` è "città", che "citta" non deve trovare), un jamo hangul con qualunque opzione
//!    ("구" non trova il "국" decomposto). La regola sta nel pattern, vedi `pattern.rs`.
//! 2. **Jolly di bordo.** Se il termine inizia (finisce) con `*`, dalla parte catturata da
//!    quel `*` si tolgono i segni di punteggiatura iniziali (finali): "c?rt*" su "carta,"
//!    evidenzia "carta", e "*arta" su "«carta" evidenzia "carta". La punteggiatura che il
//!    `*` cattura dentro la parola resta ("c*" su "c.a.r." evidenzia "c.a.r"); quella
//!    scritta nel termine non si tocca mai. Gli spazi ai bordi, possibili solo con un `*`
//!    vuoto accanto a uno spazio del termine ("* car" su " carta"), si tolgono allo stesso
//!    modo.
//! 3. **Parola.** La corrispondenza si estende a sinistra e a destra fino agli spazi
//!    (White_Space Unicode) o ai bordi del testo. Le parole così ottenute che si
//!    sovrappongono (più corrispondenze nella stessa parola, anche di termini diversi, o un
//!    termine con spazi che copre più parole) diventano un solo [`WordMatch`].
//! 4. **Punteggiatura.** Dai bordi della parola si tolgono i segni di punteggiatura
//!    (categoria Unicode `P`), senza mai entrare in una corrispondenza. I simboli
//!    (categoria `S`: `$`, `+`, `<`, …) restano.

use std::ops::Range;

use regex::CaptureLocations;

use serde::{Deserialize, Serialize};

use crate::query::{CompiledQuery, Query, TermMatcher};
use crate::unicode::is_punctuation;

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
    /// Indice della riga nel testo, 0-based (le righe finiscono con `\n`), saturato a
    /// `u32::MAX`.
    pub line: u32,
    /// Byte della riga nel testo, senza `\r\n` o `\n` finale.
    pub line_range: Range<usize>,
    /// Byte della parola nel testo: testo tra due spazi, senza punteggiatura ai bordi
    /// (mai tolta dentro una corrispondenza). Sempre dentro `line_range`, mai vuota.
    pub word: Range<usize>,
    /// Byte delle corrispondenze nel testo: non vuoti, ordinati, disgiunti e non adiacenti
    /// (due corrispondenze che si toccano diventano una), dentro `word`.
    pub highlights: Vec<Range<usize>>,
    /// Termini che corrispondono in questa parola (almeno uno).
    pub terms: TermSet,
}

/// Una corrispondenza con la parola delimitata dagli spazi che la contiene.
struct Hit {
    spaced: Range<usize>,
    hit: Range<usize>,
    term: usize,
}

/// Trova le parole di `text` che contengono almeno un termine.
///
/// `text` può contenere più righe; nessuna corrispondenza attraversa una riga, quindi
/// cercare riga per riga (o a blocchi di righe) dà gli stessi risultati, a meno dello
/// spostamento di righe e byte. Risultato ordinato per posizione, una voce per parola;
/// regole complete nella documentazione del modulo.
///
/// Tempo lineare nella lunghezza del testo per ogni termine (automi del crate `regex`),
/// anche con parole lunghissime senza spazi.
pub fn find_words(query: &CompiledQuery, text: &str) -> Vec<WordMatch> {
    let mut hits: Vec<Hit> = Vec::new();
    for (term, matcher) in query.matchers().iter().enumerate() {
        // Le corrispondenze di un termine arrivano in ordine: la parola dell'ultima si
        // riusa se contiene la successiva, così una parola lunghissima con molte
        // corrispondenze si percorre una volta sola.
        let mut last_spaced: Option<Range<usize>> = None;
        for_each_hit(matcher, text, |hit| {
            let spaced = match &last_spaced {
                Some(s) if s.start <= hit.start && hit.end <= s.end => s.clone(),
                _ => spaced_word(text, &hit),
            };
            last_spaced = Some(spaced.clone());
            hits.push(Hit { spaced, hit, term });
        });
    }
    hits.sort_by_key(|h| (h.spaced.start, h.hit.start));

    let mut lines = LineCursor::new(text);
    let mut words = Vec::new();
    let mut pending = hits.into_iter().peekable();
    while let Some(first) = pending.next() {
        let mut spaced = first.spaced;
        let mut highlights = vec![first.hit];
        let mut terms = TermSet::single(first.term);
        while let Some(next) = pending.next_if(|n| n.spaced.start < spaced.end) {
            spaced.end = spaced.end.max(next.spaced.end);
            highlights.push(next.hit);
            terms.insert(next.term);
        }
        let highlights = merge_ranges(highlights);
        let word = trim_punctuation(text, &spaced, &highlights);
        let (line, line_range) = lines.locate(word.start);
        words.push(WordMatch {
            line,
            line_range,
            word,
            highlights,
            terms,
        });
    }
    words
}

/// Parti catturate dai `*` ai bordi (byte nel testo).
#[derive(Default)]
struct StarGroups {
    lead: Option<Range<usize>>,
    trail: Option<Range<usize>>,
}

/// Chiama `emit` per ogni corrispondenza di un termine, già rifilata (passi 1 e 2 della
/// documentazione del modulo).
fn for_each_hit(matcher: &TermMatcher, text: &str, mut emit: impl FnMut(Range<usize>)) {
    let mut locations = matcher.regex.capture_locations();
    let mut position = 0;
    while position <= text.len() {
        let Some((found, stars)) = next_match(matcher, text, position, &mut locations) else {
            break;
        };
        if found.is_empty() {
            position = next_char_boundary(text, found.start);
            continue;
        }
        position = found.end;
        if let Some(hit) = trim_hit(text, found, &stars) {
            emit(hit);
        }
    }
}

/// La prima corrispondenza da `position` in poi, senza la guardia finale (che non ne fa
/// parte: la ricerca successiva riparte da lì), con le parti catturate dai `*` di bordo.
///
/// I gruppi si leggono solo quando servono: per la sola guardia basta sapere che, se la
/// corrispondenza non arriva alla fine del testo, la guardia ha consumato l'ultimo
/// carattere (`\z` è possibile solo alla fine).
fn next_match(
    matcher: &TermMatcher,
    text: &str,
    position: usize,
    locations: &mut CaptureLocations,
) -> Option<(Range<usize>, StarGroups)> {
    let regex = &matcher.regex;
    let has_stars = matcher.lead_group.is_some() || matcher.trail_group.is_some();
    if !has_stars {
        let whole = regex.find_at(text, position)?.range();
        if matcher.guard_group.is_none() {
            return Some((whole, StarGroups::default()));
        }
        if whole.end < text.len() {
            let guard_len = text[whole.clone()]
                .chars()
                .next_back()
                .map_or(0, char::len_utf8);
            return Some((whole.start..whole.end - guard_len, StarGroups::default()));
        }
    }
    let whole = regex.captures_read_at(locations, text, position)?.range();
    let group = |index: Option<usize>| {
        index
            .and_then(|i| locations.get(i))
            .map(|(start, end)| start..end)
    };
    let end = group(matcher.guard_group).map_or(whole.end, |guard| guard.start);
    let stars = StarGroups {
        lead: group(matcher.lead_group),
        trail: group(matcher.trail_group),
    };
    Some((whole.start..end, stars))
}

/// Il confine di carattere dopo `byte` (oltre la fine se `byte` è la fine).
fn next_char_boundary(text: &str, byte: usize) -> usize {
    text.get(byte..)
        .and_then(|rest| rest.chars().next())
        .map_or(byte + 1, |c| byte + c.len_utf8())
}

/// Toglie dalla corrispondenza `found` la punteggiatura di bordo catturata dai jolly
/// `lead` e `trail` e gli spazi ai bordi; `None` se non resta nulla.
fn trim_hit(text: &str, found: Range<usize>, groups: &StarGroups) -> Option<Range<usize>> {
    let mut start = found.start;
    let mut end = found.end;
    if let Some(lead) = &groups.lead {
        start = lead.start + leading_len(text.get(lead.clone())?, is_punctuation);
    }
    if let Some(trail) = &groups.trail {
        end = trail.end - trailing_len(text.get(trail.clone())?, is_punctuation);
    }
    let inner = text.get(start..end)?;
    let start_trimmed = start + leading_len(inner, char::is_whitespace);
    let end_trimmed = end - trailing_len(inner, char::is_whitespace);
    (start_trimmed < end_trimmed).then_some(start_trimmed..end_trimmed)
}

/// Byte del prefisso di `s` fatto di caratteri che soddisfano `pred`.
fn leading_len(s: &str, pred: impl Fn(char) -> bool) -> usize {
    s.char_indices()
        .find(|&(_, c)| !pred(c))
        .map_or(s.len(), |(i, _)| i)
}

/// Byte del suffisso di `s` fatto di caratteri che soddisfano `pred`.
fn trailing_len(s: &str, pred: impl Fn(char) -> bool) -> usize {
    s.char_indices()
        .rev()
        .find(|&(_, c)| !pred(c))
        .map_or(s.len(), |(i, c)| s.len() - i - c.len_utf8())
}

/// Estende la corrispondenza fino agli spazi (White_Space Unicode) a sinistra e a destra.
fn spaced_word(text: &str, hit: &Range<usize>) -> Range<usize> {
    let start = hit.start - trailing_len(&text[..hit.start], |c| !c.is_whitespace());
    let end = hit.end + leading_len(&text[hit.end..], |c| !c.is_whitespace());
    start..end
}

/// Toglie la punteggiatura ai bordi di `spaced` senza entrare negli evidenziati
/// (ordinati, non vuoti).
fn trim_punctuation(
    text: &str,
    spaced: &Range<usize>,
    highlights: &[Range<usize>],
) -> Range<usize> {
    let first = highlights.first().map_or(spaced.start, |h| h.start);
    let last = highlights.last().map_or(spaced.end, |h| h.end);
    let start = spaced.start + leading_len(&text[spaced.start..first], is_punctuation);
    let end = spaced.end - trailing_len(&text[last..spaced.end], is_punctuation);
    start..end
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

/// Numeri di riga per posizioni crescenti: percorre il testo una volta sola.
struct LineCursor<'a> {
    text: &'a str,
    /// Riga corrente, 0-based.
    line: usize,
    /// Primo byte della riga corrente.
    line_start: usize,
    /// Fin qui i `\n` sono già contati.
    scanned: usize,
    /// Fine della riga corrente (posizione del `\n` o fine del testo), se già cercata.
    line_end: Option<usize>,
}

impl<'a> LineCursor<'a> {
    fn new(text: &'a str) -> Self {
        LineCursor {
            text,
            line: 0,
            line_start: 0,
            scanned: 0,
            line_end: None,
        }
    }

    /// Riga (0-based, saturata a `u32::MAX`) e byte della riga senza terminatore.
    /// `byte` non decresce tra una chiamata e l'altra.
    fn locate(&mut self, byte: usize) -> (u32, Range<usize>) {
        let byte = byte.clamp(self.scanned, self.text.len());
        for (i, _) in self.text[self.scanned..byte].match_indices('\n') {
            self.line += 1;
            self.line_start = self.scanned + i + 1;
            self.line_end = None;
        }
        self.scanned = byte;
        let end = match self.line_end {
            Some(end) => end,
            None => {
                let end = self.text[byte..]
                    .find('\n')
                    .map_or(self.text.len(), |i| byte + i);
                self.line_end = Some(end);
                end
            }
        };
        let content_end = if self.text[self.line_start..end].ends_with('\r') {
            end - 1
        } else {
            end
        };
        (
            u32::try_from(self.line).unwrap_or(u32::MAX),
            self.line_start..content_end,
        )
    }
}

/// Testo della parola da mostrare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordText {
    /// La parola, o un suo pezzo di `max_chars` caratteri se troncata (senza "…": lo
    /// aggiunge chi la mostra, secondo `truncated_start` e `truncated_end`).
    pub text: String,
    /// Vero se la parola superava `max_chars` caratteri (`truncated_start || truncated_end`).
    pub truncated: bool,
    /// Vero se il pezzo non comprende l'inizio della parola.
    pub truncated_start: bool,
    /// Vero se il pezzo non comprende la fine della parola.
    pub truncated_end: bool,
}

/// Parola di `found` in `text`, troncata a `max_chars` caratteri su confini di carattere.
///
/// Se la prima corrispondenza sta nei primi `max_chars` caratteri si tiene l'inizio della
/// parola; altrimenti un pezzo centrato sulla prima corrispondenza, così la parte trovata
/// resta visibile.
pub fn word_text(text: &str, found: &WordMatch, max_chars: usize) -> WordText {
    let word = text.get(found.word.clone()).unwrap_or_default();
    let Some((prefix_end, _)) = word.char_indices().nth(max_chars) else {
        return WordText {
            text: word.to_owned(),
            truncated: false,
            truncated_start: false,
            truncated_end: false,
        };
    };
    let focus = found
        .highlights
        .first()
        .and_then(|h| relative(word, &found.word, h))
        .unwrap_or(0..0);
    let range = if focus.end <= prefix_end {
        0..prefix_end
    } else {
        window(word, &focus, max_chars)
    };
    WordText {
        text: word[range.clone()].to_owned(),
        truncated: true,
        truncated_start: range.start > 0,
        truncated_end: range.end < word.len(),
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
/// divisi tra sinistra e destra (il posto che avanza da un lato va all'altro), tagliati su
/// confini di carattere. Se la parola è più lunga di `max_chars`, il pezzo è centrato sulla
/// prima corrispondenza.
pub fn snippet(text: &str, found: &WordMatch, max_chars: usize) -> Snippet {
    let line = text.get(found.line_range.clone()).unwrap_or_default();
    let word = relative(line, &found.line_range, &found.word).unwrap_or(0..0);
    let word_fits = line[word.clone()].chars().nth(max_chars).is_none();
    let focus = if word_fits {
        word
    } else {
        found
            .highlights
            .first()
            .and_then(|h| relative(line, &found.line_range, h))
            .unwrap_or(word)
    };
    let range = window(line, &focus, max_chars);
    let absolute = found.line_range.start + range.start..found.line_range.start + range.end;
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
        text: line[range.clone()].to_owned(),
        highlights,
        truncated_start: range.start > 0,
        truncated_end: range.end < line.len(),
    }
}

/// `inner` (byte assoluti) relativo a `outer`, la cui porzione di testo è `s`; `None` se
/// non sta dentro o non cade su confini di carattere.
fn relative(s: &str, outer: &Range<usize>, inner: &Range<usize>) -> Option<Range<usize>> {
    let start = inner.start.checked_sub(outer.start)?;
    let end = inner.end.checked_sub(outer.start)?;
    (start <= end && s.get(start..end).is_some()).then_some(start..end)
}

/// Circa `max_chars` caratteri di `s` attorno a `focus` (byte, su confini di carattere).
///
/// Il fuoco conta per i suoi primi `max_chars` caratteri al massimo; il resto del budget si
/// divide a metà tra sinistra e destra e ciò che un lato non usa passa all'altro. Conta solo
/// i caratteri che servono, quindi resta veloce anche su righe lunghissime.
fn window(s: &str, focus: &Range<usize>, max_chars: usize) -> Range<usize> {
    let focus_text = &s[focus.clone()];
    let focus_end = focus_text
        .char_indices()
        .nth(max_chars)
        .map_or(focus.end, |(i, _)| focus.start + i);
    let focus_chars = s[focus.start..focus_end].chars().count();
    let budget = max_chars.saturating_sub(focus_chars);
    let left_avail = s[..focus.start].chars().rev().take(budget).count();
    let right_avail = s[focus_end..].chars().take(budget).count();
    let right_take = right_avail.min(budget - left_avail.min(budget / 2));
    let left_take = left_avail.min(budget - right_take);
    let start = match left_take.checked_sub(1) {
        None => focus.start,
        Some(skip) => s[..focus.start]
            .char_indices()
            .rev()
            .nth(skip)
            .map_or(0, |(i, _)| i),
    };
    let end = s[focus_end..]
        .char_indices()
        .nth(right_take)
        .map_or(s.len(), |(i, _)| focus_end + i);
    start..end
}

#[cfg(test)]
mod tests {
    use unicode_normalization::UnicodeNormalization;

    use super::*;
    use crate::query::{AccentMode, CaseMode, Combine, MatchOptions, Term, WildcardMode};

    fn compile(terms: &[&str], options: MatchOptions) -> CompiledQuery {
        let terms = terms
            .iter()
            .map(|t| Term::parse(t, options.wildcards).expect("termine valido"))
            .collect();
        Query::new(terms, options, Combine::Any)
            .and_then(|q| q.compile())
            .expect("query valida")
    }

    fn words_with(terms: &[&str], options: MatchOptions, text: &str) -> Vec<String> {
        find_words(&compile(terms, options), text)
            .iter()
            .map(|m| text[m.word.clone()].to_owned())
            .collect()
    }

    fn words(terms: &[&str], text: &str) -> Vec<String> {
        words_with(terms, MatchOptions::default(), text)
    }

    /// Testo degli evidenziati di ogni parola.
    fn highlights(terms: &[&str], options: MatchOptions, text: &str) -> Vec<Vec<String>> {
        find_words(&compile(terms, options), text)
            .iter()
            .map(|m| {
                m.highlights
                    .iter()
                    .map(|h| text[h.clone()].to_owned())
                    .collect()
            })
            .collect()
    }

    fn accents(accents: AccentMode) -> MatchOptions {
        MatchOptions {
            accents,
            ..MatchOptions::default()
        }
    }

    #[test]
    fn literal_finds_words_containing_the_term() {
        assert_eq!(
            words(&["ar"], "Cuocere l'aragosta, poi la carta e la chitarra."),
            ["l'aragosta", "carta", "chitarra"]
        );
        assert_eq!(words(&["a.b"], "axb a.b"), ["a.b"], "il punto è letterale");
    }

    #[test]
    fn empty_text_and_no_match() {
        assert!(words(&["ar"], "").is_empty());
        assert!(words(&["zz"], "carta").is_empty());
    }

    #[test]
    fn case_insensitive_by_default_and_sensitive_on_request() {
        assert_eq!(words(&["ar"], "ARMADIO grande"), ["ARMADIO"]);
        assert_eq!(words(&["città"], "CITTÀ"), ["CITTÀ"]);
        let sensitive = MatchOptions {
            case: CaseMode::Sensitive,
            ..MatchOptions::default()
        };
        assert!(words_with(&["ar"], sensitive, "ARMADIO").is_empty());
    }

    #[test]
    fn wildcards_enabled() {
        assert_eq!(
            words(&["c?rt*"], "la carta e il cortile"),
            ["carta", "cortile"]
        );
        assert_eq!(words(&["c*a"], "casa cosa ca c"), ["casa", "cosa", "ca"]);
        assert!(words(&["c?a"], "ca").is_empty(), "? vuole un carattere");
        assert!(
            words(&["c*a"], "c a").is_empty(),
            "* non attraversa gli spazi"
        );
    }

    #[test]
    fn trailing_star_does_not_swallow_edge_punctuation() {
        let opts = MatchOptions::default();
        assert_eq!(words(&["c?rt*"], "carta,"), ["carta"]);
        assert_eq!(highlights(&["c?rt*"], opts, "(carta).,"), [["carta"]]);
        // Punteggiatura dentro la parola: resta.
        assert_eq!(highlights(&["c*"], opts, "c.a.r."), [["c.a.r"]]);
        // Punteggiatura scritta nel termine: mai tolta.
        assert_eq!(highlights(&["ciao!*"], opts, "ciao!!"), [["ciao!"]]);
        assert_eq!(words(&["ciao!*"], "ciao!!"), ["ciao!"]);
    }

    #[test]
    fn leading_star_does_not_swallow_edge_punctuation() {
        let opts = MatchOptions::default();
        assert_eq!(highlights(&["*arta"], opts, "«carta»"), [["carta"]]);
        assert_eq!(words(&["*arta"], "«carta»"), ["carta"]);
        assert_eq!(highlights(&["*a*"], opts, "¿casa?"), [["casa"]]);
    }

    #[test]
    fn star_next_to_space_never_starts_with_a_space() {
        let opts = MatchOptions::default();
        assert_eq!(highlights(&["* car"], opts, " carta"), [["car"]]);
        assert_eq!(highlights(&["la *"], opts, "la "), [["la"]]);
    }

    #[test]
    fn wildcards_literal() {
        let literal = MatchOptions {
            wildcards: WildcardMode::Literal,
            ..MatchOptions::default()
        };
        assert_eq!(words_with(&["a*b"], literal, "axb a*b"), ["a*b"]);
        assert_eq!(words_with(&["?"], literal, "chi? no"), ["chi?"]);
        assert_eq!(highlights(&["*"], literal, "2*3"), [["*"]]);
    }

    #[test]
    fn phrase_spans_horizontal_spaces_only() {
        assert_eq!(words(&["la car"], "prendi la  carta"), ["la  carta"]);
        assert_eq!(words(&["la car"], "la\tcarta"), ["la\tcarta"]);
        assert_eq!(words(&["la car"], "la\u{a0}carta"), ["la\u{a0}carta"]);
        assert!(words(&["la car"], "la\ncarta").is_empty());
        assert!(words(&["la car"], "la\r\ncarta").is_empty());
    }

    #[test]
    fn edge_spaces_of_the_term_are_ignored() {
        assert_eq!(words(&[" car "], "la carta"), ["carta"]);
        assert_eq!(
            highlights(&[" car"], MatchOptions::default(), "la carta"),
            [["car"]]
        );
    }

    #[test]
    fn distinct_accents() {
        let opts = accents(AccentMode::Distinct);
        assert_eq!(words_with(&["città"], opts, "città citta"), ["città"]);
        assert!(words_with(&["citta"], opts, "città").is_empty());
        assert_eq!(words_with(&["citt"], opts, "città"), ["città"]);
    }

    #[test]
    fn distinct_accents_on_nfd_text() {
        let opts = accents(AccentMode::Distinct);
        let nfd = "citta\u{300} citta";
        assert_eq!(words_with(&["città"], opts, nfd), ["citta\u{300}"]);
        assert_eq!(highlights(&["città"], opts, nfd), [["citta\u{300}"]]);
        // "citta" non trova la "città" decomposta, ma trova "citta".
        assert_eq!(words_with(&["citta"], opts, nfd), ["citta"]);
        assert_eq!(words_with(&["citt"], opts, nfd), ["citta\u{300}", "citta"]);
        // Termine scritto in NFD: portato in NFC, trova entrambe le forme.
        assert_eq!(
            words_with(&["citta\u{300}"], opts, "città citta\u{300}"),
            ["città", "citta\u{300}"]
        );
        // `?` prende la lettera con i suoi segni.
        assert_eq!(
            highlights(&["citt?"], opts, nfd),
            [["citta\u{300}"], ["citta"]]
        );
    }

    /// Hangul NFC e NFD (jamo) con ogni opzione degli accenti: "한국" si trova in entrambe le
    /// forme; "구" non è una parte del "국" decomposto, come non lo è di quello precomposto;
    /// `?` prende la sillaba intera.
    #[test]
    fn hangul_in_nfc_and_nfd_text() {
        let nfd: String = "한국 사람".nfd().collect();
        for mode in [AccentMode::Distinct, AccentMode::Ignored] {
            let opts = accents(mode);
            assert_eq!(
                words_with(&["한국"], opts, "한국 사람"),
                ["한국"],
                "{mode:?}"
            );
            assert_eq!(words_with(&["한국"], opts, &nfd).len(), 1, "{mode:?}");
            assert!(words_with(&["구"], opts, "국").is_empty(), "{mode:?}");
            assert!(words_with(&["구"], opts, &nfd).is_empty(), "{mode:?}");
            let nfd_highlights = highlights(&["한?"], opts, &nfd);
            let shown: Vec<String> = nfd_highlights
                .iter()
                .flatten()
                .map(|h| h.nfc().collect())
                .collect();
            assert_eq!(shown, ["한국"], "{mode:?}");
        }
    }

    #[test]
    fn ignored_accents() {
        let opts = accents(AccentMode::Ignored);
        assert_eq!(
            words_with(&["citta"], opts, "città CITTÀ citta citta\u{300} cittá"),
            ["città", "CITTÀ", "citta", "citta\u{300}", "cittá"]
        );
        assert_eq!(words_with(&["città"], opts, "citta"), ["citta"]);
        assert_eq!(
            highlights(&["citta"], opts, "citta\u{300}!"),
            [["citta\u{300}"]]
        );
        assert!(
            words_with(&["o"], opts, "ø").is_empty(),
            "ø non si decompone"
        );
        let sensitive = MatchOptions {
            case: CaseMode::Sensitive,
            accents: AccentMode::Ignored,
            ..MatchOptions::default()
        };
        assert_eq!(words_with(&["e"], sensitive, "é É"), ["é"]);
    }

    #[test]
    fn punctuation_trimmed_at_edges_but_never_inside_the_match() {
        assert_eq!(words(&["arte"], "città d'arte."), ["d'arte"]);
        assert_eq!(words(&["ar"], "(carta)"), ["carta"]);
        assert_eq!(words(&["(car"], "(carta)"), ["(carta"]);
        assert_eq!(words(&["ar"], "«carta»… ¿carta? “carta”"), ["carta"; 3]);
        assert_eq!(words(&["ar"], "$carta+"), ["$carta+"], "i simboli restano");
        assert_eq!(words(&["ar"], "...ar..."), ["ar"]);
    }

    #[test]
    fn several_matches_in_one_word_give_one_row() {
        let query = compile(&["ar", "ta"], MatchOptions::default());
        let found = find_words(&query, "una carta");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].terms, TermSet::all(2));
        assert_eq!(found[0].highlights, vec![5..9]);

        let query = compile(&["c", "a"], MatchOptions::default());
        let found = find_words(&query, "cacca");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].highlights, vec![0..5]);
    }

    #[test]
    fn phrase_and_word_terms_overlapping_give_one_row() {
        let text = "prendi la carta";
        let query = compile(&["la car", "carta", "zzz"], MatchOptions::default());
        let found = find_words(&query, text);
        assert_eq!(found.len(), 1);
        assert_eq!(&text[found[0].word.clone()], "la carta");
        let mut expected = TermSet::single(0);
        expected.insert(1);
        assert_eq!(found[0].terms, expected);
        assert_eq!(found[0].highlights, vec![7..15]);
    }

    /// Una parola lunghissima senza spazi con molte corrispondenze, e una riga lunghissima
    /// con molte parole: con un algoritmo quadratico questo test non finirebbe.
    #[test]
    fn long_word_and_long_line_stay_linear() {
        let word = "ab".repeat(200_000);
        let query = compile(&["b", "*a"], MatchOptions::default());
        let found = find_words(&query, &word);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].word, 0..word.len());
        assert_eq!(word_text(&word, &found[0], 200).text.len(), 200);
        assert_eq!(snippet(&word, &found[0], 120).text.len(), 120);

        let line = "ab ".repeat(200_000);
        let found = find_words(&compile(&["b"], MatchOptions::default()), &line);
        assert_eq!(found.len(), 200_000);
        let last = &found[found.len() - 1];
        assert_eq!(last.line, 0);
        assert_eq!(snippet(&line, last, 120).text.chars().count(), 120);
    }

    #[test]
    fn one_line_without_terminator() {
        let text = "solo carta";
        let found = find_words(&compile(&["ar"], MatchOptions::default()), text);
        assert_eq!(found[0].line, 0);
        assert_eq!(found[0].line_range, 0..text.len());
    }

    #[test]
    fn line_numbers_and_ranges_with_crlf() {
        let text = "uno\r\ndue\ncarta tre\r\n\r\nar\r\n";
        let query = compile(&["ar", "uno"], MatchOptions::default());
        let found = find_words(&query, text);
        let lines: Vec<(u32, &str, &str)> = found
            .iter()
            .map(|m| (m.line, &text[m.line_range.clone()], &text[m.word.clone()]))
            .collect();
        assert_eq!(
            lines,
            [
                (0, "uno", "uno"),
                (2, "carta tre", "carta"),
                (4, "ar", "ar")
            ]
        );
    }

    #[test]
    fn word_text_truncation_on_and_off_points() {
        let query = compile(&["a"], MatchOptions::default());
        let text = "abcde";
        let found = &find_words(&query, text)[0];
        let whole = word_text(text, found, 5);
        assert!(!whole.truncated && !whole.truncated_start && !whole.truncated_end);
        let cut = word_text(text, found, 4);
        assert_eq!((cut.text.as_str(), cut.truncated), ("abcd", true));
        assert!(!cut.truncated_start && cut.truncated_end);
    }

    #[test]
    fn word_text_cuts_on_char_boundaries_around_a_late_match() {
        let query = compile(&["zz"], MatchOptions::default());
        let text = format!("{}zz{}", "à".repeat(300), "è".repeat(300));
        let found = &find_words(&query, &text)[0];
        let cut = word_text(&text, found, 200);
        assert_eq!(cut.text.chars().count(), 200);
        assert!(cut.text.contains("zz"));
        assert!(cut.truncated && cut.truncated_start && cut.truncated_end);
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
    fn snippet_gives_unused_space_to_the_other_side() {
        let query = compile(&["ar"], MatchOptions::default());
        let text = "carta 0123456789";
        let found = &find_words(&query, text)[0];
        let s = snippet(text, found, 9);
        assert_eq!(s.text, "carta 012");
        assert!(!s.truncated_start && s.truncated_end);
    }

    #[test]
    fn snippet_of_a_long_word_shows_the_match_on_char_boundaries() {
        let query = compile(&["zz"], MatchOptions::default());
        let text = format!("inizio {}zz{} fine", "ü".repeat(500), "ö".repeat(500));
        let found = &find_words(&query, &text)[0];
        let s = snippet(&text, found, 120);
        assert_eq!(s.text.chars().count(), 120);
        assert_eq!(s.highlights.len(), 1);
        assert_eq!(&s.text[s.highlights[0].clone()], "zz");
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
