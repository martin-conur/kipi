---
name: pr-conventions
description: "Use when opening or updating a pull request in an acme repository: title format, description template and labels."
x-kipi:
  scope: acme
  tags: [git, review]
---

# pr-conventions

- Title: `<ticket>: <imperative summary>`.
- Description: Problem, Solution, Testing, Rollout. Link the ticket.
- Label `breaking` for any change that needs a coordinated deploy.
- Keep PRs under ~400 changed lines; split otherwise.
