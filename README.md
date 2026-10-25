# dash

A terminal dashboard (Rust + [ratatui](https://ratatui.rs)) for what needs doing next: KEATS coursework deadlines, module announcements and Q&A threads, plus online assessments and interviews picked out of Gmail.

```
dash         open the dashboard
dash sync    refresh all sources headlessly (e.g. once after login)
dash login   sign in to KEATS (saves the token; the forum session then renews itself)
```

Keys: `↵` open link · `d` mark done · `u` undo · `r` refresh · `tab` switch panel · `j`/`k` move · `q` quit

## Sources

- **KEATS** runs `keats/keats.py` (built into the binary, written to `~/.local/share/dash/keats.py` on sync), which reads the Moodle web service API and writes `out/items.json` and `assessments.json` next to itself.
- **Coursework deadlines** aren't Moodle events: modules state them as free text in their assessment tables, each in its own format. `dash sync` has Claude read that text (`prompts/coursework.md`) and caches the result until the text changes.
- **Calendars** are iCal feeds listed in `~/.config/dash/calendars`, one `name url` per line (keep it out of git; the links are secret). The KCL timetable's own feed (MyTimetable → Subscribe → Manual subscription) is read directly rather than through Google, which only refreshes subscriptions about once a day; personal Google calendars use their *Secret address in iCal format*. An event that appears in two feeds is shown once, from the first listed.

  ```
  kcl https://scientia-eu-v4-api-d4-02.azurewebsites.net//api/ical/…/timetable.ics
  google https://calendar.google.com/calendar/ical/…/private-…/basic.ics
  ```
- **Gmail** goes through headless `claude -p` with the claude.ai Gmail connector (`prompts/gmail.md`), so it uses a Claude subscription rather than an API key.

## Requirements

- `python3`, `curl`, Firefox, and a KEATS token in `~/.config/keats/token` from `dash login`
- [Claude Code](https://docs.claude.com/en/docs/claude-code) logged in, with the Gmail connector enabled
- A Nerd Font in the terminal for the icons

State lives in `~/.local/share/dash/` (`status.json`, `gmail.json`, `coursework.json`, `calendar.json`, `state.json` for dismissed and seen items).

## Install

```
cargo install --path .
```
