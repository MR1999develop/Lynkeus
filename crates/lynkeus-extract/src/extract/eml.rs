// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Messaggi e-mail (.eml) con `mail-parser`.
//!
//! - Il messaggio è un documento con unità [`Location::Mail`]: oggetto (riga 1), mittenti e
//!   destinatari (`To`, poi `Cc`; un indirizzo per riga, `Nome <indirizzo>`), corpo (una
//!   unità per riga non vuota, righe numerate da 1 su tutte le parti del corpo). Il corpo è
//!   quello di testo; se manca, l'HTML ridotto a testo (`mail_parser::html_to_text`).
//! - Gli allegati vengono dopo la fine del messaggio, come archivi: ognuno è un documento
//!   con `inner` = nome dell'allegato reso etichetta (`allegato` se senza nome; i nomi
//!   ripetuti diventano "allegato (2)", "foto (2).jpg") e formato dal nome; un messaggio
//!   allegato (`message/rfc822`) si legge come .eml. Valgono i limiti
//!   degli archivi: profondità (gli allegati sono un livello in più), voci, byte per voce e
//!   totali (sui byte decodificati, controllati prima di decodificare).
//!
//! Il messaggio si legge in memoria entro il massimo per voce.

use lynkeus_core::{Location, MailPart};
use mail_parser::{Address, AddressList, MessageParser, MessagePart, PartKind};

use crate::error::ExtractError;
use crate::event::{Stopped, WarningKind};
use crate::extract::archive::{UniqueLabels, check_depth, entry_label};
use crate::extract::{Ctx, EntryRole, Fail, Input, read_document};
use crate::format::Format;
use crate::limits::LimitKind;

/// Nome di un allegato senza nome.
const UNNAMED: &str = "allegato";

/// Estrae un messaggio e poi i suoi allegati.
pub(crate) fn extract(ctx: &mut Ctx<'_>, input: Input<'_>) -> Result<(), Fail> {
    let bytes = input.into_bytes(ctx)?;
    let Some(message) = MessageParser::default().parse(&bytes) else {
        return Err(ExtractError::Corrupt {
            reason: "messaggio vuoto o non valido".to_owned(),
        }
        .into());
    };
    if let Some(subject) = message.subject() {
        ctx.unit(mail(MailPart::Subject, 1), subject)?;
    }
    if let Some(from) = message.from() {
        address_lines(ctx, MailPart::From, [Some(from)])?;
    }
    address_lines(ctx, MailPart::To, [message.to(), message.cc()])?;
    let mut line = 0_u32;
    for body in message.text_bodies() {
        for text in body.lines() {
            line = line.saturating_add(1);
            ctx.unit(mail(MailPart::Body, line), text)?;
        }
    }
    // Il messaggio finisce qui: gli allegati sono documenti a sé.
    ctx.finish()?;
    let attachments: Vec<MessagePart<'_>> = message.attachments().collect();
    if attachments.is_empty() {
        return Ok(());
    }
    if let Err(error) = check_depth(ctx) {
        ctx.report(error)?;
        return Ok(());
    }
    let count = u64::try_from(attachments.len()).unwrap_or(u64::MAX);
    if let Err(kind) = ctx.budget().add_entries(count) {
        ctx.warn(WarningKind::LimitExceeded(kind))?;
        return Ok(());
    }
    let mut labels = UniqueLabels::default();
    for part in attachments {
        let label = labels.unique(entry_label(part.attachment_name().unwrap_or(UNNAMED)));
        ctx.within(label, |ctx| read_attachment(ctx, &part))?;
        if ctx.budget().exhausted() {
            break;
        }
    }
    Ok(())
}

fn mail(part: MailPart, line: u32) -> Location {
    Location::Mail { part, line }
}

/// Un indirizzo per riga, numerate da 1 su tutti gli elenchi.
fn address_lines<const N: usize>(
    ctx: &mut Ctx<'_>,
    part: MailPart,
    lists: [Option<AddressList<'_>>; N],
) -> Result<(), Stopped> {
    let mut line = 0_u32;
    for list in lists.into_iter().flatten() {
        for address in list.iter() {
            let mailboxes: Vec<_> = match address {
                Address::Mailbox(mailbox) => vec![mailbox],
                Address::Group(group) => group.mailboxes().collect(),
            };
            for mailbox in mailboxes {
                let text = match (mailbox.name(), mailbox.address()) {
                    (Some(name), Some(address)) => format!("{name} <{address}>"),
                    (Some(only), None) | (None, Some(only)) => only.to_owned(),
                    (None, None) => continue,
                };
                line = line.saturating_add(1);
                ctx.unit(mail(part, line), &text)?;
            }
        }
    }
    Ok(())
}

/// Legge un allegato già "entrato" nel contesto.
fn read_attachment(ctx: &mut Ctx<'_>, part: &MessagePart<'_>) -> Result<(), Stopped> {
    let declared = if matches!(part.kind(), PartKind::Message(_)) {
        Format::Eml
    } else {
        Format::for_name(ctx.current_name()).unwrap_or(Format::Text)
    };
    let size = u64::try_from(part.decoded_len()).unwrap_or(u64::MAX);
    if size > ctx.limits().max_entry_size.bytes() {
        return ctx.warn(WarningKind::LimitExceeded(LimitKind::EntrySize));
    }
    if let Err(kind) = ctx.budget().add_bytes(size) {
        return ctx.warn(WarningKind::LimitExceeded(kind));
    }
    if ctx.entry_role() == EntryRole::Capture {
        return ctx.capture(&mut part.decoded().as_ref());
    }
    read_document(ctx, declared, Input::Bytes(part.decoded().into_owned()))
}

#[cfg(test)]
mod tests {
    use crate::event::ExtractEvent;
    use crate::extract::{ExtractOptions, extract_bytes};

    /// Due allegati senza nome e due con lo stesso nome: quattro documenti distinti.
    #[test]
    fn repeated_attachment_names_are_numbered() {
        let attachment = |name: &str| {
            let disposition = if name.is_empty() {
                "attachment".to_owned()
            } else {
                format!("attachment; filename=\"{name}\"")
            };
            format!(
                "--confine\r\nContent-Type: text/plain\r\nContent-Disposition: {disposition}\r\n\r\naragosta\r\n"
            )
        };
        let message = format!(
            "From: a@esempio.it\r\nSubject: prova\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"confine\"\r\n\r\n--confine\r\nContent-Type: text/plain\r\n\r\ncorpo\r\n{}{}{}{}--confine--\r\n",
            attachment(""),
            attachment(""),
            attachment("nota.txt"),
            attachment("nota.txt"),
        );
        let mut events = Vec::new();
        let result = extract_bytes(
            "posta.eml",
            message.into_bytes(),
            &ExtractOptions::default(),
            &mut events,
        );
        assert_eq!(result, Ok(()));
        let documents: Vec<Vec<String>> = events
            .iter()
            .filter_map(|event| match event {
                ExtractEvent::DocumentStart { inner, .. } => Some(inner.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            documents,
            [
                Vec::new(),
                vec!["allegato".to_owned()],
                vec!["allegato (2)".to_owned()],
                vec!["nota.txt".to_owned()],
                vec!["nota (2).txt".to_owned()],
            ]
        );
    }
}
