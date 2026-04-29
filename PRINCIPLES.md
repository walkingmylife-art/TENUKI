# TENUKI Principles

This is a steering document, not a harness and not an active plan.

## 0. Purpose

This file is not an active plan.

It defines how TENUKI decisions should be judged:
what is authority, what is observation, who adopts, who commits,
and what downstream code may or may not decide.

Current source code and current user instruction are the primary evidence.

Old plans, old comments, old memories, previous success, existing files,
detected candidates, and leftover artifacts are observation until the responsible
layer adopts and commits them.

Do not use this file to store phase plans, temporary status, or current task lists.

## 1. Core Rule

Observation is not authority.

A value becomes authority only when the responsible layer adopts it from current
evidence and commits it as the effective value.

Downstream code must not infer, replace, repair, or recommit authority.

Downstream code may validate, use, and materialize authority that has already
been committed.

## 2. Terms

Authority:
A decision path that owns the decision, has current evidence, and receives
responsibility again when that decision fails.

Observation:
Information before adoption.
Examples:
- saved config values
- existing files
- detected candidates
- previous successful runs
- logs
- old plans
- old comments
- old memories
- legacy paths
- session UI state
- generated previews
- leftover artifacts

Adopt:
The responsible layer chooses an observation as the value to use, with a reason.

Commit:
The responsible layer persists or finalizes the adopted value as the effective value.

Derived artifact:
A generated artifact made from authority.
It is not authority itself.
Examples:
- dict.bin generated from dict.txt
- cache
- sidecar state
- preview data
- scan results

Consumer:
A downstream user of committed authority.
A consumer may read, validate, use, and materialize committed authority.
A consumer must not infer, select, replace, repair, or recommit authority.

## 3. Required Questions

Before changing code, answer these:

1. What is authority here?
2. What is only observation?
3. What current evidence supports the decision?
4. Which layer adopts it?
5. Which layer commits it?
6. Where does failure responsibility return?
7. Can the guarantee be made at the entrance instead of repaired downstream?
8. Is a consumer being given selection, repair, provision, save, or commit responsibility?

## 4. Implementation Direction

Keep authority decisions near the entrance of the flow.

Place orchestration one layer above workers.
Workers should have narrow responsibility.

main.rs should eventually stay thin.
However, do not scatter unsettled authority decisions merely to reduce line count.
Split by responsibility boundary, not by file size alone.

Handlers should translate request and response shape.
They should not duplicate core translation logic.

Route differences should be expressed as route policy, request shape, and response shape.
The translation core should remain shared where the meaning is shared.

Existing code style is not authority.
Legacy or mid-layer style must not be copied into new code merely because it exists.

Small local fixes are allowed.
A local fix is good when it moves the code toward the final responsibility boundary.
A local fix is bad when it hides failure, preserves a wrong boundary, or creates a new hidden authority.

## 5. Startup and Config Rules

launcher_config.toml is the authority for launcher, backend, model, and llama-server conditions.

config.toml is the authority for runtime translation, UI, and TENUKI entry-server settings.

Before entering Normal mode, config.toml must be current-shape loadable through
the startup preflight path.

The same runtime config preflight may run immediately before backend startup.

check_ready is readiness only.
It must not repair config shape.

Installed pass, download pass, verify, and commit must remain separate.

An authority backend may be tried if installed.
An authority backend is not, by itself, permission to download.

## 6. Dictionary and Slot Rules

dict_slot must be selected, provisioned, and committed upstream.

The backend must not infer or select a different dict_slot from a language,
leftover path, or existing directory.

A consumer may materialize a committed dict_slot path.
Materializing a committed path is not authority selection.

dict.txt is the dictionary authority for a slot.

dict.bin is a derived artifact.
It must be generated from the current dict.txt authority.
It must not be reused across slot or language changes unless regenerated from
the current slot authority.

TranslationCache is session state.
NewEntriesCache is session state.
They must not cross dict_slot or language boundaries.

Legacy slot names such as S_0001 or s_0001 are observations for migration,
normalization, or collision avoidance only.
They are not current naming rules.
New code must create current slot names only.

## 7. Route and Side Effect Rules

/translate and /list may share the translation pipeline.

They must not share side effects blindly.

/translate may use dictionary lookup, session cache, dictionary events,
statistics, and input analysis according to normal translation policy.

/list must not update dictionary, cache, committed dictionary authority, or input analysis.

List output directory creation is output placement.
It must not change committed dictionary authority.

Route policy may differ.
Core translation logic should not be duplicated per route.

## 8. File Translate Rules

File Translate is an independent feature.

It borrows the existing UI stage and translation transport.
It is not normal translation.
It is not a reason to reshape the whole UI.
It is not a reason to weaken existing authority boundaries.

UI state is session state, not authority.
Preview is observation, not authority.
Scan results are observation, not authority.

Execution inputs should become fixed through readiness or RunPlan.
After that, workers should consume the fixed plan rather than rediscovering decisions.

Creating an output path from an accepted RunPlan is allowed.
That is materialization of the plan, not authority invention.

## 9. Legacy and Rescue Rules

Legacy detection is allowed.
Legacy detection is observation.

Legacy input may be normalized into current rules when the responsible layer
explicitly adopts that normalization.

Rescue is allowed only when:
- the observation being rescued is identified
- the adopt reason is explicit
- the responsible layer owns the commit
- failure responsibility is clear
- the rescue does not become a hidden permanent workaround

Do not let compatibility code look like current specification.
Keep current rules and legacy normalization visibly separate in comments and tests.

## 10. Comment and Naming Rules

Comments should describe current contracts, not history.

Do not preserve old comments that describe old behavior as if it were current.

Comments may be short, but they must clearly separate:
- authority
- observation
- derived artifact
- current rule
- legacy compatibility
- materialization
- commit

Avoid broad terms when the code means a specific component.

For dictionary code, distinguish:
- dict.txt authority
- dict.bin derived artifact
- TranslationCache
- NewEntriesCache
- committed dict_slot
- list output directory

For text processing, distinguish:
- encoding
- surface protection
- model transport
- restoration
- display normalization

## 11. Review Rule

Judge changes from the desired production shape, not from local convenience.

When reviewing or proposing a fix, name:
- authority
- observation
- adopt point
- commit point
- downstream consumer
- side effects

If something works but is structurally wrong, say so.

If a small fix is enough, prefer the small fix.
If the small fix preserves the wrong boundary, say that too.

## 12. One Line Summary

Observation is only material for adoption.
Only values adopted and committed from current evidence may flow downstream as authority.