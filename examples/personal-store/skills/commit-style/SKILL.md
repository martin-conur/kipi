---
name: commit-style
description: "Use when writing a git commit message: subject line format, body wrapping and what to explain."
x-kipi:
  scope: personal
  tags: [git]
---

# commit-style

- Subject: imperative mood, ≤ 50 characters, no trailing period.
- Blank line, then a body wrapped at 72 columns explaining why, not what.
- Reference the issue in the body (`Refs #12`), not the subject.
- One logical change per commit.
