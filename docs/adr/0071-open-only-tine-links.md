# 0071 — Open-only Tine links with stable graph identity

Status: Accepted (Martin, 2026-10-04, OG-QE6 task / GH #181).

## Context
External apps and agents need addresses that survive graph folder moves and block moves. Folder names are not unique. Opening must never capture content or create a missing target.

## Decision
Register only `tine`. Forms are `tine://graph/<graph UUID>`, `tine://page/<encoded name>?graph=<graph UUID>` and `tine://block/<persisted block UUID>`. Block links require no page or graph name. Only known graphs are searched; missing targets produce an error. Multiple graph copies prompt for one device-local remembered choice in the existing settings file. A missing or unreadable target in the chosen copy reports an error, even when another copy contains it.

Explicit Copy link lazily creates `logseq/tine-graph-id`: a lowercase random UUID and LF (37 ASCII bytes). It uses Store's no-clobber transaction. Only a clean create collision reads a concurrent winner; I/O, directory-sync and incomplete-publication failures remain errors even when bytes are visible. An established malformed identity is preserved and reported, because inventing another ID would break existing external addresses. This refusal covers torn/sync-delivered metadata, not an attacker controlling the local account. Block copy assigns `id::` through the existing document save door. No link reaches the clipboard until required identities have been saved.

Unit cost: one 37-byte file per graph, written once, plus a temporary file during publication; measured by `link_identity_is_lazy_durable_and_survives_moves`. Ordinary edits on 1- and 60-block pages add zero identity bytes/files or transport. Block-ID copy retains the existing whole-page save cost. Remembered choices add keys to existing device settings, not another layout.

## Consequences
Renamed/moved graphs must be opened once at their new path so Tine knows them. Copied graphs intentionally share an ID. Browsing and opening links never create IDs. Crash before the identity rename leaves absence; after it leaves the complete ID. A crash between graph-ID and block-ID save leaves a reusable graph ID and no published link. Native URL events and forwarded desktop argv queue until the intended WebView is ready. Windows/macOS/mobile runtime registration needs platform release validation; Linux evidence cannot certify those targets.
