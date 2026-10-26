# QA agent protocol

For every nontrivial code change, summon four independent QA agents after implementation: red team, blue team, contract lawyer and failure gremlin. Give each the full diff, project rules, affected paths, and relevant verification commands. They run the relevant smoke tests and then try to break the patch from their assigned angle. They review only; they do not edit the shared worktree. Reviews may run in batches when agent slots are limited.

## The red team tests boundaries

Assume callers are hostile. Probe authorization, malformed inputs, boundary values, and ways to violate security invariants. Look for a concrete path from input to failure or privilege gain.

## The blue team tests defenses

Follow defensive controls end to end. Verify that denied operations fail closed,
authority remains contained, and errors leave a diagnosable, recoverable state.
Check that regression tests would detect a weakened control or repeated failure.
Look for gaps between prevention, failure reporting and cleanup; require concrete
evidence rather than adding speculative logging, telemetry or recovery machinery.

## The contract lawyer tests promises

Read the change literally. Check syscall and ABI behavior, architecture parity, state transitions, and documented guarantees. Compare behavior across both architectures and identify cases where implementation and contract disagree.

## The failure gremlin tests cleanup

Assume resources run out and operations fail halfway through. Probe allocation exhaustion, partial operations, ownership transfers, lifetimes, lock ordering, and teardown. Check that errors leave state and ownership valid.

## Report findings as bugs

Each agent returns independent findings with a title, severity, affected code, preconditions, reproduction steps, expected behavior, actual behavior, and evidence. Label unconfirmed concerns as suspicions. If no bug is found, report what was tested and what remains uncovered. A smoke-test pass alone is not a correctness finding.
