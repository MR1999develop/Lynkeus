# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 MR
#
# Interface strings in English. IDs are stable: the same set is in it/main.ftl.

app-title = Lynkeus Searcher

## Menu

menu-file = File
menu-edit = Edit
menu-search-menu = Find
menu-view = View
menu-tools = Tools
menu-help = Help
menu-add-folder = Add folder…
menu-add-file = Add file…
menu-recent-roots = Recent folders and files
menu-quit = Quit
menu-select-all = Select all rows
menu-select-none = Deselect all
menu-search = Search
menu-cancel = Cancel search
menu-find-terms = Type a term
menu-add-term = Add a term
menu-recent-terms = Recent terms
menu-filters = Filters
menu-show-results = Results
menu-show-warnings = Warnings
menu-search-panel = Search panel
menu-theme = Theme
menu-language = Language
menu-about = About Lynkeus Searcher
theme-system = Same as system
theme-light = Light
theme-dark = Dark
language-system = System language
language-italian = Italiano
language-english = English

## Reasons for disabled items

reason-search-running = A search is already running.
reason-no-search = No search is running.
reason-no-results = There are no results to select.
reason-not-results-tab = Open the Results tab to select rows.
# The maximum number of terms is in error-query-too-many.
reason-too-many-terms = You have reached the maximum number of terms.
reason-no-recent = No recent items yet.
reason-one-term = Add at least two terms to choose how to combine them.
reason-filters-default = The filters are already the default ones.

## Search form

form-roots-title = Where to search
form-roots-empty = Drop folders or files here, or use the buttons.
form-add-folder = Add folder…
form-add-file = Add file…
form-root-remove-tooltip = Remove from the list
# $path (text): path of the folder or file.
form-root-remove-name = Remove { $path } from the list
root-kind-folder = folder
root-kind-file = file
root-kind-missing = not found
root-kind-pending = checking
form-path-label = Path
form-path-hint = Paste a path and press Enter
form-path-checking = Checking the path…
form-terms-title = Terms
form-term-hint = Text to find, e.g. ar
form-term-hint-more = Another term
# $number (number): row position, from 1.
form-term-name = Term { $number }
form-term-remove-tooltip = Remove this term
# $number (number): row position, from 1.
form-term-remove-name = Remove term { $number }
form-add-term = + Add a term
form-options-title = Options
option-case = Match case
option-case-tooltip = When on, "ar" does not find "AR".
option-accents = Ignore accents
option-accents-tooltip = When on, "citta" also finds "città".
option-wildcards = Wildcards * and ?
option-wildcards-tooltip = * stands for zero or more characters, ? for one character. When off, * and ? are searched as they are.
combine-title = With several terms, show the files that contain:
combine-any = At least one term
combine-any-tooltip = Every word that contains one of the terms.
combine-all = All terms in the same file
combine-all-tooltip = Only the files where all the terms appear.
filters-title = Filters
filters-title-customized = Filters (changed)
filters-extensions = Extensions
filters-extensions-all = All
filters-extensions-include = Only these
filters-extensions-exclude = All except these
filters-extensions-hint = e.g. txt, md, docx
filters-skip-dirs = Folders to skip
filters-skip-dirs-caption = One name per line; applies at every level.
filters-hidden = Include hidden and system files
filters-online = Include online-only files
filters-online-tooltip = Reading them downloads them from the cloud (for example from OneDrive).
filters-max-size = Skip files larger than
filters-max-size-unit = { " " }MB
filters-reset = Restore the default filters
action-search = Search
action-cancel = Cancel
action-close = Close
drop-hint = Drop to add to the list
dialog-add-folder = Add folders to search in
dialog-add-file = Add files to search in

## Form errors

error-no-folder = Add at least one folder or file to search in.
error-path-not-found = No file or folder with this path.
error-term-empty = Type the text to find, or remove this row.
error-term-only-wildcards = The term has only * and ?: add at least one letter or digit.
# $max (number): allowed characters.
error-term-too-long = The term is too long: at most { $max } characters.
error-term-line-break = The term cannot contain a line break: search one line at a time.
error-query-too-complex = The term is too complex: shorten it or use fewer wildcards.
# $max (number): allowed terms.
error-query-too-many = You can search for at most { $max } terms together.
error-extensions-missing = Type at least one extension, or choose "All".
# $extension (text): the extension typed by the user.
error-extension-invalid = "{ $extension }" is not a valid extension: write it without inner dots, spaces or wildcards, e.g. txt.
# $extension (text): the extension typed; $max (number): allowed characters.
error-extension-too-long = "{ $extension }" is too long: at most { $max } characters.
error-start-failed = The search could not start: the system is out of resources. Close some programs and try again.

## Tabs, tables and empty states

# $count (number): result rows.
tab-results = Results ({ $count })
# $count (number): warnings.
tab-warnings = Warnings ({ $count })
column-word = Word
column-term = Term
column-file = File
column-position = Location
column-context = Context
column-reason = Reason
column-sort-tooltip = Sort by this column; click again to reverse the order.
empty-add-root-title = Where shall we start?
empty-add-root-text = Add a folder or a file to search in: drop them here or use the buttons.
empty-write-term-title = Type what to find
empty-write-term-text = Type a term in the panel on the left and press Search (or F5). You will find the words that contain it.
empty-searching-title = Searching…
empty-searching-text = The words found will appear here as they come.
empty-none-title = No words found
empty-none-text = Try a shorter term, wildcards, "Ignore accents", or check the filters.
warnings-empty = No warnings: every file was read.
banner-limit-title = Too many results.
# $max (number): result limit.
banner-limit-text = The search stopped at { $max } rows: use a more specific term, fewer folders or the filters.

## About

# $version (text): program version.
about-version = Version { $version }
about-description = Finds the words that contain a text, in files and folders.
about-license = Free software, released under the GNU GPL version 3.

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
# $done (number): files already examined; $seen (number): files found so far.
status-progress = { $done } of { $seen } files
# $count (number): selected rows.
status-selected = { $count ->
        [one] 1 row selected
       *[other] { $count } rows selected
    }

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
