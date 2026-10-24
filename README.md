# dash

A terminal dashboard (Rust + [ratatui](https://ratatui.rs)) for what needs doing next: KEATS coursework deadlines, module announcements and Q&A threads, plus online assessments and interviews picked out of Gmail.

```
dash         open the dashboard
dash sync    refresh all sources headlessly (e.g. once after login)
```

Keys: `↵` open link · `d` mark done · `u` undo · `r` refresh · `tab` switch panel · `j`/`k` move · `q` quit

## Sources

- **KEATS** runs `~/keats/keats.py` (copy in `keats/keats.py`), which reads the Moodle web service API and writes `~/keats/out/items.json` and `assessments.json`.
- **Coursework deadlines** aren't Moodle events: modules state them as free text in their assessment tables, each in its own format. `dash sync` has Claude read that text (`prompts/coursework.md`) and caches the result until the text changes.
- **Gmail** goes through headless `claude -p` with the claude.ai Gmail connector (`prompts/gmail.md`), so it uses a Claude subscription rather than an API key.

## Requirements

- `python3` and a KEATS token in `~/.config/keats/token` (see `keats.py`)
- [Claude Code](https://docs.claude.com/en/docs/claude-code) logged in, with the Gmail connector enabled
- A Nerd Font in the terminal for the icons

State lives in `~/.local/share/dash/` (`status.json`, `gmail.json`, `coursework.json`, `state.json` for dismissed items).

## Install

```
cargo install --path .
```
