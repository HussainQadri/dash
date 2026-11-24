# dash

A terminal dashboard (Rust + [ratatui](https://ratatui.rs)) for what needs doing next: KEATS coursework deadlines, module announcements and Q&A threads, online assessments and interviews picked out of Gmail, and a week view of your timetable and calendar.

```
hud          open the dashboard
hud sync     refresh all sources headlessly (e.g. once after login)
hud login    set up calendars (first time) and sign in to KEATS
hud ask …    ask a question about your modules (no question: interactive Claude Code)
```

Keys: `↵` open link · `a` ask · `d` mark done · `u` undo · `c` calendar · `r` refresh · `tab` switch panel · `j`/`k` move · `q` quit

In the ask pane: type and `↵` to ask, `esc` to stop typing (or stop an answer), then `a` follow up · `j`/`k` or the mouse wheel to scroll · `tab` switch model (sonnet/opus, remembered) · `y` copy the answer, `Y` the whole conversation, `c` its code blocks (`^y` while typing) · `↵` open the last link in the answer · `r` new conversation · `esc` back

In the calendar: `h`/`l` previous/next week · `t` this week · `j`/`k` pick an event · `c` or `esc` back

## Sources

- **KEATS** runs `keats/keats.py` (built into the binary, written to `~/.local/share/dash/keats.py` on sync), which reads the Moodle web service API and writes `out/items.json` and `assessments.json` next to itself.
- **Coursework deadlines** aren't Moodle events: modules state them as free text in their assessment tables, each in its own format. `hud sync` has Claude read that text (`prompts/coursework.md`) and caches the result until the text changes.
- **Calendars** are iCal feeds listed in `~/.config/dash/calendars`, one `name url` per line (keep it out of git; the links are secret). `hud login` asks for them the first time and checks each one loads. The KCL timetable's own feed (MyTimetable → Subscribe → Manual subscription) is read directly rather than through Google, which only refreshes subscriptions about once a day; personal Google calendars use their *Secret address in iCal format*. An event that appears in two feeds is shown once, from the first listed.

  ```
  kcl https://scientia-eu-v4-api-d4-02.azurewebsites.net//api/ical/…/timetable.ics
  google https://calendar.google.com/calendar/ical/…/private-…/basic.ics
  ```
- **Ask** (`a`, or `hud ask`) runs headless `claude` in `~/.local/share/dash` with read-only tools (`--restricted`; Read, Grep, Glob, plus `unzip -l` and `pdftotext`) and `prompts/ask.md`, which describes the synced files. The item selected when you press `a` is passed along, and follow-ups resume the same session. `tab` in the pane switches between Sonnet and Opus and is remembered; `HUD_ASK_MODEL` overrides it.
- **Module files** from every module course are mirrored into `~/.local/share/dash/files/<module>/` (files over 100 MB skipped), and Q&A threads keep each post's full text, so ask can read specs, templates and staff answers.
- **Gmail** goes through headless `claude -p` with the claude.ai Gmail connector (`prompts/gmail.md`), so it uses a Claude subscription rather than an API key.

## Requirements

Linux or macOS, with:

- `python3`, `curl`, and a KEATS token in `~/.config/keats/token` from `hud login`
- Firefox or a Chromium-family browser (Chrome, Chromium, Brave, Edge) for `hud login`, which signs in from a private window or a throwaway profile. With neither, it prints the link to open in any browser's private window. The MyTimetable sign-in needs the Chromium-family one, or the timetable link pasted by hand.
- [Claude Code](https://docs.claude.com/en/docs/claude-code) logged in, with the Gmail connector enabled
- A Nerd Font in the terminal for the icons, and 24-bit colour

On macOS, `python3` comes with the Xcode Command Line Tools (`xcode-select --install`) or Homebrew. `hud login` handles the `moodledl://` link with a small AppleScript app it builds in `~/Applications/hud login.app`; on Linux it's a `.desktop` handler.

State lives in `~/.local/share/dash/` (`status.json`, `gmail.json`, `coursework.json`, `calendar.json`, `state.json` for dismissed and seen items).

## Install

```
cargo install --path .
```
