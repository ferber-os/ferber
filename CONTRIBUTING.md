# Contributing to Ferber

Your first contribution does not need to touch the kernel. Documentation,
translations, tests, CI and tooling, diagnostics and technical reviews all have
useful entry points.

## Find a bounded task

| Experience or interest | Contribution paths |
| --- | --- |
| New to systems programming | Documentation, tests, tooling and diagnostics |
| Comfortable with Rust | Bounded kernel tasks, syscall plumbing, validation and tests |
| Low-level systems and architecture | Paging, traps, syscall entry, scheduling, drivers and architecture parity |
| Security | Capability review, unsafe invariants, threat modeling and fuzzing |
| Non-code | Localization, testing, documentation, UX and appropriate voice text |

Prefer issues labeled `good first issue`, `help wanted`, or equivalent contributor
labels. The [roadmap](ROADMAP.md) describes project direction. Not every unchecked
item is a good first issue. If no bounded task is clear, open an issue to discuss
one. Coordinate larger changes before building incompatible solutions in parallel.

Translations and localization are welcome. Discuss a new language or translation
scope in an issue so it can be maintained consistently.

## Read what your change depends on

Start with the [documentation index](docs/README.md). Changes to kernel
architecture require the [architecture guide](docs/guides/architecture.md) and
[capability model](docs/reference/capability-model.md). Read the
[IPC](docs/reference/ipc.md) or [memory](docs/reference/memory.md) reference when
changing those subsystems. ABI changes need a matching reference update.

A useful proposal states the current behavior, the proposed behavior, the
required authority and the failure/cleanup rules. State architecture-specific
assumptions explicitly. Existing RISC-V and x86-64 behavior must remain compatible
unless a deliberate interface change is documented.

## Keep code and contracts reviewable

Use `rustfmt` and Clippy with `-D warnings`. Make commits describe one coherent
engineering change, with a specific imperative subject. Keep unrelated edits
separate.

Explain each unsafe operation's caller obligations and invariants. Prefer
fallible operations and explicit error handling at trust boundaries. Follow
existing kernel logging conventions instead of introducing standard-library
output. Comments should explain the reason for code or a non-obvious constraint;
link the applicable specification when it establishes the rule.

Capability checks, memory ownership and failure atomicity are part of the
interface. Do not add ambient authority or bypass validation for a supposedly
trusted process. A narrower grant must never become broader authority through
an implementation shortcut. Discuss any design that appears to require an
exception before implementing it.

## Show what was verified

Add a regression for a bug or a new behavior. Exercise invalid authority,
malformed input, partial failure and cleanup when those boundaries change.
Report what actually ran: target, build profile, command and relevant result.
Separate host tests, QEMU, hosted CI and physical hardware evidence.

Common checks from the repository root include:

```sh
bash scripts/memory-tests.sh
bash scripts/channel-tests.sh
bash scripts/smoke-log.sh --self-test
bash scripts/riscv-smoke.sh
make x86-smoke
bash scripts/x86-iso-smoke.sh init
```

Choose additional subsystem tests from the
[verification reference](docs/reference/verification.md). A QEMU result does not
establish physical hardware support. If a check was not run, say so and explain
what remains uncertain.

## Submit the change with enough context

A contribution should explain the concrete problem, resulting behavior,
validation and limitations. Include reproduction steps for bugs and examples
for user-facing interfaces. Documentation changes should use the same terms as
the code and distinguish working behavior from plans.

A release history belongs in release notes; a specification describes the
current contract. Keep internal coordination and personal development procedures
out of public reference pages.

## Security issues

If you find a vulnerability in the capability model, such as escalating authority,
forging a handle or bypassing a rights check, open an issue titled
`[SECURITY] <brief description>`. Include a detailed reproduction case where
possible. Explain the authority initially available and the access obtained.

## Voice and non-code work

Use precise, direct English. Define unfamiliar terms before relying on them.
Avoid marketing claims and guarantees the implementation does not establish.
Match existing user-facing wording when changing diagnostics or help text.
Easter eggs and voice text follow [EASTEREGGS.md](EASTEREGGS.md).

Contributors receive credit in the repository and release notes. Documentation,
testing and localization count as contributions.
