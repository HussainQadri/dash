You are scanning the user's Gmail for recruiting tasks that need action: online assessments (OAs), coding tests, assessment centres, and phone/video/onsite interviews.

1. Search with search_threads using:
   after:{since} (assessment OR "online assessment" OR "coding challenge" OR "coding test" OR interview OR hackerrank OR codility OR codesignal OR hirevue OR "next steps" OR "assessment centre")
   Run one or two extra searches only if they are likely to find more (e.g. from:greenhouse.io, from:lever.co, from:myworkday.com, from:ashbyhq.com).
2. Open a thread with get_thread when the snippet doesn't make the company, deadline, or link clear.
3. One item per task: if several threads concern the same task (reminders, corrections, new links), return only the original thread, updated with the latest details.
4. Ignore newsletters and digests (e.g. "LeetCode Weekly Digest"), job alerts and recommendations, marketing, events and webinars, and application-received confirmations that ask for no action.

Items already being tracked (JSON). If a newer email shows one was completed, submitted, scheduled, rescheduled, cancelled or rejected, include it with the updated fields:
{open}

Return every relevant thread found, plus any tracked item that changed:
- thread_id: the Gmail thread id
- company
- kind: "OA", "assessment" or "interview"
- title: short action, e.g. "HackerRank OA", "First-round video interview"
- deadline: due date or interview time in local time as "YYYY-MM-DD" or "YYYY-MM-DDTHH:MM", or null if none is given
- url: the direct assessment or scheduling link if the email has one, else null
- status: "pending" (still needs doing, or the interview is upcoming), "done" (completed/submitted/interview took place), or "rejected"

If nothing is relevant, return an empty list.
