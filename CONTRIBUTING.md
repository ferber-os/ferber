# Contributing to Wolfram

Wolfram is a kernel project. Your first contribution does not need to touch the kernel. Documentation, tests, CI and tooling, diagnostics, and localization all have useful entry points.

---

## Before You Write Code

Start with this guide. Read the architecture documents when the change calls for them:

- For kernel architecture work, read [docs/architecture.md](docs/architecture.md) and [docs/capability-model.md](docs/capability-model.md) before proposing a design.
- For IPC or memory changes, also read [docs/ipc.md](docs/ipc.md) or [docs/memory.md](docs/memory.md), respectively.

If something in the docs is wrong or unclear, that's a contribution too.
Open a PR. Docs are code.

---

## What To Work On

Choose a task that fits your experience:

- **New to systems programming:** documentation, tests, CI and tooling, or diagnostics.
- **Comfortable with Rust:** bounded kernel tasks, syscall plumbing, validation, and tests.
- **Comfortable with low-level systems or architecture code:** paging, syscall paths, scheduler, drivers, and RISC-V/x86-64 parity.
- **Security-focused:** capability-model review, unsafe invariants, threat modeling, or fuzzing.
- **Non-code:** translations, documentation, testing, UX, or Easter eggs and voice text where appropriate.

The [roadmap](ROADMAP.md) describes project direction, not a queue of starter tasks. Not every unchecked item is a good first issue. Prefer issues labeled `good first issue`, `help wanted`, or an equivalent contributor-friendly label. If none are available, open an issue to ask which bounded task needs help. Do not assume an unclaimed architecture item is ready to pick up.

For larger contributions, open an issue first and describe what you're doing.
Not because we'll say no. Because coordination prevents two people building
the same thing in parallel with incompatible designs.

### Non-code Contributions

Translations and localization are welcome, along with documentation, testing, UX, diagnostics, and Easter eggs or voice text where appropriate. Open an issue first for a new translation so the language and scope are clear.

---

## Code Style

**Rust:**
- `rustfmt` always. No exceptions. `cargo fmt` before every commit.
- `clippy` always. `cargo clippy -- -D warnings`. Fix the warnings.
- `unsafe` requires a comment explaining exactly why it's necessary
  and exactly what invariants make it safe. One line is not enough.
- No `unwrap()` in kernel code. Ever. Panics in the kernel are panics.
  Use `Result`, use `Option`, handle the failure case.
- No `println!` in kernel code. Use the kernel's serial logger.

**Comments:**
- Write comments for why, not what. The code says what.
- If you're doing something non-obvious, explain it.
- If you're citing a spec or paper, link it.
- Easter eggs in comments are allowed and encouraged.
  Read [EASTEREGGS.md](EASTEREGGS.md) for the style guide.

**Commits:**
- Present tense, imperative mood: "add capability revocation" not "added"
- Be specific: "implement buddy allocator phase 2 migration" not "memory stuff"
- One logical change per commit. Not one file. One change.

---

## Testing

```bash
bash scripts/memory-tests.sh     # host memory/FDT/allocator regressions
bash scripts/channel-tests.sh    # host channel concurrency/allocation failures
bash scripts/smoke-log.sh --self-test
make run                        # RISC-V boot checks and resident user probe
bash scripts/riscv-smoke.sh      # RISC-V normal and protection-fault matrix
make x86-smoke                  # x86-64 UEFI normal and protection-fault matrix
bash scripts/x86-iso-smoke.sh    # x86-64 release ISO boot
```

If you're adding kernel functionality, add a test.
If you're fixing a bug, add a test that would have caught it.
If you're touching the capability model, add a lot of tests.
The capability model is the whole point. It needs to be correct.

---

## The Capability Model Is Sacred

This is the one area where we will push back hard on PRs.

The capability model is Wolfram's reason for existing. Every design decision
in the kernel exists to serve it. If a change weakens the capability model —
adds ambient authority, bypasses handle checks, introduces a shortcut for
"trusted" processes — it will not be merged.

There are no trusted processes. That's the point.
There is no root. That's the point.
There is no shortcut. That's the point.

If you think you need one of these things, open an issue and explain why.
We'll figure out the right capability-based solution together.

---

## Easter Eggs

Yes, really. Read [EASTEREGGS.md](EASTEREGGS.md).
Good easter eggs are welcome contributions.
They have a style guide. Follow it.

---

## Security Issues

If you find a vulnerability in the capability model — a way to escalate
authority beyond what was granted, forge a handle, bypass a rights check —
that's the most important kind of issue you can report.

Open an issue titled `[SECURITY] <brief description>`.
Be detailed. Include a reproduction case if possible.
We'll respond fast. This is the core of what Wolfram is.

---

## The Tone

Wolfram has a voice. It's dry, honest, occasionally funny, always precise.
Error messages, panic screens, CLI output, comments — they all sound like
the same person wrote them. A person who cares deeply about the project
and has a dark sense of humor about kernel development.

If you're writing user-facing output, read the existing messages first.
Match the tone.

---

## Credit

Everyone who contributes gets credited. In the repo, in release notes,
in the model card if we ever train something on Wolfram's codebase.

You built part of a kernel. That should be on your GitHub forever.

---
