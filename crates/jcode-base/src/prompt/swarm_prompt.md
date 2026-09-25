<!--
This file IS the swarm config. Swarms are complicated, dynamic systems, so
routing policy is passed to the models as a prompt rather than as options in
a standard config file. Edit freely: override globally at
~/.jcode/swarm-prompt.md or per-project at ./.jcode/swarm-prompt.md.
-->

Model routing guidance for spawned swarm agents. Pass `model` (and optionally
`effort`) when spawning or assigning swarm work. Run `swarm list_models` first
when you need to confirm which models/routes are actually available.

- Omit `model` by default so every worker inherits the coordinator's configured
  model route and its ordered provider fallback chain.
- Implementation tasks: inherit the configured model with `effort: "low"`.
- Design, investigation, debugging, review, and verification: inherit the
  configured model with `effort: "high"`.
- Context fetching, bulk reading, and summarization: inherit the configured model
  with `effort: "none"`.
- Use an explicit model only when the user requests one or the task requires a
  capability unavailable through the configured fallback chain.

Structure guidance for spawned swarm agents:

- Always pass `label` when spawning (e.g. `label: "api reviewer"`) so the swarm
  UI shows what each agent is for. The explicit `spawn` action rejects missing or
  blank labels.
- In normal and light-swarm mode, only the root session may spawn agents. Workers
  must complete their assigned task directly and report back rather than creating
  another generation.
- Recursive spawning is reserved for a root running in `swarm-deep` mode. In that
  mode the spawner owns its children, and manager-style decomposition may create
  deeper subtrees when it materially improves coverage.
