# File Translate Feature Positioning

## 0. Purpose

This is a steering document, not an active plan and not a current task list.

It defines how File Translate should be positioned inside TENUKI:
an independent feature that borrows the existing UI stage and existing translation transport
without reshaping the whole application or weakening authority boundaries.

Current source code and current user instruction decide the active implementation state.

## 1. Core Position

File Translate is not a simple extension of normal translation.

It is an independent feature lane that borrows the existing TENUKI stage.

Meaning:
- separate processing lane
- existing UI frame may be borrowed
- existing components may be reused
- the stage itself must not be broken

## 2. Relationship to list

File Translate may use the existing `/list` transport.

That does not mean File Translate and `/list` have the same feature meaning.

Use `/list` as a transport component when useful.
Do not force File Translate into the normal translation core just to make the architecture look unified.

## 3. What File Translate is

File Translate is:
- an independent feature
- an independent processing lane
- a workflow staged inside the existing TENUKI UI
- allowed to borrow existing UI/result/log/transport components

File Translate is not:
- normal translation
- a reason to reshape the whole UI
- a reason to weaken authority boundaries
- a reason to create project-wide abstractions too early

## 4. Borrowed components

File Translate may borrow:
- existing UI frame
- existing center result area
- existing log layer
- existing translation transport
- existing helper functions when their meaning still fits

Borrowing is not redefinition.

Do not treat borrowed parts as proof that File Translate has the same meaning as normal translation.

## 5. Wrong reasons to change UI or authority

Do not change the feature shape merely because:
- it makes authority easier to express
- it makes a commit point easier to add
- it moves logic closer to backend
- it makes commonization look cleaner
- it avoids writing a feature-local controller

The feature shape comes first.
Then the internal implementation must satisfy that shape without breaking authority boundaries.

## 6. UI Position

File Translate uses the existing UI as its stage.

Correct direction:
- borrow existing layout
- keep the existing center result/work area
- put feature-specific navigation and preview in side panels
- use existing log structure without replacing the whole center UI

Wrong direction:
- replace the center with a File Translate-only screen
- add unrelated operation systems
- change normal translation UI contracts because File Translate exists

## 7. Authority Position

File Translate may be independent, but it must still respect TENUKI authority rules.

Do not:
- turn preview into authority
- let UI convenience create hidden commit points
- let `/list` update dictionary/cache/input analysis
- treat run-only output placement as committed dictionary authority
- weaken normal translation boundaries

Do:
- keep preview as observation
- fix execution inputs through readiness or RunPlan
- treat List output directory as output placement
- keep committed dict_slot separate from File Translate List output directory

## 8. Current State

This file does not define current state.

For current behavior, inspect source code and tests.

Especially verify:
- whether Run/Stop is currently implemented
- how readiness is evaluated
- what output schema runner currently writes
- how dict_slot and List output directory are separated
- whether `/list` side effects remain disabled

## 9. One-line Summary

File Translate is an independent feature that borrows the existing TENUKI UI stage and translation transport. Borrow components when useful, but do not break the stage, blur feature meaning, or weaken authority boundaries.