<!--
This file IS the swarm config. For these complex, dynamic systems, models
receive routing policy as a prompt, not standard config options. Edit freely:
override globally at
~/.jcode/swarm-prompt.md or per-project at ./.jcode/swarm-prompt.md.
-->

Model routing for spawned swarm agents. Pass `model` (optionally
`effort`) when spawning or assigning work. To confirm available models/routes,
run `swarm list_models` first.

- Worker models are selected by the operator through `agents.swarm_model`; do not attempt to override them per spawn.
- Implementation tasks: `gpt-5.5` with `effort: "low"`.
- Design, investigation, debugging, review, and verification: `claude-api:claude-fable-5`.
- Context fetching / bulk reading / summarization: `gpt-5.5` with `effort: "none"`.
- If the requested route is unavailable, or the user asked for a specific model,
  or you are unsure, omit `model` so the worker inherits the coordinator's model.

Structure guidance for spawned swarm agents:

- Always pass `label` when spawning (e.g. `label: "api reviewer"`) so the swarm
  UI shows each agent's purpose. Explicit `spawn` rejects missing or blank labels.
- In normal and light-swarm mode, only the root session may spawn agents. Workers
  must complete their assigned task directly and report back rather than creating
  another generation.
- Recursive spawning is reserved for a root running in `swarm-deep` mode. In that
  mode the spawner owns its children, and manager-style decomposition may create
  deeper subtrees when it materially improves coverage.
