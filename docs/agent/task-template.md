# Task template

Every implementation task handed to a coding agent uses this structure
(architecture addendum §15). Tasks must be small enough to review independently.
Agents record newly discovered work instead of expanding scope.

```markdown
# Task <ID> — <Name>

- **Phase:** …
- **Status:** todo | in-progress | review | done | blocked
- **Depends on:** …

## Objective
What this task must accomplish.

## Dependencies
Which previous tasks/contracts must already exist.

## Required reading
- relevant ADRs
- architecture docs
- related implementation files

## Allowed scope
Directories/files the agent may modify.

## Explicit non-goals
Things the agent must not implement.

## Required behavior
Concrete functional requirements.

## Failure behavior
Expected behavior under cancellation, invalid input, process death,
filesystem errors, etc.

## Acceptance criteria
Observable pass/fail conditions.

## Tests required
Unit/integration/E2E/manual checks.

## Commands to run
Formatting, linting, tests and build commands.

## Completion report
- summary
- files changed
- tests actually executed
- results
- architectural decisions
- known limitations
- follow-up tasks
```
