# srcmap agent guide

## Write for the reader

Write code comments, docs, commit messages, PR text, and changelog entries
for a reader who did not see the session. Do not put the conversation, the
prompt, discarded attempts, or the edit history into them. Describe the code
as it is now. For a regression, state what breaks without the code, and link
the issue. `scripts/check-comment-quality.mjs` blocks narration and history
comments in the hk pre-commit hook, the CI "JS Hygiene" job, and Claude edits.
