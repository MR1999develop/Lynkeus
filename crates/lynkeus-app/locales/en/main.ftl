# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 MR
#
# Interface strings in English. IDs are stable: the same set is in it/main.ftl.

app-title = Lynkeus Searcher

## Search form

form-folder-label = Folder
form-folder-hint = Path of the folder to search in
form-term-label = Term
form-term-hint = Text to find inside words
action-search = Search
action-cancel = Cancel

## Form errors

error-no-folder = Choose the folder to search in.
error-term-empty = Type the text to find.
error-term-only-wildcards = The term has only * and ?: add at least one letter or digit.
# $max (number): allowed characters.
error-term-too-long = The term is too long: at most { $max } characters.
error-term-line-break = The term cannot contain a line break: search one line at a time.
error-query-too-complex = The term is too complex: shorten it or use fewer wildcards.
# $max (number): allowed terms.
error-query-too-many = You can search for at most { $max } terms together.
error-start-failed = The search could not start: the system is out of resources. Close some programs and try again.

## Results table

column-word = Word
column-file = File
column-position = Position
column-context = Context
results-empty-idle = Choose a folder, type a term and press Search.
results-empty-none = No words found.

## Positions

# $line (number): line, from 1.
location-line = line { $line }
# $page, $line (numbers): page and line in the page.
location-page-line = page { $page }, line { $line }
# $sheet (text): sheet name; $cell (text): cell, e.g. B3.
location-cell = { $sheet }!{ $cell }
# $slide (number): slide.
location-slide = slide { $slide }
# $paragraph (number): paragraph.
location-paragraph = paragraph { $paragraph }
# $part (text): message part (mail-part-*); $line (number): line.
location-mail = { $part }, line { $line }
mail-part-subject = subject
mail-part-from = sender
mail-part-to = recipients
mail-part-body = body

## Status

status-idle = Ready.
# $searched, $hits, $skipped, $binary, $warnings, $seconds (numbers).
status-running = Searching: { $searched ->
        [one] 1 file read
       *[other] { $searched } files read
    }, { $hits ->
        [one] 1 word found
       *[other] { $hits } words found
    }, { $skipped } skipped, { $binary } binary, { $warnings ->
        [one] 1 warning
       *[other] { $warnings } warnings
    } ({ $seconds } s)
# Same as status-running.
status-completed = Search complete: { $searched ->
        [one] 1 file read
       *[other] { $searched } files read
    }, { $hits ->
        [one] 1 word found
       *[other] { $hits } words found
    }, { $skipped } skipped, { $binary } binary, { $warnings ->
        [one] 1 warning
       *[other] { $warnings } warnings
    } ({ $seconds } s)
# Same as status-running.
status-cancelled = Search cancelled: { $searched ->
        [one] 1 file read
       *[other] { $searched } files read
    }, { $hits ->
        [one] 1 word found
       *[other] { $hits } words found
    } ({ $seconds } s)
# $max (number): result limit.
status-limit-reached = Reached the limit of { $max } results: narrow the search with a more specific folder or term.
status-worker-lost = The search stopped because of an internal error. Try again; if it happens again, please report the problem.
# $file (text): path of the file being read.
status-current-file = Reading: { $file }

## Warnings

warning-not-found = Not found
warning-access-denied = Access denied
warning-in-use = In use by another program
warning-io = Read error
warning-encrypted = Password protected
warning-corrupt = Damaged file
warning-timeout = Timed out
warning-limit-depth = Too many nested archives
warning-limit-entries = Too many files in the archive
warning-limit-entry-size = File in the archive too large
warning-limit-archive-size = Archive too large once decompressed
warning-limit-ratio = Suspicious compression, possibly a malicious archive
warning-limit-memory = Out of memory while reading
warning-content-mismatch = Content does not match the extension
warning-no-text = No text, maybe a scan
