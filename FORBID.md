# TENUKI Forbid

## 0. Purpose

This file lists destructive patterns.

Do not apply these bans by keyword alone.

Judge whether the code is:
- changing authority in the wrong layer
- hiding failure
- duplicating responsibility
- treating observation as committed truth
- preserving legacy behavior as current specification
- making temporary rescue permanent

Stable files must contain principles, prohibitions, and checklists.
They must not contain active phase plans.

## 1. Authority Violations

Do not use observation as authority.

Do not treat a found file as truth just because it exists.

Do not treat a saved value, previous success, old comment, old plan, old memory,
legacy path, or detected candidate as current authority.

Do not commit effective values before verification.

Do not let backend startup repair config shape.

Do not let a downstream consumer infer, select, replace, repair, provision, save,
or recommit authority.

Do not make temporary rescue permanent without an explicit adopt reason and
commit path.

Do not use “it worked last time” as current evidence.

Do not allow download only because a backend is named in authority config.

## 2. Route and Pipeline Violations

Do not duplicate core translation logic per route.

Do not create separate hidden translation behavior for /translate, /list,
or future /v1 routes.

Do not erase route policy differences in the name of commonization.

Do not cover failure with dummy success responses.

Do not let /list update dictionary, cache, committed dictionary authority,
or input analysis.

Do not let List output directory creation modify committed dictionary authority.

Do not mix request/response shape handling with core translation logic.

## 3. File and Output Violations

Do not treat partial writes as completed output.

Do not truncate or delete output files in a way that breaks continuation or resume assumptions.

Do not add hardcoded authority paths.

Do not treat derived artifacts as authority.

Do not treat dict.bin as dictionary authority.

Do not reuse dict.bin across slot or language changes unless it is regenerated
from the current dict.txt authority.

Do not let output placement become dictionary authority.

## 4. Legacy and Migration Violations

Do not copy legacy or mid-layer style into new code.

Do not treat old slot names such as S_0001 or s_0001 as current naming rules.

Do not place legacy compatibility and current specification at the same level
in comments or tests.

Do not let old handoff MD, old active plans, old comments, or old memories
outrank current source code and current user instruction.

Do not keep obsolete plan text in stable instruction files.

Do not mix current principles with temporary project status.

Do not remove legacy detection merely because legacy is not current.
Legacy may be observed, normalized, or migrated when the current authority layer
explicitly adopts that action.

## 5. UI and Worker Violations

Do not run size-proportional I/O, parse, or render work on the UI thread.

Do not add user-facing strings directly where localized text helpers already exist.

Do not let UI decide authority because it is convenient for rendering.

Do not let preview state become execution authority without an explicit readiness
or plan step.

Do not apply worker results unless they still match the current target,
source, generation, or session.

Do not split files only to reduce line count while leaving responsibility
boundaries unclear.

## 6. Comment and Naming Violations

Do not write comments as history.

Do not preserve comments that describe old behavior as if it were current.

Do not use broad words such as “dictionary” when the code means one of:
- dict.txt authority
- dict.bin derived artifact
- TranslationCache
- NewEntriesCache
- committed dict_slot
- list output directory

Do not use “read only” if the code also materializes a committed path.
Use wording like:
“does not infer or commit authority; may materialize committed path.”

Do not use “pattern dictionary” unless that exact current component exists.

Do not mix encoding repair, text protection, model transport, and output restoration
in one comment.

Do not name tests so that legacy behavior looks equal to current specification.

## 7. Operational Violations

Do not skip the alignment step after implementation.

Do not stop at “tests pass” if naming, comments, side effects, or entry flow
still contradict the current design.

Do not use old phase plans as task authority.

Do not let a temporary implementation note become a stable instruction.

If rescue is needed, state:
- what observation was found
- why it is adopted
- who commits it
- what happens on failure

## 8. Not Forbidden

The following are not forbidden when the authority boundary is already fixed:

- materializing a committed path
- validating a committed path
- regenerating a derived artifact from current authority
- detecting legacy paths as observation
- normalizing legacy input into current rules
- creating output paths from an accepted RunPlan
- keeping route policy differences
- making a small local fix that moves the code toward the final responsibility boundary
- delaying file splitting until the responsibility boundary is clear

Do not block these actions merely because they resemble a forbidden pattern by name.
Check whether they actually change authority, hide failure, or move responsibility
to the wrong layer.