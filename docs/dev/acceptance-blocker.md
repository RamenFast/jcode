# Evidenced acceptance blockers

## Ask and failure

The automatic ownership loop repeatedly requested acceptance checks even after the agent reported a real provider rejection and exhausted constraints. The current acceptance predicate correctly refuses to count `acceptance_blocked` as `acceptance_aligned`. The scheduler incorrectly treats that distinction as a reason to keep requesting the impossible check.

Originating intent: `FEEDBACK.md`.

## Required behavior

1. Keep successful delivery and justified stopping as separate decisions.
2. Recognize an acceptance blocker only when the goal explicitly records `acceptance_blocked`, `constraints_exhausted`, a nonempty feedback-loop observation, and nonempty stopping evidence.
3. Stop automatic goal-quality and ownership nudges for that blocked goal. Do not rewrite its assessment, confidence, or delivery state.
4. Keep plan-level review, unrelated goals, incomplete todos, and unsupported blocker claims eligible for their normal checks.
5. Give one concise blocked closeout instead of saying quality checks passed. Explain the observed blocker, useful partial result, and condition needed to resume.
6. Keep future work armed. Reopening a todo must resume normal continuation.
7. Preserve existing synthetic-message detection and add recognition for the blocked closeout.

## Verification map

| Behavior | Real project check |
|---|---|
| Repeated loop stops locally and remotely | Persist completed todos, blocked goal, and deferred observation. Invoke the real App scheduler, consume the closeout, then invoke it eight more times. Expect no queued nudge. |
| Blocked does not become passed | Reload the goal and compare it byte-equivalently at the data level. Existing delivery and acceptance predicates must remain false. |
| Honest visible result | Inspect the App's displayed notice and final queued message. Neither may claim successful acceptance. |
| Missing evidence still prompts | Replace stopping evidence with whitespace and run the App scheduler. Expect the existing acceptance guidance. |
| Reopened work resumes | Change the persisted todo back to in-progress and run the same scheduler. Expect an incomplete-todo follow-up. |
| Mixed goals and plan review remain | Base-crate tests combine blocked and actionable groups, plus plan-level observations. Only the blocked group's nudges may disappear. |
| Historical sessions still render | Existing synthetic-message tests remain unchanged. New closeout has its own recognized prefix and summary. |

These tests run the production Rust functions and persisted App state machine, without a model worker or copied scheduler implementation. Installation must preserve the old immutable executable and avoid interrupting the active shared daemon.

## Results

The pre-fix App regression reproduced the unwanted repeated acceptance prompt. After the fix, all 47 base todo tests passed. The real App tests passed for local and remote persisted sessions, eight unchanged polls, unchanged assessments, truthful closeout, missing evidence, and reopened work.

All five affected TUI filters passed: `acceptance_blocker` (2), `auto_poke` (26), `ownership` (3), `completion_gate` (3), and `gate_digest` (1). Filters overlap. Two broader auto-poke failures were also reproduced with pristine production code. Those stale fixtures and two matching final-handoff expectations now consume the existing final-response message before expecting silence.

`cargo fmt --all --check`, `git diff --check`, module resolution, crate boundaries, and the wildcard export budget passed. Four repository-wide ratchets already fail in v0.81.7. Pristine-versus-patched comparison found no regressions. Code-size failing entries fell from 69 to 67. Swallowed errors fell from 3251 to 3250. Baselines were not changed.

Local logs live beside this checkout in `../checks/`: `regression-before.log`, `regression-after.log`, `baseline-auto-poke.log`, `runtime-verified.log`, and `guardrail-comparison.json`. `install-before.json` records original launcher and server pointers for rollback.

## Deployment boundary

The supported release installer will install this committed build for new launches, with `JCODE_SKIP_SERVER_RELOAD=1`. Running clients and the shared daemon must remain untouched. The regression tests exercise the production App scheduler and persistence boundary, not a live model session. Installation and launcher probes are recorded separately after the build.

## Installer path correction

The optimized build passed. Its installer then failed before pointer changes because the version probe did not quote the executable path. The physical checkout path contains `Mass storage`. Quoting `"$bin"` fixes this actual installation failure. `bash -n` and the quoted version probe pass against the built executable. The supported installer is rerun after committing the one-line correction.
