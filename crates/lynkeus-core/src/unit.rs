// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Unità di testo: il pezzo di documento che un estrattore produce e in cui si cerca.

use serde::{Deserialize, Serialize};

use crate::location::Location;

/// Un pezzo di testo con la sua posizione: una riga di un file di testo, una cella,
/// un paragrafo, una diapositiva, una riga di una pagina PDF, una parte di un'e-mail.
///
/// Viaggia nel protocollo del processo estrattore e nella cache; `text` può contenere più
/// righe (es. una diapositiva), e [`find_words`](crate::find_words) le numera da 0.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextUnit {
    /// Dove si trova il testo nel documento.
    pub location: Location,
    /// Il testo estratto, in UTF-8.
    pub text: String,
}
