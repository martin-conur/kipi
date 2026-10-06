---
name: code-review
description: Use when reviewing a pull request in an acme repository: checklist for correctness, tests, migrations and API compatibility.
x-kipi:
  scope: acme
  tags: [review]
---

# code-review

1. Read the linked ticket and the PR description; restate the intended change.
2. Check that tests cover the new behavior and that CI ran for the head commit.
3. Flag database migrations that are not backward compatible with the previous release.
4. For API changes, confirm the OpenAPI file changed first and clients were considered.
5. Leave one summary comment with a verdict and a numbered list of findings.
