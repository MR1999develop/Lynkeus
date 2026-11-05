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
menu-open = Open
menu-reveal = Show in folder
menu-export = Export…
menu-export-settings = Export settings…
menu-copy = Copy
menu-preview = Preview
menu-next-in-file = Next result in file
menu-previous-in-file = Previous result in file
menu-next-file = Next file
menu-previous-file = Previous file
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
reason-no-selection = Select at least one row.
reason-no-current-row = Click a row of the results.
reason-last-in-file = This is the last result in this file.
reason-first-in-file = This is the first result in this file.
reason-last-file = This is the last file in the results.
reason-first-file = This is the first file in the results.
reason-nothing-to-export = There are no results to export.
reason-export-running = An export is already running.
reason-export-no-selection = No rows selected: select some, or choose "All rows" in Export settings.

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
dialog-export = Export the results

## Form errors

error-no-folder = Add at least one folder or file to search in.
error-path-not-found = No file or folder with this path.
# $max (number): maximum number of search locations in the list.
error-path-too-many-roots = The list already has { $max } search locations, the maximum. Remove some to add others.
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
warning-extractor-unavailable = Document reader not started

## Preview

preview-empty = Click a result to see its preview.
preview-loading = Reading the file…
preview-context-only = The full preview of this document is not available; here is the context found.
preview-previous-file = « File
preview-previous = ‹ Previous
preview-next = Next ›
preview-next-file = File »
preview-open = Open
# $file (text): file path; $location (text): location, e.g. "line 12".
preview-title = { $file } — { $location }
# $reason (text): reason, as in the Warnings tab.
preview-unreadable = Preview not available: { $reason }.
preview-changed = Preview not available: the file changed after the search. Search again to update the results.
preview-no-thread = Preview not available: the system has no free resources. Close some programs and try again.

## Messages above the table

notice-executable = For safety, programs and scripts are not opened from here: use "Show in folder" and open it from there if you trust it.
notice-archive-entry = Archive entries cannot be opened right now because the document reader is not running: use "Show in folder" to open the archive.
# $file (text): file path.
notice-open-failed = Error: { $file } did not open. Check that it still exists and that there is a program to open it.
# $file (text): file path.
notice-reveal-failed = Error: { $file } could not be shown in its folder. Check that it still exists.
# $count (number): rows copied.
notice-copied = { $count ->
        [one] 1 row copied to the clipboard.
       *[other] { $count } rows copied to the clipboard.
    }
# $count (number): rows written; $file (text): exported file.
notice-exported = { $count ->
        [one] 1 row exported to { $file }.
       *[other] { $count } rows exported to { $file }.
    }
# $file (text): file that was to be written.
notice-export-failed = Error: export to { $file } failed; an existing file was left untouched. Choose another folder or check the free space.
notice-no-thread = Error: the system has no free resources. Close some programs and try again.

## Export

export-dialog-title = Export settings
export-format-title = Format
# $separator (text): character between columns.
export-format-csv-choice = CSV for spreadsheets (columns separated by { $separator })
export-format-csv = CSV
export-format-txt = Text with tab-separated columns
export-scope-title = Rows
# $count (number): rows in the table.
export-scope-visible = All rows ({ $count })
# $count (number): selected rows.
export-scope-selected = Selected rows only ({ $count })
export-dialog-confirm = Export…
# Suggested file name, without extension.
export-file-name = results

## Document cache (Tools menu)

menu-cache = Document cache
menu-cache-enabled = Use the cache
menu-cache-enabled-hint = Keeps the text read from PDFs, documents and archives, so the next search in the same files is faster. The text stays in the app folder until you clear the cache.
menu-cache-size = Maximum size
# $mib (number): size in megabytes.
menu-cache-size-choice = { $mib } MB
menu-cache-clear = Clear the cache
reason-no-cache = The cache folder is not available (the system does not provide one or it is not private).
reason-cache-clearing = The cache is being cleared.
# $mib (number): megabytes freed.
notice-cache-cleared = Cache cleared: { $mib } MB freed.
notice-cache-clear-failed = Error: the cache was not fully cleared. Close the other Lynkeus Searcher windows and try again.

## Archive entries that do not open

# $file (text): archive entry, as shown in the table.
notice-entry-encrypted = Error: { $file } is password protected and cannot be extracted to open it. Open the archive with its own program ("Show in folder") and enter the password there.
# $file (text): archive entry, as shown in the table.
notice-entry-limit = Error: { $file } exceeds the safety limits for archives (size, compression or time) and was not extracted. Open the archive with its own program ("Show in folder").
# $file (text): archive entry, as shown in the table.
notice-entry-corrupt = Error: { $file } cannot be extracted because the archive is damaged or changed after the search. Search again or open the archive with its own program.
# $file (text): archive entry, as shown in the table.
notice-entry-unreadable = Error: { $file } could not be extracted: the archive cannot be read (moved, in use or no permission) or there is no free space. Check and try again.

## Right-click and diagnostics (Tools menu)

# Text of the item in the right-click menu of Explorer and Nautilus.
context-menu-label = Search with Lynkeus
menu-context-menu = "Search with Lynkeus" in the right-click menu
menu-context-menu-hint = Adds "Search with Lynkeus" to the right-click menu of files and folders; the item opens this window with the chosen item among the search locations. On Windows 11 it is under "Show more options".
reason-no-context-menu = The right-click item is not available on this system.
reason-context-menu-updating = The right-click item is being updated.
notice-context-menu-exe-path = Error: the right-click item was not added because the program path contains characters that are not allowed. Move Lynkeus Searcher to a folder with a simple name and try again.
notice-context-menu-too-long = Error: the right-click item was not added because the program path is too long. Move Lynkeus Searcher to a folder closer to the root of the drive and try again.
notice-context-menu-exe-dir = Error: the right-click item was not added because other users can change the program or its folder. Move Lynkeus Searcher to your user folder or to Program Files and try again.
notice-context-menu-foreign = Error: the right-click menu already has an item with the same name from another program; it was not changed.
notice-context-menu-io = Error: the right-click item was not updated. Try again; if it happens again, check the permissions of your profile.
# $max (number): maximum number of search locations in the list.
notice-roots-limit = Warning: the list already has { $max } search locations, the maximum, so the others were not added. Remove some and try again.
# $count (number): app folders that were refused.
notice-private-dirs = { $count ->
        [one] Warning: one app folder can be accessed by other users and is not used in this session for safety (cache, log or settings may not be saved). Check the permissions of the Lynkeus Searcher folders in your profile.
       *[other] Warning: { $count } app folders can be accessed by other users and are not used in this session for safety (cache, log or settings may not be saved). Check the permissions of the Lynkeus Searcher folders in your profile.
    }
menu-log-level = Log detail
log-level-error = Errors only
log-level-warn = Errors and warnings
log-level-info = Information too
log-level-debug = Everything (for diagnosis)
