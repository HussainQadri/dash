Below is text gathered from the user's KEATS (Moodle) module pages at King's College London, one section per module code. Today is {today}; the academic year is {y1}/{y2}.

Extract every deadline the user must meet for assessed coursework, plus required actions with a stated deadline (e.g. a mandatory sign-up form). One item per submission: if several parts share a single deadline, merge them into one item (e.g. "Scala coursework (Core 1–3, Main 2–5)").

- module: the module code of the section the deadline came from
- title: short name, e.g. "C++ coursework", "Coursework 1", "Small group project"
- weight: e.g. "50%", or null
- due: "YYYY-MM-DDTHH:MM" when a time is given, "YYYY-MM-DD" when only a date is, null when it is TBD or not stated. When the year is omitted, Sep–Dec means {y1} and Jan–Aug means {y2}.
- note: a very short qualifier when useful (e.g. "TBD: last week of teaching or revision week", "via Gradescope"), else null

Do not include exams, feedback or release dates, formative/self-assessment quizzes, or general rules (mitigating circumstances, late penalties). If a module states no deadlines, return nothing for it. Use only what the text says; never guess a date. Text in ~~tildes~~ is struck through on the page: it has been superseded, so ignore it and use the date that replaces it.

{text}
